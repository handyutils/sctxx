//! The one background thread the TUI owns.
//!
//! Reading a session takes seconds, so it cannot happen on the thread that
//! draws. One worker, first in first out, and a *generation counter* rather than
//! a queue of intentions: `j` is a key people hold down, and holding it must not
//! queue twenty parses of sessions nobody will still be looking at when they
//! finish. A job whose session is no longer wanted is dropped before it starts,
//! and its answer is dropped again before it is sent.
//!
//! The thread is not joined on shutdown. `q` must be instant, and the work in
//! flight is a read with no side effect, so the process simply exits and takes
//! the thread with it.

use super::preview::{self, LedgerPreview};
use crate::adapters::discovery::SessionSummary;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

/// How the worker reads a session. A seam, so the staleness rule below can be
/// tested without depending on how long a real transcript takes to parse.
type Loader = Box<dyn Fn(&SessionSummary) -> crate::error::Result<LedgerPreview> + Send>;

/// A request.
struct Job {
    /// Which request this is. Only the newest generation is worth answering.
    generation: u64,
    summary: SessionSummary,
}

/// An answer.
#[derive(Debug)]
pub enum Done {
    /// One session's ledgers, or why they could not be read.
    Ledgers {
        id: String,
        result: std::result::Result<Box<LedgerPreview>, String>,
    },
}

/// The handle the UI holds. Dropping it ends the worker.
pub struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// The newest request. Anything older is stale.
    generation: Arc<AtomicU64>,
}

impl Worker {
    /// Start the real worker: [`preview::load`] on its own thread.
    pub fn spawn() -> Self {
        Self::spawn_with(Box::new(preview::load))
    }

    /// Visible to the parent module, whose tests need a worker they control.
    pub(super) fn spawn_with(loader: Loader) -> Self {
        let (jobs, job_rx) = mpsc::channel::<Job>();
        let (done_tx, done) = mpsc::channel::<Done>();
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);

        std::thread::Builder::new()
            .name("sctxx-preview".to_string())
            // A thread that cannot start is not worth failing the TUI over: the
            // pane stays on its loading line and everything else still works.
            .spawn(move || run(job_rx, done_tx, &worker_generation, &loader))
            .ok();

        Self {
            jobs,
            done,
            generation,
        }
    }

    /// Ask for one session's ledgers, superseding every earlier request.
    pub fn request(&self, summary: &SessionSummary) {
        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        // A send can only fail if the worker thread died, which would leave the
        // pane on its loading line rather than break the UI.
        let _ = self.jobs.send(Job {
            generation,
            summary: summary.clone(),
        });
    }

    /// Take every answer that has arrived. Never blocks.
    pub fn drain(&self) -> Vec<Done> {
        let mut answers = Vec::new();
        // Ends on either variant of the error: empty means "nothing yet", and
        // disconnected means the worker is gone, which is equally nothing.
        while let Ok(answer) = self.done.try_recv() {
            answers.push(answer);
        }
        answers
    }
}

fn run(jobs: Receiver<Job>, done: Sender<Done>, generation: &AtomicU64, loader: &Loader) {
    // Ends when the UI drops its handle.
    while let Ok(job) = jobs.recv() {
        // Superseded while queued: the developer has already moved on.
        if generation.load(Ordering::SeqCst) != job.generation {
            continue;
        }

        let id = job.summary.id.clone();
        let result = loader(&job.summary)
            .map(Box::new)
            .map_err(|error| error.to_string());

        // Superseded while reading: the answer is already out of date, so do
        // not spend the UI's attention on it.
        if generation.load(Ordering::SeqCst) != job.generation {
            continue;
        }
        if done.send(Done::Ledgers { id, result }).is_err() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    fn summary(id: &str) -> SessionSummary {
        SessionSummary {
            agent: "claude",
            id: id.to_string(),
            path: std::path::PathBuf::from("/tmp/does-not-matter.jsonl"),
            cwd: None,
            started_at: None,
            ended_at: None,
            lines: 0,
            bytes: 0,
            title: None,
            first_message: None,
            mtime: 0,
        }
    }

    fn preview_named(goal: &str) -> LedgerPreview {
        LedgerPreview {
            goal: Some(goal.to_string()),
            ..LedgerPreview::default()
        }
    }

    /// A worker under the test's control, plus the handles that control it.
    struct Gated {
        worker: Worker,
        /// Fires when the first read starts.
        started: Receiver<()>,
        /// Releasing this lets the first read finish.
        gate: Sender<()>,
        /// Every session id that actually reached the reader.
        seen: Arc<Mutex<Vec<String>>>,
    }

    /// A worker whose *first* read blocks until the test releases it, so the
    /// test decides the ordering rather than the scheduler. Later reads return
    /// immediately, which is what lets one release prove both rules.
    fn gated() -> Gated {
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (gate_tx, gate_rx) = mpsc::channel::<()>();
        let blocked_once = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_writer = Arc::clone(&seen);

        let worker = Worker::spawn_with(Box::new(move |summary: &SessionSummary| {
            if let Ok(mut seen) = seen_writer.lock() {
                seen.push(summary.id.clone());
            }
            // Only the first read waits.
            if !blocked_once.swap(true, Ordering::SeqCst) {
                let _ = started_tx.send(());
                let _ = gate_rx.recv_timeout(Duration::from_secs(5));
            }
            Ok(preview_named(&summary.id))
        }));
        Gated {
            worker,
            started: started_rx,
            gate: gate_tx,
            seen,
        }
    }

    #[test]
    fn an_answer_carries_the_id_it_was_asked_for() {
        let worker = Worker::spawn_with(Box::new(|summary: &SessionSummary| {
            Ok(preview_named(&summary.id))
        }));
        worker.request(&summary("aaa"));
        let answers = wait_for_answers(&worker, 1);
        match &answers[0] {
            Done::Ledgers { id, result } => {
                assert_eq!(id, "aaa");
                assert_eq!(
                    result.as_ref().expect("ok").goal.as_deref(),
                    Some("aaa"),
                    "the answer must belong to the session that was asked about"
                );
            }
        }
    }

    #[test]
    fn a_superseded_request_is_never_read_and_never_answered() {
        let gated = gated();

        gated.worker.request(&summary("first"));
        gated
            .started
            .recv_timeout(Duration::from_secs(5))
            .expect("the first read should start");
        // `second` queues behind `first`; `third` supersedes both.
        gated.worker.request(&summary("second"));
        gated.worker.request(&summary("third"));
        let _ = gated.gate.send(());

        let answers = wait_for_answers(&gated.worker, 1);
        match &answers[0] {
            Done::Ledgers { id, .. } => assert_eq!(id, "third"),
        }

        // `first` was already in flight and is dropped on completion; `second`
        // was still queued and must never reach the reader at all -- that is the
        // difference between a stale answer and wasted work.
        let seen = gated
            .seen
            .lock()
            .map(|seen| seen.clone())
            .unwrap_or_default();
        assert_eq!(
            seen,
            vec!["first".to_string(), "third".to_string()],
            "a queued job that has been superseded must not be read"
        );
    }

    #[test]
    fn a_failing_read_is_reported_rather_than_cached_as_empty() {
        let worker = Worker::spawn_with(Box::new(|_: &SessionSummary| {
            Err(crate::error::Error::Usage("nope".to_string()))
        }));
        worker.request(&summary("bad"));
        let answers = wait_for_answers(&worker, 1);
        match &answers[0] {
            Done::Ledgers { result, .. } => {
                assert!(result.is_err(), "the failure must survive the thread hop");
            }
        }
    }

    /// Wait for at least `count` answers, so no test depends on how fast the
    /// thread is scheduled.
    fn wait_for_answers(worker: &Worker, count: usize) -> Vec<Done> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut answers = Vec::new();
        while Instant::now() < deadline {
            answers.extend(worker.drain());
            if answers.len() >= count {
                // One more drain, so "only one answer" is a real assertion
                // rather than a race the test happened to win.
                std::thread::sleep(Duration::from_millis(50));
                answers.extend(worker.drain());
                return answers;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the worker did not answer within five seconds");
    }
}
