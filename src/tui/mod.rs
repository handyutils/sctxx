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

mod form;
mod preview;
mod ui;
mod work;

use crate::adapters::discovery::{self, SessionSummary};
use crate::agents::Agent;
use crate::agents::seeding::Launch;
use crate::cli::GlobalArgs;
use crate::error::{Error, Result};
use browser::Browser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use form::{Control, Form};
use preview::LedgerPreview;
use ratatui::DefaultTerminal;
use std::collections::BTreeMap;
use std::io::IsTerminal;
use std::path::PathBuf;
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
}

/// What the event loop decided to do next.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Step {
    Continue,
    Quit,
    /// Hand the terminal to a launched agent and take it back when it exits
    /// (ADR 0006). Only the event loop can do this, because only it owns the
    /// terminal.
    HandOver(Box<Launch>),
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
    /// Showing exactly what will run, waiting for a yes.
    Confirming(Box<Launch>),
}

/// The whole of the TUI's state.
struct App {
    browser: Browser,
    mode: Mode,
    /// Previews by session id, so returning to a session is instant.
    previews: BTreeMap<String, PreviewState>,
    /// The extraction form, when one is open.
    form: Option<Form>,
    /// The handoff flow, when it is open.
    handoff: Option<HandoffState>,
    /// Which detected agent the cursor is on.
    handoff_cursor: usize,
    /// What the last handoff did, so the developer sees it when they come back.
    handoff_note: Option<String>,
    /// The agents on this machine, once they have been looked for. `None` means
    /// nobody has asked yet, which is why the pane can say so.
    agents: Option<Vec<Agent>>,
    /// What the extraction pane is showing.
    run: RunState,
    worker: Worker,
    /// The selection the settle delay is counting for, and since when.
    settle: Option<(String, Instant)>,
    /// A field rather than the constant so a test removes the wait instead of
    /// sleeping through it.
    settle_after: Duration,
    /// The flags this invocation was given, for the form to convert against.
    global: GlobalArgs,
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
            handoff: None,
            handoff_cursor: 0,
            handoff_note: None,
            agents: None,
            run: RunState::Idle,
            worker: Worker::spawn(),
            settle: None,
            settle_after: SETTLE,
            global,
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
                work::Done::Agents(detected) => {
                    self.agents = Some(detected);
                }
                work::Done::Extracted { result } => {
                    self.run = match result {
                        Ok(outcome) => RunState::Done(outcome),
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

    /// Open the handoff, once an extraction has produced something to hand on.
    fn open_handoff(&mut self) {
        if !matches!(self.run, RunState::Done(_)) {
            // Nothing extracted yet, so there is no artifact to hand over.
            return;
        }
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

    /// Build the launch for the agent under the cursor.
    ///
    /// Everything that can be wrong — no artifact, no binary, an unreadable
    /// handoff — is reported here, before the terminal is handed over.
    fn build_launch(&self, cursor: usize) -> Result<Launch> {
        let agents = self.agents.as_ref().ok_or_else(|| {
            Error::Usage("still looking for the agents installed here".to_string())
        })?;
        let agent = agents
            .get(cursor)
            .ok_or_else(|| Error::Usage("no agent is selected".to_string()))?;
        let artifact = match &self.run {
            RunState::Done(outcome) => outcome.handoff.clone(),
            _ => {
                return Err(Error::Usage(
                    "extract a session before handing it on".to_string(),
                ));
            }
        };
        let cwd = self
            .browser
            .selected()
            .and_then(|session| session.cwd.clone());
        Launch::interactive(agent, &artifact, cwd.as_deref().map(std::path::Path::new))
    }

    /// Choose an agent, then confirm the command. Two steps, never one.
    fn handle_handoff(&mut self, key: KeyEvent) -> Step {
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            return Step::Quit;
        }
        let count = self.agents.as_ref().map(Vec::len).unwrap_or(0);
        match self.handoff.clone() {
            Some(HandoffState::Choosing) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => {
                    self.handoff = None;
                    self.mode = Mode::Browse;
                }
                // `count` is zero while detection is still in flight, and
                // there is nothing to move onto yet.
                KeyCode::Char('j') | KeyCode::Down if count > 0 => {
                    self.handoff_cursor = (self.handoff_cursor + 1).min(count - 1);
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    self.handoff_cursor = self.handoff_cursor.saturating_sub(1);
                }
                KeyCode::Enter => match self.build_launch(self.handoff_cursor) {
                    Ok(launch) => {
                        self.handoff_note = None;
                        self.handoff = Some(HandoffState::Confirming(Box::new(launch)));
                    }
                    Err(error) => {
                        // The reason is shown where the choice was, so the
                        // developer can pick a different agent or go and extract.
                        self.handoff_note = Some(error.to_string());
                    }
                },
                _ => {}
            },
            Some(HandoffState::Confirming(launch)) => match key.code {
                KeyCode::Esc => self.handoff = Some(HandoffState::Choosing),
                KeyCode::Enter | KeyCode::Char('y') => return Step::HandOver(launch),
                _ => {}
            },
            None => {
                self.mode = Mode::Browse;
            }
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
            return;
        }
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
        self.worker
            .extract(&session, options, PathBuf::from(destination));
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

        let step = match self.mode {
            Mode::Browse => handle_browse(key, &mut self.browser, &mut self.mode),
            Mode::Search => handle_search(key, &mut self.browser, &mut self.mode),
            Mode::Form => return self.handle_form(key),
            Mode::Handoff => return self.handle_handoff(key),
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
                self.start_run();
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
        match app.handle(key) {
            Step::Quit => return Ok(()),
            Step::HandOver(launch) => {
                // A coding agent is a full-screen application, so it gets the
                // whole terminal rather than a pane inside this one (ADR 0006).
                ratatui::restore();
                let outcome = launch.run();
                // Re-initialise rather than assume the terminal survived an
                // application that took it over.
                *terminal = ratatui::try_init().map_err(|error| {
                    Error::Other(format!("could not restart the terminal interface: {error}"))
                })?;
                app.launched(&launch, outcome);
            }
            Step::Continue => {}
        }
    }
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

    /// An app that has already extracted, with the handoff file really on disk —
    /// because the launch checks it before anything else happens.
    fn extraction_app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().expect("tempdir");
        let handoff = dir.path().join("handoff.md");
        std::fs::write(&handoff, "# handoff\n\n[evt 1-2] something\n").expect("write");

        let mut app = app();
        app.run = RunState::Done(Box::new(crate::tui::work::Outcome {
            reference: "claude:aaa".into(),
            handoff: handoff.clone(),
            paths: vec![handoff],
            warnings: Vec::new(),
            git_warning: None,
            live_events: 1,
            total_events: 2,
        }));
        app.agents = Some(vec![
            agent_fixture("claude", true, Some("2.1.268"), "2.1.268"),
            agent_fixture("codex", false, None, "0.153.4"),
        ]);
        (dir, app)
    }

    #[test]
    fn h_does_nothing_until_something_has_been_extracted() {
        // There is no handoff to make yet, and inventing one would be worse
        // than doing nothing.
        let mut app = app();
        app.handle(key(KeyCode::Char('h')));
        assert!(app.handoff.is_none());
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn the_handoff_is_two_steps_and_the_first_one_runs_nothing() {
        let (_dir, mut app) = extraction_app();
        app.handle(key(KeyCode::Char('h')));
        assert_eq!(app.mode, Mode::Handoff);
        assert!(matches!(app.handoff, Some(HandoffState::Choosing)));

        // Choosing builds the command and shows it; it does not launch.
        assert_eq!(app.handle(key(KeyCode::Enter)), Step::Continue);
        match &app.handoff {
            Some(HandoffState::Confirming(launch)) => {
                assert_eq!(launch.agent, "claude");
                assert!(launch.display.contains("--append-system-prompt-file"));
                assert!(launch.display.contains("handoff.md"));
            }
            other => panic!("expected a confirmation, got {other:?}"),
        }

        // Only the second step hands over the terminal.
        assert!(matches!(app.handle(key(KeyCode::Enter)), Step::HandOver(_)));
    }

    #[test]
    fn esc_backs_out_of_the_confirmation_without_running_anything() {
        let (_dir, mut app) = extraction_app();
        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Enter));
        assert_eq!(app.handle(key(KeyCode::Esc)), Step::Continue);
        assert!(matches!(app.handoff, Some(HandoffState::Choosing)));
        // And esc again leaves the handoff entirely.
        app.handle(key(KeyCode::Esc));
        assert!(app.handoff.is_none());
        assert_eq!(app.mode, Mode::Browse);
    }

    #[test]
    fn choosing_an_agent_that_is_not_installed_explains_rather_than_launching() {
        let (_dir, mut app) = extraction_app();
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
    fn a_handoff_that_vanished_is_caught_before_the_terminal_is_handed_over() {
        let (dir, mut app) = extraction_app();
        std::fs::remove_file(dir.path().join("handoff.md")).expect("remove the artifact");
        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Enter));
        let note = app.handoff_note.clone().expect("a reason");
        assert!(note.contains("not readable"), "{note}");
        assert!(matches!(app.handoff, Some(HandoffState::Choosing)));
    }

    #[test]
    fn the_cursor_is_clamped_to_the_agents_that_were_found() {
        let (_dir, mut app) = extraction_app();
        app.handle(key(KeyCode::Char('h')));
        app.handle(key(KeyCode::Char('k')));
        assert_eq!(app.handoff_cursor, 0);
        app.handle(key(KeyCode::Char('j')));
        app.handle(key(KeyCode::Char('j')));
        app.handle(key(KeyCode::Char('j')));
        assert_eq!(app.handoff_cursor, 1, "two agents, so one is the last");
    }

    #[test]
    fn coming_back_from_an_agent_reports_what_it_did() {
        let (_dir, mut app) = extraction_app();
        let launch = app.build_launch(0).expect("a launch for claude");

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
