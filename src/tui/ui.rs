//! Rendering for the session browser.
//!
//! Rendering owns no state: it reads a [`Browser`] and draws it. Every decision
//! about *what* is on screen already happened in `browser.rs`, which is why
//! that file is the one with the tests.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use super::browser::Browser;
use crate::VERSION;

/// Accent colours kept in one place so the two screens cannot drift.
const ACCENT: Color = Color::LightGreen;
const DIM: Color = Color::DarkGray;

pub fn draw(frame: &mut Frame, browser: &Browser, search: bool) {
    let area = frame.area();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // title
            Constraint::Min(4),    // body
            Constraint::Length(2), // keys + status
        ])
        .split(area);

    header(frame, rows[0], browser);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(rows[1]);
    list(frame, body[0], browser);
    preview(frame, body[1], browser);

    footer(frame, rows[2], browser, search);
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

fn preview(frame: &mut Frame, area: Rect, browser: &Browser) {
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
        field("cwd", session.cwd.clone().unwrap_or_else(|| "?".into())),
        field(
            "size",
            format!("{} lines, {} KB", session.lines, session.bytes / 1024),
        ),
    ];
    if let Some(title) = &session.title {
        lines.push(field("title", title.clone()));
    }
    lines.push(Line::default());
    lines.push(Line::styled("first message", Style::default().fg(ACCENT)));
    lines.push(Line::raw(
        session
            .first_message
            .clone()
            .unwrap_or_else(|| "(none recorded)".to_string()),
    ));
    lines.push(Line::default());
    lines.push(Line::styled("path", Style::default().fg(ACCENT)));
    lines.push(Line::styled(
        session.path.display().to_string(),
        Style::default().fg(DIM),
    ));

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
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
