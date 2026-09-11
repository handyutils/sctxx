//! The interactive mode behind `sctxx --tui`.
//!
//! This module owns the terminal and the event loop; it owns as little else as
//! possible. Everything a test can check lives in [`browser`], because an event
//! loop needs a TTY and a test does not have one.
//!
//! The screen is a view over the same discovery layer the CLI uses: it lists
//! sessions with `adapters::discovery`, so nothing here can disagree with
//! `sctxx list`.

pub mod browser;

mod ui;

use crate::adapters::discovery;
use crate::cli::GlobalArgs;
use crate::error::{Error, Result};
use browser::Browser;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::DefaultTerminal;
use std::io::IsTerminal;
use std::time::{SystemTime, UNIX_EPOCH};

/// Which half of the interaction the keyboard is in.
///
/// A mode rather than bare keystrokes because `a`, `r`, `p` and `q` are both
/// commands and letters a developer might type into a search box. Guessing
/// would make the search unusable for anyone searching for "query".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Browse,
    Search,
}

/// What the event loop decided to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    Continue,
    Quit,
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
    let mut browser = Browser::new(sessions, now, project);

    let mut terminal = ratatui::try_init().map_err(|error| {
        Error::Other(format!("could not start the terminal interface: {error}"))
    })?;
    let outcome = event_loop(&mut terminal, &mut browser);
    // Always restore, including on error: leaving a terminal in raw mode after
    // a crash is worse than the crash.
    ratatui::restore();
    outcome.map(|_| 0)
}

fn event_loop(terminal: &mut DefaultTerminal, browser: &mut Browser) -> Result<()> {
    let mut mode = Mode::Browse;
    loop {
        terminal
            .draw(|frame| ui::draw(frame, browser, mode == Mode::Search))
            .map_err(|error| Error::Other(format!("could not draw: {error}")))?;

        let Event::Key(key) = event::read()
            .map_err(|error| Error::Other(format!("could not read a key press: {error}")))?
        else {
            continue;
        };
        // Windows reports press and release; act on press only.
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let step = match mode {
            Mode::Browse => handle_browse(key, browser, &mut mode),
            Mode::Search => handle_search(key, browser, &mut mode),
        };
        if step == Step::Quit {
            return Ok(());
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

    fn browser() -> Browser {
        let sessions = vec![
            SessionSummary {
                agent: "claude",
                id: "aaa".into(),
                path: PathBuf::from("/a.jsonl"),
                cwd: Some("/code".into()),
                started_at: None,
                ended_at: None,
                lines: 1,
                bytes: 1,
                title: Some("alpha".into()),
                first_message: Some("work".into()),
                mtime: 100,
            },
            SessionSummary {
                agent: "codex",
                id: "bbb".into(),
                path: PathBuf::from("/b.jsonl"),
                cwd: Some("/code".into()),
                started_at: None,
                ended_at: None,
                lines: 1,
                bytes: 1,
                title: Some("beta".into()),
                first_message: Some("work".into()),
                mtime: 90,
            },
        ];
        Browser::new(sessions, 100, None)
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
        for c in "beta".chars() {
            handle_search(key(KeyCode::Char(c)), &mut b, &mut mode);
        }
        assert_eq!(b.query(), "beta");
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
        for c in "alpha".chars() {
            handle_search(key(KeyCode::Char(c)), &mut b, &mut mode);
        }
        handle_search(key(KeyCode::Enter), &mut b, &mut mode);
        assert_eq!(mode, Mode::Browse);
        assert_eq!(b.query(), "alpha");
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
}
