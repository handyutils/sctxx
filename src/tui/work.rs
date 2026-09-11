//! The one background thread the TUI owns.
//!
//! Reading a session takes seconds and extracting one takes longer, so neither
//! can happen on the thread that draws. One worker, first in first out, and a
//! *generation counter* for previews rather than a queue of intentions: `j` is a
//! key people hold down, and holding it must not queue twenty reads of sessions
//! nobody will still be looking at when they finish. A preview whose session is
//! no longer wanted is dropped before it starts, and its answer is dropped again
//! before it is sent.
//!
//! An extraction is deliberately **not** superseded: it is started by an explicit
//! keypress, it is the thing the developer is waiting for, and its progress is
//! the whole point of the pane. It also cannot be cancelled mid-pipeline; the
//! pane says so rather than pretending.
//!
//! The thread is not joined on shutdown. `q` must be instant, and the work in
//! flight is a read or a write of files the developer asked for, so the process
//! exits and takes the thread with it.

use super::preview::{self, LedgerPreview};
use crate::adapters::{self, discovery, discovery::SessionSummary};
use crate::agents::{self, Agent};
use crate::cli::GlobalArgs;
use crate::ir::{AgentKind, EventIdx};
use crate::pipeline::{self, ExtractOptions};
use crate::pipeline::{Cancel, artifact};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

/// How the worker reads a session. A seam, so the staleness rule below can be
/// tested without depending on how long a real transcript takes to parse.
type Loader = Box<dyn Fn(&SessionSummary) -> crate::error::Result<LedgerPreview> + Send>;

/// A request.
enum Job {
    /// Read one session's ledgers. Superseded by a newer preview request.
    Ledgers {
        /// Which request this is. Only the newest generation is worth answering.
        generation: u64,
        summary: SessionSummary,
    },
    /// Look for the coding agents installed on this machine.
    ///
    /// Asked for once, when the developer first reaches for a handoff, because
    /// reading three `--version` outputs is fast but not free and belongs off
    /// the thread that draws.
    DetectAgents,
    /// Read the events behind an `[evt a–b]` pointer.
    ///
    /// It parses a whole session, so it belongs here rather than on the thread
    /// that draws — the canvas says which range, this reads it.
    Expand {
        reference: String,
        ranges: Vec<(EventIdx, EventIdx)>,
        context: EventIdx,
        global: Box<GlobalArgs>,
    },
    /// Run an extraction and write it where the developer said.
    Extract {
        summary: SessionSummary,
        options: Box<ExtractOptions>,
        out: PathBuf,
        /// The host's flag for stopping it.
        cancel: Cancel,
    },
}

/// What an extraction produced.
#[derive(Debug, Clone)]
pub struct Outcome {
    /// The session this came from, as `claude:1367d688`.
    pub reference: String,
    /// The file a receiving agent should read.
    pub handoff: PathBuf,
    /// Everything that was written.
    pub paths: Vec<PathBuf>,
    /// Notes the pipeline raised about its own output.
    pub warnings: Vec<String>,
    /// The CLI's own "this directory is not ignored by git" warning, produced by
    /// the CLI's own function so the wording cannot drift (FR-015).
    pub git_warning: Option<String>,
    pub live_events: usize,
    pub total_events: usize,
}

/// An answer.
#[derive(Debug)]
pub enum Done {
    /// One session's ledgers, or why they could not be read.
    Ledgers {
        id: String,
        result: std::result::Result<Box<LedgerPreview>, String>,
    },
    /// One stage of a running extraction (spec FR-013's stage names).
    Progress { stage: String, message: String },
    /// An extraction finished, or failed.
    Extracted {
        result: std::result::Result<Box<Outcome>, String>,
    },
    /// The agents on this machine. Detection always answers, with the reason
    /// each agent is or is not usable, so this cannot fail.
    Agents(Vec<Agent>),
    /// The rows behind a pointer, or why they could not be read.
    Expanded {
        label: String,
        result: std::result::Result<String, String>,
    },
}

/// The handle the UI holds. Dropping it ends the worker.
pub struct Worker {
    jobs: Sender<Job>,
    done: Receiver<Done>,
    /// The newest preview request. Anything older is stale.
    generation: Arc<AtomicU64>,
}

impl Worker {
    /// Start the real worker on its own thread.
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
            .name("sctxx-work".to_string())
            // A thread that cannot start is not worth failing the TUI over: the
            // pane stays on its working line and everything else still works.
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
        let _ = self.jobs.send(Job::Ledgers {
            generation,
            summary: summary.clone(),
        });
    }

    /// Read the events behind one `[evt a–b]` pointer.
    pub fn expand(&self, reference: &str, range: (EventIdx, EventIdx), global: &GlobalArgs) {
        let _ = self.jobs.send(Job::Expand {
            reference: reference.to_string(),
            ranges: vec![range],
            context: 0,
            global: Box::new(global.clone()),
        });
    }

    /// Ask which coding agents are installed.
    pub fn detect_agents(&self) {
        let _ = self.jobs.send(Job::DetectAgents);
    }

    /// Run an extraction. Only the UI decides when this is legal.
    pub fn extract(
        &self,
        summary: &SessionSummary,
        options: ExtractOptions,
        out: PathBuf,
        cancel: Cancel,
    ) {
        let _ = self.jobs.send(Job::Extract {
            summary: summary.clone(),
            options: Box::new(options),
            out,
            cancel,
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
        match job {
            Job::Ledgers {
                generation: wanted,
                summary,
            } => {
                // Superseded while queued: the developer has already moved on.
                if generation.load(Ordering::SeqCst) != wanted {
                    continue;
                }

                let id = summary.id.clone();
                let result = loader(&summary)
                    .map(Box::new)
                    .map_err(|error| error.to_string());

                // Superseded while reading: the answer is already out of date,
                // so do not spend the UI's attention on it.
                if generation.load(Ordering::SeqCst) != wanted {
                    continue;
                }
                if done.send(Done::Ledgers { id, result }).is_err() {
                    break;
                }
            }
            Job::Expand {
                reference,
                ranges,
                context,
                global,
            } => {
                let label = range_label(&ranges);
                let result = expand(&reference, &ranges, context, &global)
                    .map_err(|error| error.to_string());
                if done.send(Done::Expanded { label, result }).is_err() {
                    break;
                }
            }
            Job::DetectAgents => {
                if done
                    .send(Done::Agents(agents::Machine::this_one().detect()))
                    .is_err()
                {
                    break;
                }
            }
            Job::Extract {
                summary,
                options,
                out,
                cancel,
            } => {
                let result = extract(&summary, &options, &out, &done, &cancel)
                    .map(Box::new)
                    .map_err(|error| error.to_string());
                if done.send(Done::Extracted { result }).is_err() {
                    break;
                }
            }
        }
    }
}

/// How a range is named on screen, matching what `sctxx expand` prints.
fn range_label(ranges: &[(EventIdx, EventIdx)]) -> String {
    ranges
        .iter()
        .map(|(start, end)| {
            if start == end {
                format!("evt {start}")
            } else {
                format!("evt {start}–{end}")
            }
        })
        .collect::<Vec<String>>()
        .join(", ")
}

/// Read the rows behind a pointer.
///
/// The same library call `sctxx expand` makes, reached through the same
/// function, so a pointer means one thing in both places (FR-016).
fn expand(
    reference: &str,
    ranges: &[(EventIdx, EventIdx)],
    context: EventIdx,
    global: &GlobalArgs,
) -> crate::error::Result<String> {
    let parsed = discovery::parse_reference(reference)?;
    // Any project: the handoff may name a session whose directory the developer
    // is not standing in.
    let options = global.resolve_options(true, true);
    let summary = discovery::resolve(&parsed, &options)?;
    let agent = AgentKind::from_slug(summary.agent)
        .ok_or_else(|| crate::error::Error::UnknownFormat(summary.path.clone()))?;
    let source = adapters::source::read(&summary.path)?;
    let session = adapters::parse_as(agent, source, adapters::DEFAULT_MAX_BAD_LINE_RATE)?;
    Ok(artifact::expand_ranges(&session, ranges, context))
}

/// Run the pipeline and write the artifact, reporting each stage as it starts.
///
/// This is the same library call the CLI makes; the only difference is where the
/// progress and the payload go. Nothing here writes to stdout, which is what
/// keeps FR-004 true: a TUI never puts an artifact on the terminal.
fn extract(
    summary: &SessionSummary,
    options: &ExtractOptions,
    out: &std::path::Path,
    done: &Sender<Done>,
    cancel: &Cancel,
) -> crate::error::Result<Outcome> {
    let mut progress = |stage: &str, message: &str| {
        let _ = done.send(Done::Progress {
            stage: stage.to_string(),
            message: message.to_string(),
        });
    };

    let extraction = pipeline::extract_interruptible(summary, options, &mut progress, cancel)?;
    let destination = resolve_destination(out, summary);
    let written = pipeline::write_destination(&extraction, options, &destination)?;

    Ok(Outcome {
        reference: summary.reference(),
        handoff: written.handoff,
        paths: written.paths,
        warnings: extraction.report.warnings.clone(),
        git_warning: written
            .directory
            .then(|| crate::cli::extract::git_track_warning(&destination))
            .flatten(),
        live_events: extraction.session.active.len(),
        total_events: extraction.session.events.len(),
    })
}

/// Where a relative `--out` lands.
///
/// A developer running the TUI from anywhere means "next to the project this
/// session was about", not "next to wherever sctxx happens to be". An absolute
/// path, or a session whose directory is gone, is left exactly as given.
fn resolve_destination(out: &std::path::Path, summary: &SessionSummary) -> PathBuf {
    if out.is_absolute() {
        return out.to_path_buf();
    }
    let Some(cwd) = summary.cwd.as_deref() else {
        return out.to_path_buf();
    };
    let base = std::path::Path::new(cwd);
    if base.is_dir() {
        base.join(out)
    } else {
        out.to_path_buf()
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
            other => panic!("expected ledgers, got {other:?}"),
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
            other => panic!("expected ledgers, got {other:?}"),
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
            other => panic!("expected ledgers, got {other:?}"),
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
