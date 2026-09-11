//! The interactive mode behind `sctxx --tui`.
//!
//! This module owns the terminal, the event loop, and the state the screen is a
//! view over; it owns as little else as possible. Everything a test can check
//! without a TTY lives in [`browser`], [`form`], [`preview`] and [`work`],
//! because an event loop needs a terminal and a test does not have one.
//!
//! The screen reads the same discovery layer the CLI reads, so nothing here can
//! disagree with `sctxx list`. Reading a session's contents and extracting it
//! are both seconds of work, so both happen on the worker thread [`work`] owns
//! and arrive in the pane when they are ready, never as a stall.

pub mod browser;

mod canvas;
mod form;
mod preview;
mod ui;
mod work;

use crate::adapters::discovery::{self, SessionSummary};
use crate::agents::Agent;
use crate::agents::seeding::Launch;
use crate::cli::GlobalArgs;
use crate::error::{Error, Result};
use crate::pipeline::{Cancel, ExtractOptions, artifact};
use browser::Browser;
use canvas::Canvas;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use form::{Control, Form};
use preview::LedgerPreview;
use ratatui::DefaultTerminal;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use work::Worker;

/// How long the selection must sit still before its transcript is read.
///
/// Reading a session takes seconds and `j` is a key people hold down, so
/// without a settle delay a held key would ask for every session it passed.
const SETTLE: Duration = Duration::from_millis(150);

/// How often the loop wakes when nothing is typed, so work that finished while
/// the developer was reading appears without a key press.
const TICK: Duration = Duration::from_millis(50);

/// How many previews are kept. Each is a few strings; the cap exists only so a
/// long browsing session cannot grow without bound.
const CACHE_MAX: usize = 256;

/// How many progress lines the extraction pane keeps.
const PROGRESS_KEPT: usize = 12;

/// The body height assumed before the first frame is drawn.
const DEFAULT_BODY_HEIGHT: usize = 24;

/// Which half of the interaction the keyboard is in.
///
/// A mode rather than bare keystrokes because `a`, `r`, `p`, `e` and `q` are
/// both commands and letters a developer might type into a search box or a form.
/// Guessing would make both unusable for anyone searching for "query".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Search,
    Form,
    Handoff,
    /// Reading the artifact.
    Canvas,
    /// Typing a path to an artifact that already exists.
    OpenArtifact,
}

/// What the event loop decided to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Continue,
    Quit,
}

/// What the pane knows about the selected session's ledgers.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PreviewState {
    /// Asked for, or about to be. The pane says so rather than looking empty.
    Loading,
    Ready(Box<LedgerPreview>),
    /// Terminal for this session: a read that failed is not retried on every
    /// cursor pass, because the failure is almost always the file itself.
    Failed(String),
}

/// What the extraction pane is showing.
#[derive(Debug, Clone)]
enum RunState {
    /// Nothing run yet for the form on screen.
    Idle,
    /// Running, with the stage the pipeline reported last and a short log.
    Running {
        stage: String,
        message: String,
        lines: Vec<String>,
    },
    /// Finished, with what was written.
    Done(Box<work::Outcome>),
    /// Stopped because the developer asked it to stop. Not a failure, and not
    /// silence either.
    Cancelled,
    /// Failed, with the reason. The form is left untouched so it can be fixed.
    Failed(String),
}

impl RunState {
    fn running(&self) -> bool {
        matches!(self, RunState::Running { .. })
    }
}

/// The handoff flow.
///
/// Deliberately two steps. Extraction never launches anything by itself
/// (FR-021b), and the exact command is on screen before it runs (FR-021).
#[derive(Debug, Clone)]
enum HandoffState {
    /// Picking an agent from the ones that are installed.
    Choosing,
    /// Showing exactly what will happen — where the context will be written and
    /// which agent will start — and waiting for a yes.
    Confirming {
        agent: usize,
        /// The directory the artifact goes to, absolute, because "`.sctxx/`" is
        /// not an answer to "where did that go?".
        destination: PathBuf,
    },
}

/// The whole of the TUI's state.
struct App {
    browser: Browser,
    mode: Mode,
    /// Previews by session id, so returning to a session is instant.
    previews: BTreeMap<String, PreviewState>,
    /// The extraction form, when one is open.
    form: Option<Form>,
    /// The artifact open for reading (FR-016, FR-016a).
    canvas: Option<Canvas>,
    /// What has been typed into the "open an artifact" prompt.
    open_path: String,
    /// How tall the body is, in rows. The event loop sets it each frame, because
    /// the canvas needs it to know what a page is.
    body_height: usize,
    /// The handoff flow, when it is open.
    handoff: Option<HandoffState>,
    /// Which detected agent the cursor is on.
    handoff_cursor: usize,
    /// What the last handoff did, so the developer sees it when they come back.
    handoff_note: Option<String>,
    /// The agents on this machine, once they have been looked for. `None` means
    /// nobody has asked yet, which is why the pane can say so.
    agents: Option<Vec<Agent>>,
    /// The session being handed on, taken when the handoff opens so that moving
    /// the cursor afterwards cannot change what is being extracted.
    handoff_session: Option<SessionSummary>,
    /// The agent to launch once the extraction finishes, if the handoff started
    /// the extraction itself.
    pending_launch: Option<usize>,
    /// A launch the event loop should perform. Set when the confirmation is
    /// given and the artifact is already there, or when the extraction the
    /// handoff started finishes.
    pending_handover: Option<Box<Launch>>,
    /// What the extraction pane is showing.
    run: RunState,
    worker: Worker,
    /// The selection the settle delay is counting for, and since when.
    settle: Option<(String, Instant)>,
    /// A field rather than the constant so a test removes the wait instead of
    /// sleeping through it.
    settle_after: Duration,
    /// The flags this invocation were given, for the form to convert against.
    global: GlobalArgs,
    /// The flag the running extraction watches.
    cancel: Cancel,
    /// What `--llm auto` resolves to on this machine, so the form can show the
    /// consequence of leaving it alone instead of letting the developer find out
    /// by waiting.
    resolved_llm: Option<String>,
}

impl App {
    fn new(
        sessions: Vec<SessionSummary>,
        now: u64,
        project: Option<String>,
        global: GlobalArgs,
    ) -> Self {
        Self {
            browser: Browser::new(sessions, now, project),
            mode: Mode::Browse,
            previews: BTreeMap::new(),
            form: None,
            canvas: None,
            open_path: String::new(),
            body_height: DEFAULT_BODY_HEIGHT,
            handoff: None,
            handoff_cursor: 0,
            handoff_note: None,
            agents: None,
            handoff_session: None,
            pending_launch: None,
            pending_handover: None,
            run: RunState::Idle,
            worker: Worker::spawn(),
            settle: None,
            settle_after: SETTLE,
            global,
            cancel: Cancel::new(),
            resolved_llm: crate::llm::resolve_auto().map(|selection| selection.to_string()),
        }
    }

    /// The state to draw for the selected session, if it has one.
    fn preview_state(&self) -> Option<&PreviewState> {
        self.browser
            .selected()
            .and_then(|session| self.previews.get(&session.id))
    }

    /// Take anything the worker finished, and ask for the next session once the
    /// selection has held still long enough to be worth reading.
    fn pump(&mut self) {
        for done in self.worker.drain() {
            match done {
                work::Done::Ledgers { id, result } => {
                    let state = match result {
                        Ok(preview) => PreviewState::Ready(preview),
                        Err(reason) => PreviewState::Failed(reason),
                    };
                    self.remember(id, state);
                }
                work::Done::Progress { stage, message } => {
                    if let RunState::Running {
                        stage: current,
                        message: current_message,
                        lines,
                    } = &mut self.run
                    {
                        *current = stage.clone();
                        *current_message = message.clone();
                        if lines.len() < PROGRESS_KEPT {
                            lines.push(format!("[{stage}] {message}"));
                        }
                    }
                }
                work::Done::Expanded { label, result } => {
                    if let Some(canvas) = self.canvas.as_mut() {
                        match result {
                            Ok(text) => canvas.expansion_ready(label, &text),
                            Err(reason) => canvas.expansion_failed(label, reason),
                        }
                    }
                }
                work::Done::Agents(detected) => {
                    self.agents = Some(detected);
                }
                work::Done::Extracted { result } => {
                    self.run = match result {
                        // A stop the developer asked for is not an error, and
                        // saying "failed" would be a lie about their own action.
                        Err(_) if self.cancel.cancelled() => RunState::Cancelled,
                        Ok(outcome) => {
                            // A handoff extracts in order to launch, so it does
                            // not stop to show the artifact on the way.
                            if let Some(agent) = self.pending_launch.take() {
                                self.run = RunState::Done(outcome);
                                self.launch_now(agent);
                                return;
                            }
                            // FR-016: the artifact opens itself, at L0, without
                            // another keypress. A developer who has to go and
                            // find it will not read it.
                            match Canvas::load(&outcome.handoff) {
                                Ok(canvas) => {
                                    self.canvas = Some(canvas);
                                    self.mode = Mode::Canvas;
                                }
                                Err(error) => {
                                    // The files are written; only reading them
                                    // back failed, so say so and stay put.
                                    self.handoff_note = Some(error.to_string());
                                }
                            }
                            RunState::Done(outcome)
                        }
                        Err(reason) => RunState::Failed(reason),
                    };
                }
            }
        }

        let Some(id) = self.browser.selected().map(|session| session.id.clone()) else {
            self.settle = None;
            return;
        };
        // Known, in flight, or already failed: never ask twice.
        if self.previews.contains_key(&id) {
            return;
        }

        let settled = match &self.settle {
            Some((pending, since)) => *pending == id && since.elapsed() >= self.settle_after,
            None => false,
        };
        if !settled {
            // Arm, or leave running, the delay for this selection.
            if !matches!(&self.settle, Some((pending, _)) if *pending == id) {
                self.settle = Some((id, Instant::now()));
            }
            return;
        }

        if let Some(summary) = self.browser.selected().cloned() {
            self.worker.request(&summary);
        }
        self.remember(id, PreviewState::Loading);
        self.settle = None;
    }

    /// Record a preview, evicting deterministically once the cache is full.
    fn remember(&mut self, id: String, state: PreviewState) {
        if self.previews.len() >= CACHE_MAX && !self.previews.contains_key(&id) {
            // BTreeMap order, not a HashMap's: the eviction must not depend on
            // how the table happens to be laid out.
            if let Some(oldest) = self.previews.keys().next().cloned() {
                self.previews.remove(&oldest);
            }
        }
        self.previews.insert(id, state);
    }

    /// The height of the reading area, which is what a page is.
    fn page(&self) -> usize {
        self.body_height.saturating_sub(2).max(1)
    }

    /// Show the artifact: the one just written, or one already on disk for the
    /// selected session.
    ///
    /// A previous run's `.sctxx/handoff.md` is the ordinary case FR-016a names —
    /// the point is that the pane is a viewer, not only a receipt.
    fn open_canvas(&mut self) {
        if self.canvas.is_some() {
            self.mode = Mode::Canvas;
            return;
        }
        let candidate = self
            .browser
            .selected()
            .and_then(|session| session.cwd.clone())
            .map(|cwd| PathBuf::from(cwd).join(".sctxx"));
        let Some(candidate) = candidate else {
            return;
        };
        if !candidate.join("handoff.md").is_file() && !candidate.join("handoff.json").is_file() {
            return;
        }
        match Canvas::load(&candidate) {
            Ok(canvas) => {
                self.canvas = Some(canvas);
                self.mode = Mode::Canvas;
            }
            Err(error) => {
                self.handoff = None;
                self.run = RunState::Failed(error.to_string());
            }
        }
    }

    /// Open an artifact by the path the developer typed (FR-016a).
    fn handle_open_artifact(&mut self, key: KeyEvent) -> Step {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Step::Quit;
            }
            KeyCode::Esc => {
                self.open_path.clear();
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => {
                let typed = self.open_path.trim().to_string();
                if !typed.is_empty() {
                    match Canvas::load(Path::new(&typed)) {
                        Ok(canvas) => {
                            self.canvas = Some(canvas);
                            self.mode = Mode::Canvas;
                        }
                        Err(error) => {
                            self.handoff_note = Some(error.to_string());
                        }
                    }
                }
                self.open_path.clear();
            }
            KeyCode::Backspace => {
                self.open_path.pop();
            }
            KeyCode::Char(character) => self.open_path.push(character),
            _ => {}
        }
        Step::Continue
    }

    /// Reading keys: layers, scrolling, and following a pointer.
    fn handle_canvas(&mut self, key: KeyEvent) -> Step {
        let page = self.page();
        let Some(canvas) = self.canvas.as_mut() else {
            self.mode = Mode::Browse;
            return Step::Continue;
        };
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Step::Quit;
            }
            KeyCode::Char('q') => return Step::Quit,
            // `esc` closes an expansion first, and only then the canvas: a
            // reader who opened a pointer expects esc to put it back.
            KeyCode::Esc if !canvas.dismiss_expansion() => self.mode = Mode::Browse,
            KeyCode::Tab => canvas.step_layer(1),
            KeyCode::BackTab => canvas.step_layer(-1),
            KeyCode::Char('1') => canvas.select_layer(0),
            KeyCode::Char('2') => canvas.select_layer(1),
            KeyCode::Char('3') => canvas.select_layer(2),
            KeyCode::Char('4') => canvas.select_layer(3),
            KeyCode::Char('j') | KeyCode::Down => canvas.move_cursor(1, page),
            KeyCode::Char('k') | KeyCode::Up => canvas.move_cursor(-1, page),
            KeyCode::PageDown | KeyCode::Char(' ') => canvas.move_page(true, page),
            KeyCode::PageUp | KeyCode::Char('b') => canvas.move_page(false, page),
            KeyCode::Char('g') | KeyCode::Home => canvas.move_to(false, page),
            KeyCode::Char('G') | KeyCode::End => canvas.move_to(true, page),
            KeyCode::Enter => {
                if let Some(range) = canvas.pointer_under_cursor() {
                    let label = if range.0 == range.1 {
                        format!("evt {}", range.0)
                    } else {
                        format!("evt {}–{}", range.0, range.1)
                    };
                    canvas.expansion_pending(label.clone());
                    // The artifact knows its own source, which is what makes a
                    // colleague's handoff readable too.
                    match artifact::source_reference(canvas.path()) {
                        Some(reference) => self.worker.expand(&reference, range, &self.global),
                        None => canvas.expansion_failed(
                            label,
                            "this artifact does not name the session it came from".to_string(),
                        ),
                    }
                }
            }
            _ => {}
        }
        Step::Continue
    }

    /// Open the handoff, once an extraction has produced something to hand on.
    fn open_handoff(&mut self) {
        let Some(session) = self.browser.selected().cloned() else {
            return;
        };
        // No extraction required first. The whole point of the feature is that
        // "hand this session to another agent" is one action, not a pipeline the
        // developer has to drive.
        self.handoff_session = Some(session);
        self.pending_launch = None;
        self.handoff = Some(HandoffState::Choosing);
        self.handoff_cursor = 0;
        self.handoff_note = None;
        self.mode = Mode::Handoff;
        // Look for the agents the first time it is asked for, off the UI thread
        // because reading three `--version` outputs takes a moment.
        if self.agents.is_none() {
            self.worker.detect_agents();
        }
    }

    /// Where the artifact for the handoff session goes, as an absolute path.
    ///
    /// Relative to the project the session was about, because that is what
    /// `.sctxx/` means to someone handing work on — and shown absolute, because
    /// a relative path does not answer "where did that go?".
    fn handoff_destination(&self) -> PathBuf {
        let relative = PathBuf::from(".sctxx");
        if let Some(cwd) = self
            .handoff_session
            .as_ref()
            .and_then(|session| session.cwd.as_deref())
        {
            let base = Path::new(cwd);
            if base.is_dir() {
                return base.join(relative);
            }
        }
        std::env::current_dir()
            .map(|cwd| cwd.join(&relative))
            .unwrap_or(relative)
    }

    /// The agent under the cursor, when it can actually be launched.
    fn handoff_agent(&self, cursor: usize) -> Result<&Agent> {
        let agents = self.agents.as_ref().ok_or_else(|| {
            Error::Usage("still looking for the agents installed here".to_string())
        })?;
        let agent = agents
            .get(cursor)
            .ok_or_else(|| Error::Usage("no agent is selected".to_string()))?;
        if !agent.installed() {
            return Err(Error::Usage(format!(
                "{} is not installed (no `{}` on PATH)",
                agent.label, agent.id
            )));
        }
        Ok(agent)
    }

    /// The options the handoff uses.
    ///
    /// Deterministic on purpose. "Move this session to another agent" is not a
    /// request to spend eight hundred thousand tokens: the deterministic
    /// artifact already carries every `[evt a-b]` pointer, every ledger, and the
    /// recency tail, which is what a receiving agent needs. A richer artifact is
    /// available through `e`, deliberately and visibly.
    fn deterministic_options(&self, reference: &str) -> Result<ExtractOptions> {
        crate::cli::extract::ExtractArgs::parse_argv(["sctxx", "--llm", "none", reference])?
            .options(&self.global)
    }

    /// The developer said yes: make sure the context is on disk, then launch.
    ///
    /// If the artifact is already there *and belongs to this session*, it is
    /// reused rather than rewritten — a handoff of something already extracted
    /// should not re-do the work.
    fn confirm_handoff(&mut self, agent: usize, destination: PathBuf) {
        let Some(session) = self.handoff_session.clone() else {
            return;
        };
        let artifact = destination.join("handoff.md");
        let current = artifact::source_reference(&artifact);
        if current.as_deref() == Some(session.reference().as_str()) {
            self.launch_now(agent);
            return;
        }

        let options = match self.deterministic_options(&session.reference()) {
            Ok(options) => options,
            Err(error) => {
                self.handoff_note = Some(error.to_string());
                return;
            }
        };
        self.pending_launch = Some(agent);
        self.cancel = Cancel::new();
        self.run = RunState::Running {
            stage: "extract".into(),
            message: format!("writing {}", artifact.display()),
            lines: Vec::new(),
        };
        self.worker
            .extract(&session, options, destination, self.cancel.clone());
    }

    /// Build the launch and ask the event loop to hand the terminal over.
    fn launch_now(&mut self, agent: usize) {
        let Some(session) = self.handoff_session.clone() else {
            return;
        };
        let artifact = self.handoff_destination().join("handoff.md");
        let Some(target) = self.agents.as_ref().and_then(|agents| agents.get(agent)) else {
            return;
        };
        match Launch::interactive(target, &artifact, session.cwd.as_deref().map(Path::new)) {
            Ok(launch) => self.pending_handover = Some(Box::new(launch)),
            Err(error) => self.handoff_note = Some(error.to_string()),
        }
    }

    /// Take the launch the event loop should perform, if there is one.
    fn take_pending_handover(&mut self) -> Option<Box<Launch>> {
        self.pending_handover.take()
    }

    /// Choose an agent, then confirm what will happen. Two steps, never one.
    fn handle_handoff(&mut self, key: KeyEvent) -> Step {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Step::Quit;
        }
        let count = self.agents.as_ref().map(Vec::len).unwrap_or(0);
        match self.handoff.clone() {
            Some(HandoffState::Choosing) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.handoff = None;
                    self.handoff_session = None;
                    self.mode = Mode::Browse;
                }
                KeyCode::Char('j') | KeyCode::Down if count > 0 => {
                    self.handoff_cursor = (self.handoff_cursor + 1).min(count - 1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.handoff_cursor = self.handoff_cursor.saturating_sub(1);
                }
                KeyCode::Enter => match self.handoff_agent(self.handoff_cursor) {
                    Ok(_) => {
                        self.handoff_note = None;
                        let destination = self.handoff_destination();
                        self.handoff = Some(HandoffState::Confirming {
                            agent: self.handoff_cursor,
                            destination,
                        });
                    }
                    Err(error) => self.handoff_note = Some(error.to_string()),
                },
                _ => {}
            },
            Some(HandoffState::Confirming { agent, destination }) => match key.code {
                KeyCode::Esc => self.handoff = Some(HandoffState::Choosing),
                KeyCode::Enter | KeyCode::Char('y') => self.confirm_handoff(agent, destination),
                _ => {}
            },
            None => self.mode = Mode::Browse,
        }
        Step::Continue
    }

    /// Record what a handed-over agent did, once the terminal is ours again.
    fn launched(&mut self, launch: &Launch, outcome: Result<i32>) {
        self.handoff_note = Some(match outcome {
            Ok(0) => format!("{} finished", launch.agent),
            Ok(code) => format!("{} exited with {code}", launch.agent),
            Err(error) => error.to_string(),
        });
        self.handoff = Some(HandoffState::Choosing);
        self.mode = Mode::Handoff;
    }

    /// Open the extraction form for the selected session.
    fn open_form(&mut self) {
        let Some(session) = self.browser.selected() else {
            return;
        };
        self.form = Some(Form::new(&session.reference()));
        self.run = RunState::Idle;
        self.mode = Mode::Form;
    }

    /// Validate the form and hand the work to the worker.
    ///
    /// Validation and option-building happen here, on the UI thread, so a typo is
    /// reported instantly instead of after a round trip.
    fn start_run(&mut self) {
        if self.run.running() {
            // Say so rather than ignoring the key, and point at the way out.
            if let Some(form) = self.form.as_mut() {
                form.set_hint(Some(
                    "already running \u{2014} enter on the last row stops it".to_string(),
                ));
            }
            return;
        }
        // A fresh flag per run, so a previous cancellation does not poison this
        // one.
        self.cancel = Cancel::new();
        let (Some(form), Some(session)) = (self.form.as_ref(), self.browser.selected().cloned())
        else {
            return;
        };
        let options = match form.options(&self.global) {
            Ok(options) => options,
            Err(error) => {
                self.run = RunState::Failed(error.to_string());
                return;
            }
        };
        let destination = form.get("out").unwrap_or_default().to_string();
        if destination.is_empty() {
            self.run = RunState::Failed(
                "choose where the artifact goes: the --out field is empty".to_string(),
            );
            return;
        }
        self.run = RunState::Running {
            stage: "start".to_string(),
            message: String::new(),
            lines: Vec::new(),
        };
        self.worker.extract(
            &session,
            options,
            PathBuf::from(destination),
            self.cancel.clone(),
        );
    }

    /// Ask the running extraction to stop at its next chunk boundary.
    fn cancel_run(&mut self) {
        self.cancel.cancel();
        if let RunState::Running { message, .. } = &mut self.run {
            *message = "stopping at the next step\u{2026}".to_string();
        }
    }

    fn handle(&mut self, key: KeyEvent) -> Step {
        // `e` opens the form, but only where `e` is a command rather than a
        // letter someone is typing.
        if self.mode == Mode::Browse && key.code == KeyCode::Char('e') {
            self.open_form();
            return Step::Continue;
        }
        // `h` hands off, but only once there is something to hand off.
        if self.mode == Mode::Browse && key.code == KeyCode::Char('h') {
            self.open_handoff();
            return Step::Continue;
        }
        if self.mode == Mode::Browse && key.code == KeyCode::Char('c') {
            self.open_canvas();
            return Step::Continue;
        }
        if self.mode == Mode::Browse && key.code == KeyCode::Char('o') {
            self.open_path.clear();
            self.mode = Mode::OpenArtifact;
            return Step::Continue;
        }

        let step = match self.mode {
            Mode::Browse => handle_browse(key, &mut self.browser, &mut self.mode),
            Mode::Search => handle_search(key, &mut self.browser, &mut self.mode),
            Mode::Form => return self.handle_form(key),
            Mode::Handoff => return self.handle_handoff(key),
            Mode::Canvas => return self.handle_canvas(key),
            Mode::OpenArtifact => return self.handle_open_artifact(key),
        };
        // Any key restarts the delay: while a query is being typed the visible
        // list is still moving, and reading a session the developer is about to
        // filter away is wasted work.
        self.settle = None;
        step
    }

    /// The form is a text field: letters type, so movement is arrows and tab.
    /// Space would otherwise be both "toggle" and "type a space".
    fn handle_form(&mut self, key: KeyEvent) -> Step {
        match key.code {
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                return Step::Quit;
            }
            KeyCode::Esc => {
                self.form = None;
                self.mode = Mode::Browse;
                return Step::Continue;
            }
            KeyCode::Enter => {
                // `enter` commits the field it is on and moves on. Running is
                // the row at the bottom, because a key that sometimes means
                // "edit" and sometimes means "spend money and time" is a trap.
                let on_run_row = self.form.as_ref().is_some_and(Form::on_run_row);
                if on_run_row {
                    if self.run.running() {
                        self.cancel_run();
                    } else {
                        self.start_run();
                    }
                } else if let Some(form) = self.form.as_mut() {
                    form.activate();
                    form.move_focus(1);
                }
                return Step::Continue;
            }
            _ => {}
        }

        let Some(form) = self.form.as_mut() else {
            self.mode = Mode::Browse;
            return Step::Continue;
        };
        match key.code {
            KeyCode::Down | KeyCode::Tab => form.move_focus(1),
            KeyCode::Up | KeyCode::BackTab => form.move_focus(-1),
            // Left and right step a fixed-choice field; on text they are free
            // for a future cursor and do nothing today.
            KeyCode::Left => form.step(-1),
            KeyCode::Right => form.step(1),
            KeyCode::Backspace => form.pop_char(),
            KeyCode::Char(' ') => {
                let text = matches!(
                    form.focused().map(|field| field.control.clone()),
                    Some(Control::Text)
                );
                if text {
                    form.push_char(' ');
                } else {
                    form.activate();
                }
            }
            KeyCode::Char(character) => form.push_char(character),
            _ => {}
        }
        Step::Continue
    }
}

pub fn run(global: &GlobalArgs) -> Result<i32> {
    // A TUI needs a terminal on both ends. Agents pipe one or the other, and
    // they must keep getting the CLI behaviour rather than a hang.
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(Error::Usage(
            "--tui needs a terminal (stdin and stdout must both be a TTY). \
             Piping is for the CLI: use `sctxx list`, `sctxx find`, or `sctxx extract`."
                .to_string(),
        ));
    }

    let sessions = discovery::list_all(&global.roots());
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let project = std::env::current_dir()
        .ok()
        .map(|path| path.to_string_lossy().into_owned());
    let mut app = App::new(sessions, now, project, global.clone());

    let mut terminal = ratatui::try_init().map_err(|error| {
        Error::Other(format!("could not start the terminal interface: {error}"))
    })?;
    let outcome = event_loop(&mut terminal, &mut app);
    // Always restore, including on error: leaving a terminal in raw mode after
    // a crash is worse than the crash.
    ratatui::restore();
    outcome.map(|_| 0)
}

fn event_loop(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    loop {
        app.pump();
        // A handoff extracts before it launches, so the request to hand over
        // arrives from the worker's answer rather than from a keypress.
        hand_over_if_asked(terminal, app)?;
        // The canvas needs to know what a page is, and only the terminal does.
        app.body_height = terminal
            .size()
            .map(|size| size.height as usize)
            .unwrap_or(DEFAULT_BODY_HEIGHT);
        terminal
            .draw(|frame| ui::draw(frame, app))
            .map_err(|error| Error::Other(format!("could not draw: {error}")))?;

        // Wake on the tick even with no input, so work that finished while the
        // developer was reading appears by itself.
        let ready = event::poll(TICK)
            .map_err(|error| Error::Other(format!("could not watch for input: {error}")))?;
        if !ready {
            continue;
        }
        let Ok(Event::Key(key)) = event::read()
            .map_err(|error| Error::Other(format!("could not read a key press: {error}")))
        else {
            continue;
        };
        // Windows reports press and release; act on press only.
        if key.kind != KeyEventKind::Press {
            continue;
        }
        if app.handle(key) == Step::Quit {
            return Ok(());
        }
        hand_over_if_asked(terminal, app)?;
    }
}

/// Hand the terminal over, when the app has a launch ready.
///
/// A coding agent is a full-screen application, so it gets the whole terminal
/// rather than a pane inside this one (ADR 0006). Only the event loop can do
/// this, because only it owns the terminal.
fn hand_over_if_asked(terminal: &mut DefaultTerminal, app: &mut App) -> Result<()> {
    let Some(launch) = app.take_pending_handover() else {
        return Ok(());
    };
    ratatui::restore();
    let outcome = launch.run();
    // Re-initialise rather than assume the terminal survived an application that
    // took it over.
    *terminal = ratatui::try_init().map_err(|error| {
        Error::Other(format!("could not restart the terminal interface: {error}"))
    })?;
    app.launched(&launch, outcome);
    Ok(())
}

fn handle_browse(key: KeyEvent, browser: &mut Browser, mode: &mut Mode) -> Step {
    match key.code {
        KeyCode::Char('q') | KeyCode::Esc => return Step::Quit,
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Step::Quit,
        KeyCode::Char('j') | KeyCode::Down => browser.move_by(1),
        KeyCode::Char('k') | KeyCode::Up => browser.move_by(-1),
        KeyCode::Char('g') | KeyCode::Home => browser.select(0),
        KeyCode::Char('G') | KeyCode::End => browser.select(usize::MAX),
        KeyCode::Char('a') => browser.cycle_agent(),
        KeyCode::Char('r') => browser.cycle_recency(),
        KeyCode::Char('p') => browser.toggle_project(),
        KeyCode::Char('/') => *mode = Mode::Search,
        _ => {}
    }
    Step::Continue
}

fn handle_search(key: KeyEvent, browser: &mut Browser, mode: &mut Mode) -> Step {
    match key.code {
        KeyCode::Enter => *mode = Mode::Browse,
        KeyCode::Esc => {
            browser.set_query("");
            *mode = Mode::Browse;
        }
        KeyCode::Backspace => browser.pop_query(),
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Step::Quit,
        KeyCode::Up => browser.move_by(-1),
        KeyCode::Down => browser.move_by(1),
        KeyCode::Char(c) => browser.push_query(c),
        _ => {}
    }
    Step::Continue
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::discovery::SessionSummary;
    use std::path::PathBuf;

    fn summary(id: &str) -> SessionSummary {
        SessionSummary {
            agent: "claude",
            id: id.into(),
            path: PathBuf::from("/a.jsonl"),
            cwd: Some("/code".into()),
            started_at: None,
            ended_at: None,
            lines: 1,
            bytes: 1,
            title: Some(format!("session {id}")),
            first_message: Some("work".into()),
            mtime: 100,
        }
    }

    fn browser() -> Browser {
        let mut first = summary("aaa");
        first.agent = "claude";
        let mut second = summary("bbb");
        second.agent = "codex";
        Browser::new(vec![first, second], 100, None)
    }

    /// An app whose worker answers immediately and whose settle delay is zero,
    /// so the tests exercise the pump logic without waiting on either.
    fn app() -> App {
        let mut app = App::new(vec![summary("aaa")], 100, None, GlobalArgs::default());
        app.worker = Worker::spawn_with(Box::new(|summary: &SessionSummary| {
            Ok(LedgerPreview {
                goal: Some(format!("goal of {}", summary.id)),
                user_turns: 7,
                ..LedgerPreview::default()
            })
        }));
        app.settle_after = Duration::ZERO;
        app
    }

    /// Pump until the selected session has a preview, so no test depends on how
    /// fast the worker thread is scheduled.
    fn settle(app: &mut App) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.pump();
            if matches!(app.preview_state(), Some(PreviewState::Ready(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        panic!("the preview never arrived");
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn browse_keys_navigate_and_search_is_explicit() {
        let mut b = browser();
        let mut mode = Mode::Browse;
        assert_eq!(
            handle_browse(key(KeyCode::Char('j')), &mut b, &mut mode),
            Step::Continue
        );
        assert_eq!(b.selected_row(), 1);
        assert_eq!(
            handle_browse(key(KeyCode::Char('/')), &mut b, &mut mode),
            Step::Continue
        );
        assert_eq!(mode, Mode::Search, "/ must enter search, not type into it");
        assert!(b.query().is_empty());
    }

    #[test]
    fn letters_are_commands_in_browse_and_text_in_search() {
        let mut b = browser();
        let mut mode = Mode::Browse;
        // In browse, every one of these is a command and none becomes a query.
        for c in ['a', 'r', 'p'] {
            handle_browse(key(KeyCode::Char(c)), &mut b, &mut mode);
        }
        assert!(
            b.query().is_empty(),
            "commands must not leak into the query"
        );

        let mut b = browser();
        let mut mode = Mode::Search;
        for c in "session bbb".chars() {
            handle_search(key(KeyCode::Char(c)), &mut b, &mut mode);
        }
        assert_eq!(b.query(), "session bbb");
        assert_eq!(b.visible_len(), 1, "search must filter as it is typed");
    }

    #[test]
    fn escape_clears_the_query_and_returns_to_browse() {
        let mut b = browser();
        let mut mode = Mode::Search;
        for c in "zzz".chars() {
            handle_search(key(KeyCode::Char(c)), &mut b, &mut mode);
        }
        assert_eq!(b.visible_len(), 0);
        assert_eq!(
            handle_search(key(KeyCode::Esc), &mut b, &mut mode),
            Step::Continue
        );
        assert_eq!(mode, Mode::Browse);
        assert!(b.query().is_empty());
        assert_eq!(b.visible_len(), 2);
    }

    #[test]
    fn enter_keeps_the_query() {
        let mut b = browser();
        let mut mode = Mode::Search;
        for c in "session aaa".chars() {
            handle_search(key(KeyCode::Char(c)), &mut b, &mut mode);
        }
        handle_search(key(KeyCode::Enter), &mut b, &mut mode);
        assert_eq!(mode, Mode::Browse);
        assert_eq!(b.query(), "session aaa");
        assert_eq!(b.visible_len(), 1);
    }

    #[test]
    fn quit_is_available_from_both_modes() {
        let mut b = browser();
        let mut mode = Mode::Browse;
        assert_eq!(
            handle_browse(key(KeyCode::Char('q')), &mut b, &mut mode),
            Step::Quit
        );
        let mut mode = Mode::Search;
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(handle_search(ctrl_c, &mut b, &mut mode), Step::Quit);
    }

    #[test]
    fn the_selected_session_gets_a_preview() {
        let mut app = app();
        settle(&mut app);
        match app.preview_state() {
            Some(PreviewState::Ready(preview)) => {
                assert_eq!(preview.goal.as_deref(), Some("goal of aaa"));
                assert_eq!(preview.user_turns, 7);
            }
            other => panic!("expected a ready preview, got {other:?}"),
        }
    }

    #[test]
    fn a_session_is_read_once_and_then_remembered() {
        let mut app = app();
        settle(&mut app);
        let first = app.previews.len();
        // Many more pumps must not ask again: a cursor resting on a session
        // must not re-read it fifty times a second.
        for _ in 0..20 {
            app.pump();
        }
        assert_eq!(app.previews.len(), first, "the preview must be cached");
        assert!(matches!(app.preview_state(), Some(PreviewState::Ready(_))));
    }

    #[test]
    fn a_failed_read_is_remembered_as_a_failure_and_not_retried() {
        let mut app = app();
        app.worker = Worker::spawn_with(Box::new(|_: &SessionSummary| {
            Err(crate::error::Error::Usage("cannot read".to_string()))
        }));
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            app.pump();
            if matches!(app.preview_state(), Some(PreviewState::Failed(_))) {
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        match app.preview_state() {
            Some(PreviewState::Failed(reason)) => assert!(reason.contains("cannot read")),
            other => panic!("expected a failure, got {other:?}"),
        }
        // Still one entry after more pumps: no retry storm on a bad file.
        for _ in 0..10 {
            app.pump();
        }
        assert_eq!(app.previews.len(), 1);
    }

    #[test]
    fn the_cache_is_bounded() {
        let mut app = App::new(vec![summary("aaa")], 100, None, GlobalArgs::default());
        for index in 0..CACHE_MAX + 10 {
            app.remember(format!("id{index:04}"), PreviewState::Loading);
        }
        assert!(
            app.previews.len() <= CACHE_MAX,
            "the cache must not grow without bound"
        );
    }

    #[test]
    fn e_opens_the_form_in_browse_and_is_a_letter_in_search() {
        let mut browsing = app();
        browsing.handle(key(KeyCode::Char('e')));
        assert!(browsing.form.is_some(), "`e` must open the extraction form");
        assert_eq!(browsing.mode, Mode::Form);

        // In search it is a character, because someone may search for "entry".
        let mut searching = app();
        searching.handle(key(KeyCode::Char('/')));
        searching.handle(key(KeyCode::Char('e')));
        assert!(searching.form.is_none());
        assert_eq!(searching.browser.query(), "e");
    }

    #[test]
    fn escape_closes_the_form_without_running_anything() {
        let mut app = app();
        app.handle(key(KeyCode::Char('e')));
        app.handle(key(KeyCode::Esc));
        assert!(app.form.is_none());
        assert_eq!(app.mode, Mode::Browse);
        assert!(matches!(app.run, RunState::Idle));
    }

    #[test]
    fn a_form_with_no_destination_refuses_to_run() {
        let mut app = app();
        app.handle(key(KeyCode::Char('e')));
        app.form.as_mut().expect("form").set("out", "");
        app.start_run();
        match &app.run {
            RunState::Failed(reason) => assert!(reason.contains("--out"), "{reason}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_bad_value_fails_before_the_worker_is_asked() {
        let mut app = app();
        app.handle(key(KeyCode::Char('e')));
        app.form
            .as_mut()
            .expect("form")
            .set("max_bad_lines", "half");
        app.start_run();
        match &app.run {
            RunState::Failed(reason) => assert!(reason.contains("max-bad-lines"), "{reason}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    /// The whole point of the form: filling it in and pressing enter produces an
    /// artifact on disk, offline, through the real pipeline.
    #[test]
    fn running_the_form_writes_a_handoff() {
        let output = tempfile::tempdir().expect("temp dir");
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/claude/basic.jsonl");
        let mut session = summary("fixture");
        session.path = fixture;

        let mut app = App::new(vec![session], 100, None, GlobalArgs::default());
        app.open_form();
        {
            let form = app.form.as_mut().expect("form");
            form.set("llm", "none");
            form.set("no_verify", "true");
            form.set("out", &output.path().to_string_lossy());
        }
        app.start_run();
        assert!(
            matches!(app.run, RunState::Running { .. }),
            "the run must start: {:?}",
            app.run
        );

        let deadline = Instant::now() + Duration::from_secs(60);
        while Instant::now() < deadline && !matches!(app.run, RunState::Done(_)) {
            app.pump();
            if matches!(app.run, RunState::Failed(_)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }

        match &app.run {
            RunState::Done(outcome) => {
                assert_eq!(outcome.reference, "claude:fixture");
                // FR-016: the run opened the artifact by itself, at L0.
                assert_eq!(app.mode, Mode::Canvas, "the artifact opens itself");
                let canvas = app.canvas.as_ref().expect("the canvas is up");
                assert_eq!(canvas.current(), 0, "and starts at L0");
                assert!(
                    canvas.layers().iter().any(|layer| layer.name == "L0"),
                    "L0 is the layer the artifact was rendered with"
                );
                assert!(
                    outcome.paths.len() >= 5,
                    "a directory destination gets the full set, got {:?}",
                    outcome.paths
                );
                assert!(outcome.handoff.exists(), "handoff.md must exist");
                assert!(
                    outcome.handoff.ends_with("handoff.md"),
                    "{:?}",
                    outcome.handoff
                );
                let written = std::fs::read_to_string(&outcome.handoff).expect("read back");
                assert!(
                    written.contains("evt"),
                    "the artifact must carry provenance pointers"
                );
            }
            other => panic!("expected a finished extraction, got {other:?}"),
        }
    }

    fn agent_fixture(
        id: &'static str,
        installed: bool,
        version: Option<&str>,
        verified: &'static str,
    ) -> Agent {
        Agent {
            id,
            label: if id == "claude" {
                "Claude Code"
            } else {
                "Codex CLI"
            },
            program: installed.then(|| PathBuf::from(format!("/usr/local/bin/{id}"))),
            version: version.map(str::to_string),
            store: PathBuf::from("/tmp/store"),
            store_exists: false,
            verified_against: verified,
        }
    }

    /// An app with a session that has a real project directory, so the handoff
    /// has somewhere to write.
    fn handoff_app() -> (tempfile::TempDir, App) {
        let project = tempfile::tempdir().expect("tempdir");
        let mut session = summary("aaa");
        session.cwd = Some(project.path().to_string_lossy().into_owned());

        let mut app = App::new(vec![session], 100, None, GlobalArgs::default());
        app.agents = Some(vec![
            agent_fixture("claude", true, Some("2.1.268"), "2.1.268"),
            agent_fixture("codex", false, None, "0.153.4"),
        ]);
        (project, app)
    }

    /// Write an artifact that claims to come from `session_id`.
    fn write_artifact(destination: &Path, session_id: &str) {
        std::fs::create_dir_all(destination).expect("create destination");
        std::fs::write(
            destination.join("handoff.md"),
            format!(
                "source: {{agent: claude, session: {session_id}}}\n\n## L0 \u{b7} Brief\n\nthe goal [evt 41]\n"
            ),
        )
        .expect("write artifact");
    }

    #[test]
    fn h_opens_the_handoff_straight_from_a_session() {
        // The main line: pick a session, say which agent continues it, go. No
        // extraction first, no flag form, no model.
        let (_project, mut app) = handoff_app();
        app.handle(key(KeyCode::Char('h')));
        assert_eq!(app.mode, Mode::Handoff);
        assert!(matches!(app.handoff, Some(HandoffState::Choosing)));
        assert!(
            app.handoff_session.is_some(),
            "the session is taken when the handoff opens, so moving the cursor cannot change it"
        );
    }

    #[test]
    fn the_confirmation_says_which_agent_and_exactly_where() {
        let (project, mut app) = handoff_app();
        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Enter));

        match &app.handoff {
            Some(HandoffState::Confirming { agent, destination }) => {
                assert_eq!(*agent, 0);
                assert!(
                    destination.is_absolute(),
                    "a relative path answers nothing: {destination:?}"
                );
                assert_eq!(destination, &project.path().join(".sctxx"));
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }
    }

    #[test]
    fn confirming_extracts_deterministically_and_then_launches() {
        let (project, mut app) = handoff_app();
        assert!(
            !project.path().join(".sctxx/handoff.md").exists(),
            "nothing has been extracted yet"
        );

        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Enter)); // choose claude
        app.handle(key(KeyCode::Enter)); // confirm

        assert!(
            matches!(app.run, RunState::Running { .. }),
            "step one is the extraction: {:?}",
            app.run
        );
        assert_eq!(app.pending_launch, Some(0), "step two is the launch");

        // And it spends nothing: the deterministic artifact is what a handoff
        // needs, and 800k tokens through a model is not what "hand this on"
        // means.
        let options = app.deterministic_options("claude:aaa").expect("options");
        assert!(
            matches!(options.llm, crate::llm::Selection::None),
            "the handoff must not call a model"
        );
    }

    #[test]
    fn an_artifact_that_already_belongs_to_this_session_is_reused() {
        let (project, mut app) = handoff_app();
        write_artifact(&project.path().join(".sctxx"), "aaa");

        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Enter));
        app.handle(key(KeyCode::Enter));

        assert!(
            !matches!(app.run, RunState::Running { .. }),
            "an extraction that is already done is not redone"
        );
        assert!(
            app.pending_handover.is_some(),
            "it goes straight to the launch"
        );
    }

    #[test]
    fn an_artifact_belonging_to_another_session_is_not_reused() {
        let (project, mut app) = handoff_app();
        write_artifact(&project.path().join(".sctxx"), "someone-elses-session");

        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Enter));
        app.handle(key(KeyCode::Enter));

        assert!(
            matches!(app.run, RunState::Running { .. }),
            "another session's artifact is not this session's context"
        );
        assert_eq!(app.pending_launch, Some(0));
    }

    #[test]
    fn esc_backs_out_of_the_confirmation_without_running_anything() {
        let (_project, mut app) = handoff_app();
        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Enter));
        app.handle(key(KeyCode::Esc));
        assert!(matches!(app.handoff, Some(HandoffState::Choosing)));
        assert!(!matches!(app.run, RunState::Running { .. }));

        app.handle(key(KeyCode::Esc));
        assert!(app.handoff.is_none());
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn choosing_an_agent_that_is_not_installed_explains_rather_than_launching() {
        let (_project, mut app) = handoff_app();
        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Char('j')));
        assert_eq!(app.handoff_cursor, 1);
        app.handle(key(KeyCode::Enter));
        assert!(
            matches!(app.handoff, Some(HandoffState::Choosing)),
            "an agent that is not installed must not reach a confirmation"
        );
        let note = app.handoff_note.clone().expect("a reason");
        assert!(note.contains("not installed"), "{note}");
    }

    #[test]
    fn the_cursor_is_clamped_to_the_agents_that_were_found() {
        let (_project, mut app) = handoff_app();
        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Char('k')));
        assert_eq!(app.handoff_cursor, 0);
        for _ in 0..5 {
            app.handle(key(KeyCode::Char('j')));
        }
        assert_eq!(app.handoff_cursor, 1, "two agents, so one is the last");
    }

    #[test]
    fn coming_back_from_an_agent_reports_what_it_did() {
        let (project, mut app) = handoff_app();
        write_artifact(&project.path().join(".sctxx"), "aaa");
        app.handoff_session = Some(app.browser.selected().cloned().expect("session"));
        app.launch_now(0);
        let launch = app.take_pending_handover().expect("a launch");

        app.launched(&launch, Ok(0));
        assert!(
            app.handoff_note
                .as_deref()
                .unwrap_or_default()
                .contains("finished")
        );
        assert_eq!(app.mode, Mode::Handoff, "the flow stays open to run again");

        app.launched(&launch, Ok(3));
        assert!(
            app.handoff_note
                .as_deref()
                .unwrap_or_default()
                .contains('3')
        );

        app.launched(&launch, Err(crate::error::Error::Other("boom".into())));
        assert!(
            app.handoff_note
                .as_deref()
                .unwrap_or_default()
                .contains("boom")
        );
    }

    #[test]
    fn moving_the_cursor_restarts_the_settle_delay() {
        let mut app = App::new(
            vec![summary("aaa"), summary("bbb")],
            100,
            None,
            GlobalArgs::default(),
        );
        app.settle_after = Duration::from_secs(60);
        app.pump();
        assert!(app.settle.is_some(), "the first selection arms the delay");
        assert!(app.previews.is_empty(), "nothing is read before it settles");
        // A key press means the developer is still moving.
        app.handle(key(KeyCode::Char('j')));
        assert!(app.settle.is_none(), "a key press restarts the delay");
    }
}
