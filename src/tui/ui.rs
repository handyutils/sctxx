//! Rendering for the session browser.
//!
//! Rendering owns no state: it reads an [`App`] and draws it. Every decision
//! about *what* is on screen already happened in `browser.rs` and `mod.rs`,
//! which is why those files are the ones with the tests.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::browser::Browser;
use super::preview::LedgerPreview;
use super::{App, PreviewState};
use crate::VERSION;

/// Accent colours kept in one place so the two screens cannot drift.
const ACCENT: Color = Color::LightGreen;
const DIM: Color = Color::DarkGray;
const WARN: Color = Color::Red;

/// How much of a quoted value the pane keeps before it elides.
const VALUE_MAX: usize = 64;

pub(super) fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(4),    // body
            Constraint::Length(2), // keys + status
        ])
        .split(area);

    header(frame, rows[0], &app.browser);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(rows[1]);
    list(frame, body[0], &app.browser);
    preview(frame, body[1], &app.browser, app.preview_state());

    footer(frame, rows[2], &app.browser, app.searching());
}

fn header(frame: &mut Frame, area: Rect, browser: &Browser) {
    let title = Line::from(vec![
        Span::styled(
            " sctxx ",
            Style::default()
                .fg(Color::Black)
                .bg(ACCENT)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" v{VERSION}  "), Style::default().fg(DIM)),
        Span::styled(
            "sessions",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {} of {}", browser.visible_len(), browser.total()),
            Style::default().fg(DIM),
        ),
    ]);
    frame.render_widget(Paragraph::new(title), area);
}

fn list(frame: &mut Frame, area: Rect, browser: &Browser) {
    let items: Vec<ListItem> = browser
        .visible()
        .map(|session| {
            let when = session
                .started_at
                .as_deref()
                .and_then(|t| t.get(..16))
                .unwrap_or("???????????????");
            let what = session
                .first_message
                .as_deref()
                .or(session.title.as_deref())
                .unwrap_or("")
                .replace(['\n', '\r'], " ");
            let text = Line::from(vec![
                Span::styled(
                    format!(" {:<8}", session.agent),
                    Style::default().fg(ACCENT),
                ),
                Span::styled(format!("{when}  "), Style::default().fg(DIM)),
                Span::raw(format!("{:<20}", short_id(&session.id))),
                Span::styled(truncate(&what, 60), Style::default().fg(DIM)),
            ]);
            ListItem::new(text)
        })
        .collect();

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" sessions "))
        .highlight_style(
            Style::default()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌");

    let mut state = ListState::default();
    if browser.visible_len() > 0 {
        state.select(Some(browser.selected_row()));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

/// The preview pane: facts about the file, then what is *in* it.
///
/// The facts come from discovery and are on screen immediately. The contents
/// come from the ledgers and arrive later, so the pane says it is reading rather
/// than looking empty — and until they arrive it shows the one thing discovery
/// already knows about the conversation, which is how it opens.
fn preview(frame: &mut Frame, area: Rect, browser: &Browser, state: Option<&PreviewState>) {
    let block = Block::default().borders(Borders::ALL).title(" preview ");
    let Some(session) = browser.selected() else {
        let empty = Paragraph::new(Line::styled(
            "no session selected",
            Style::default().fg(DIM),
        ))
        .block(block);
        frame.render_widget(empty, area);
        return;
    };

    let mut lines = vec![
        field("agent", session.agent.to_string()),
        field("id", session.id.clone()),
        field(
            "when",
            session.started_at.clone().unwrap_or_else(|| "?".into()),
        ),
        field(
            "cwd",
            truncate(&session.cwd.clone().unwrap_or_else(|| "?".into()), 200),
        ),
        field(
            "size",
            format!("{} lines, {} KB", session.lines, session.bytes / 1024),
        ),
    ];
    if let Some(title) = &session.title {
        lines.push(field("title", truncate(title, 200)));
    }
    lines.push(field("path", session.path.display().to_string()));

    match state {
        Some(PreviewState::Ready(ledger)) => contents(&mut lines, ledger),
        Some(PreviewState::Failed(reason)) => {
            lines.push(Line::default());
            lines.push(Line::styled(
                " this session could not be read",
                Style::default().fg(WARN),
            ));
            lines.push(Line::styled(
                format!(" {reason}"),
                Style::default().fg(WARN),
            ));
            lines.push(Line::styled(
                " it stays selectable: `sctxx show` reports the same problem",
                Style::default().fg(DIM),
            ));
        }
        _ => {
            // Discovery's first message is instant, so it fills the gap while
            // the transcript is being read.
            lines.push(Line::default());
            lines.push(Line::styled("first message", Style::default().fg(ACCENT)));
            lines.push(Line::raw(
                session
                    .first_message
                    .clone()
                    .unwrap_or_else(|| "(none recorded)".to_string()),
            ));
            lines.push(Line::default());
            lines.push(Line::styled(
                "reading the transcript…",
                Style::default().fg(DIM),
            ));
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// What the session actually contains, from the deterministic ledgers.
fn contents(lines: &mut Vec<Line<'static>>, ledger: &LedgerPreview) {
    lines.push(Line::default());
    lines.push(Line::styled(" contents", Style::default().fg(ACCENT)));

    if let Some(goal) = &ledger.goal {
        lines.push(field("goal", truncate(goal, 300)));
    }
    lines.push(field(
        "turns",
        format!("{} user turns on the active branch", ledger.user_turns),
    ));
    lines.push(field(
        "events",
        format!("{} live of {}", ledger.live_events, ledger.total_events),
    ));
    lines.push(field("files", format!("{} touched", ledger.files_touched)));
    for path in &ledger.top_files {
        lines.push(sub(truncate(path, VALUE_MAX)));
    }

    if let Some(command) = &ledger.last_command {
        let status = ledger.last_command_status.unwrap_or("unknown");
        lines.push(field(
            "last",
            format!("{}  [{status}]", truncate(command, VALUE_MAX)),
        ));
    }

    let error_style = if ledger.unresolved_errors > 0 {
        Style::default().fg(WARN)
    } else {
        Style::default().fg(DIM)
    };
    lines.push(Line::from(vec![
        Span::styled(" errors  ", Style::default().fg(DIM)),
        Span::styled(
            format!("{} unresolved", ledger.unresolved_errors),
            error_style,
        ),
    ]));
    for error in &ledger.errors {
        lines.push(Line::styled(
            format!("         {}", truncate(error, VALUE_MAX)),
            Style::default().fg(WARN),
        ));
    }

    if ledger.compactions > 0 {
        // A reset is not a re-anchor: it means the early history is gone.
        let detail = if ledger.compaction_resets > 0 {
            format!(
                "{} provider compaction(s), {} discarding history",
                ledger.compactions, ledger.compaction_resets
            )
        } else {
            format!("{} provider compaction(s)", ledger.compactions)
        };
        lines.push(field("compact", detail));
    }

    if !ledger.is_clean() {
        let first = ledger.diagnostic.clone().unwrap_or_default();
        lines.push(field(
            "notes",
            format!(
                "{} diagnostic(s): {}",
                ledger.diagnostics,
                truncate(&first, VALUE_MAX)
            ),
        ));
    }
}

fn footer(frame: &mut Frame, area: Rect, browser: &Browser, search: bool) {
    let keys = if search {
        " type to filter · enter keep · esc clear "
    } else {
        " j/k move · / search · a agent · r date · p project · q quit "
    };
    let lines = vec![
        Line::styled(keys, Style::default().fg(if search { ACCENT } else { DIM })),
        Line::from(vec![
            Span::styled(" filters: ", Style::default().fg(DIM)),
            Span::raw(browser.filter_summary()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn field(name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {name:<7}"), Style::default().fg(DIM)),
        Span::raw(value),
    ])
}

/// A value under the field column, for the lines that belong to the one above.
fn sub(value: String) -> Line<'static> {
    Line::styled(format!("         {value}"), Style::default().fg(DIM))
}

/// The first eight characters of an id: enough to tell sessions apart in a
/// list, short enough to leave room for the message.
fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// Truncate on a character boundary. Never byte-slice untrusted text.
fn truncate(text: &str, max: usize) -> String {
    let mut out: String = text.chars().take(max).collect();
    if text.chars().count() > max {
        out.push('…');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::discovery::SessionSummary;
    use crate::tui::PreviewState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::path::PathBuf;

    fn summary() -> SessionSummary {
        SessionSummary {
            agent: "claude",
            id: "1367d688-7dcd-43d8-8d4a-30210a3137f6".into(),
            path: PathBuf::from("/sessions/1367d688.jsonl"),
            cwd: Some("/code".into()),
            started_at: Some("2026-09-01T15:41:47.144Z".into()),
            ended_at: None,
            lines: 98_597,
            bytes: 224_000_000,
            title: Some("a title".into()),
            first_message: Some("the opening message".into()),
            mtime: 100,
        }
    }

    fn app() -> App {
        App::new(vec![summary()], 100, None)
    }

    fn ledger() -> LedgerPreview {
        LedgerPreview {
            goal: Some("make the parser stop eating brackets".into()),
            user_turns: 42,
            files_touched: 12,
            top_files: vec!["src/lexer.rs".into(), "src/parser.rs".into()],
            last_command: Some("cargo test --lib".into()),
            last_command_status: Some("FAILED"),
            unresolved_errors: 2,
            errors: vec!["error[E0308]: mismatched types".into()],
            compactions: 3,
            compaction_resets: 1,
            live_events: 6_753,
            total_events: 14_151,
            diagnostics: 2,
            diagnostic: Some("line 91: unexpected end of JSON".into()),
        }
    }

    /// Render the whole screen and return it as text, so a test asserts on what
    /// a developer would actually read rather than on a frame's diff stream.
    fn screen(app: &App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("test terminal");
        terminal
            .draw(|frame| draw(frame, app))
            .expect("drawing must not fail");
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| {
                        buffer
                            .cell((x, y))
                            .map(|cell| cell.symbol())
                            .unwrap_or(" ")
                            .to_string()
                    })
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn the_pane_says_it_is_reading_until_the_preview_arrives() {
        let text = screen(&app(), 150, 44);
        assert!(
            text.contains("reading the transcript"),
            "the pane must not look empty while it works:\n{text}"
        );
        assert!(
            text.contains("the opening message"),
            "discovery's first message should fill the gap:\n{text}"
        );
    }

    #[test]
    fn a_ready_preview_draws_the_ledger_section() {
        let mut app = app();
        app.previews.insert(
            "1367d688-7dcd-43d8-8d4a-30210a3137f6".into(),
            PreviewState::Ready(Box::new(ledger())),
        );
        let text = screen(&app, 150, 44);

        for expected in [
            "contents",
            "42 user turns",
            "6753 live of 14151",
            "12 touched",
            "src/lexer.rs",
            "cargo test --lib",
            "FAILED",
            "2 unresolved",
            "error[E0308]",
            "3 provider compaction(s), 1 discarding history",
            "2 diagnostic(s)",
        ] {
            assert!(text.contains(expected), "missing {expected:?} in:\n{text}");
        }
        // The loading line must be gone once the answer is in.
        assert!(
            !text.contains("reading the transcript"),
            "the loading line must be replaced, not left behind:\n{text}"
        );
    }

    #[test]
    fn a_failed_read_shows_its_reason_and_stays_selectable() {
        let mut app = app();
        app.previews.insert(
            "1367d688-7dcd-43d8-8d4a-30210a3137f6".into(),
            PreviewState::Failed("permission denied".into()),
        );
        let text = screen(&app, 150, 44);
        assert!(text.contains("could not be read"), "{text}");
        assert!(text.contains("permission denied"), "{text}");
        assert!(
            text.contains("1367d688"),
            "the session must still be listed, not dropped:\n{text}"
        );
    }

    #[test]
    fn a_clean_session_reports_no_problems() {
        let mut app = app();
        app.previews.insert(
            "1367d688-7dcd-43d8-8d4a-30210a3137f6".into(),
            PreviewState::Ready(Box::new(LedgerPreview {
                user_turns: 1,
                live_events: 2,
                total_events: 2,
                errors: Vec::new(),
                ..LedgerPreview::default()
            })),
        );
        let text = screen(&app, 150, 44);
        assert!(text.contains("0 unresolved"), "{text}");
        // Nothing to report means no compaction or diagnostic line at all.
        assert!(!text.contains("provider compaction"), "{text}");
        assert!(!text.contains("diagnostic(s)"), "{text}");
    }

    #[test]
    fn the_header_and_the_list_survive_a_narrow_terminal() {
        // Every pane decision has to hold at 80 columns, which is where long
        // paths and ids start to collide.
        let text = screen(&app(), 80, 24);
        assert!(text.contains("sessions"), "{text}");
        assert!(text.contains("preview"), "{text}");
        assert!(text.contains("filters:"), "{text}");
    }
}
