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
use super::canvas::Expansion;
use super::form::Form;
use super::preview::LedgerPreview;
use super::{App, HandoffState, Mode, PreviewState, RunState};
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

    // Reading takes the whole body: prose in half a terminal is not reading.
    if app.mode == Mode::Canvas && app.canvas.is_some() {
        canvas_pane(frame, rows[1], app);
    } else {
        let body = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
            .split(rows[1]);
        list(frame, body[0], &app.browser);
        // The handoff and the canvas take the pane when they are open: each is
        // the last step of its flow and needs the room.
        match (&app.handoff, &app.form, app.mode) {
            (Some(state), _, _) => handoff_pane(frame, body[1], app, state),
            (None, Some(form), _) => extract_pane(frame, body[1], form, &app.run),
            (None, None, Mode::OpenArtifact) => open_artifact_pane(frame, body[1], app),
            (None, None, _) => preview(frame, body[1], &app.browser, app.preview_state()),
        }
    }

    footer(frame, rows[2], &app.browser, app.mode);
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

/// The extraction form, and whatever the last run did.
///
/// Every field here was read from clap by `form.rs`: the flag, the value, and
/// whether it is a toggle, a choice, or text. Nothing in this function decides
/// what a field *is*, which is what keeps the form and the CLI the same thing.
fn extract_pane(frame: &mut Frame, area: Rect, form: &Form, run: &RunState) {
    let block = Block::default().borders(Borders::ALL).title(" extract ");
    let inner = area.height.saturating_sub(2) as usize;

    // The run state goes first so a result can never be scrolled off the bottom,
    // and the focused field's help goes last because it is the least costly thing
    // to lose on a short terminal.
    let mut lines = run_lines(run);
    lines.push(Line::default());
    let list_height = inner.saturating_sub(lines.len() + 2).max(1);
    let first = form.scroll_for(list_height);

    for (index, field) in form
        .fields()
        .iter()
        .enumerate()
        .skip(first)
        .take(list_height)
    {
        let focused = index == form.focus();
        let text = format!(
            "{} {:<20} {}",
            if focused { "▌" } else { " " },
            field.flag,
            field.shown()
        );
        let style = if focused {
            Style::default()
                .bg(ACCENT)
                .fg(Color::Black)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::styled(truncate(&text, 240), style));
    }

    lines.push(Line::default());
    if let Some(field) = form.focused() {
        lines.push(Line::styled(
            format!(" {}", truncate(&field.help, 300)),
            Style::default().fg(DIM),
        ));
    }

    frame.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// The handoff: choose an agent, then confirm exactly what will run.
///
/// The two steps are the requirement, not a flourish. Extraction never starts
/// another agent as a side effect, and the command is on screen before the
/// terminal is handed over (FR-021, FR-021b).
fn handoff_pane(frame: &mut Frame, area: Rect, app: &App, state: &HandoffState) {
    let block = Block::default().borders(Borders::ALL).title(" handoff ");
    let width = area.width.saturating_sub(2) as usize;
    let mut lines = Vec::new();

    if let RunState::Done(outcome) = &app.run {
        lines.extend(wrapped_field(
            "handoff",
            &outcome.handoff.display().to_string(),
            width,
        ));
    }
    if let Some(note) = &app.handoff_note {
        lines.push(Line::styled(format!(" {note}"), Style::default().fg(WARN)));
    }
    lines.push(Line::default());

    match state {
        HandoffState::Choosing => {
            lines.push(Line::styled(
                " continue this in a new session of…",
                Style::default().fg(ACCENT),
            ));
            match &app.agents {
                None => lines.push(Line::styled(
                    " looking for the agents installed here…",
                    Style::default().fg(DIM),
                )),
                Some(agents) if agents.iter().all(|agent| !agent.installed()) => {
                    // Never hidden silently: say what is missing and what to do.
                    lines.push(Line::styled(
                        " none of claude, codex, or pi is on PATH",
                        Style::default().fg(WARN),
                    ));
                    lines.push(Line::styled(
                        " install one, or read the handoff yourself",
                        Style::default().fg(DIM),
                    ));
                }
                Some(agents) => {
                    for (index, agent) in agents.iter().enumerate() {
                        let focused = index == app.handoff_cursor;
                        let marker = if focused { "▌" } else { " " };
                        let text = format!(
                            "{marker} {:<12} {}",
                            agent.label,
                            truncate(&agent.status(), 120)
                        );
                        let style = if focused {
                            Style::default()
                                .bg(ACCENT)
                                .fg(Color::Black)
                                .add_modifier(Modifier::BOLD)
                        } else if agent.installed() {
                            Style::default()
                        } else {
                            Style::default().fg(DIM)
                        };
                        lines.push(Line::styled(truncate(&text, 240), style));
                    }
                }
            }
        }
        HandoffState::Confirming(launch) => {
            lines.push(Line::styled(
                " this exact command will run",
                Style::default().fg(ACCENT),
            ));
            lines.push(field("agent", launch.agent.to_string()));
            lines.extend(wrapped_field("route", launch.route.label(), width));
            lines.extend(wrapped_field(
                "cwd",
                &launch.cwd.display().to_string(),
                width,
            ));
            if launch.route.is_fallback() {
                lines.push(Line::styled(
                    " the detected version is not one the seeding channel was verified on,",
                    Style::default().fg(WARN),
                ));
                lines.push(Line::styled(
                    " so the agent gets the pointer and the directory, and no flags.",
                    Style::default().fg(WARN),
                ));
            }
            lines.push(Line::default());
            // The exact command, whole. Broken across lines by us rather than
            // left to a wrapper that might quietly drop the end of a path.
            for chunk in wrap_hard(&launch.display, width.saturating_sub(1)) {
                lines.push(Line::raw(format!(" {chunk}")));
            }
            lines.push(Line::default());
            // Measured, not assumed: both Claude Code and Codex stop at their own
            // trust prompt for a directory they have not seen before, before the
            // first turn. That prompt is theirs and sctxx must not bypass it, so
            // the least it can do is not let it be a surprise (T2419).
            lines.push(Line::styled(
                " a directory the agent has not seen before will ask to be trusted first.",
                Style::default().fg(DIM),
            ));
            lines.push(Line::styled(
                " that prompt belongs to the agent: sctxx never bypasses it.",
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

/// The artifact, readable in place.
///
/// The cursor is the top visible line — that is the line `enter` follows a
/// pointer from — so it is drawn highlighted rather than left as a rule the
/// reader has to guess.
fn canvas_pane(frame: &mut Frame, area: Rect, app: &App) {
    let Some(canvas) = app.canvas.as_ref() else {
        return;
    };
    let title = match canvas.expansion() {
        Some(expansion) => format!(" artifact · {} ", expansion.label()),
        None => format!(" artifact · {} ", canvas.path().display()),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(truncate(&title, 240));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(inner);
    let height = rows[0].height as usize;

    let mut lines: Vec<Line> = Vec::new();
    if let Some(Expansion::Pending { label }) = canvas.expansion() {
        lines.push(Line::styled(
            format!(" reading {label}…"),
            Style::default().fg(ACCENT),
        ));
    }
    let visible = height.saturating_sub(lines.len());
    let cursor_row = canvas.cursor_row(visible);
    for (offset, text) in canvas.window(visible).iter().enumerate() {
        let style = if Some(offset) == cursor_row {
            Style::default().bg(ACCENT).fg(Color::Black)
        } else {
            Style::default()
        };
        lines.push(Line::styled(text.clone(), style));
    }
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), rows[0]);

    // Where the reader is: which layer, and how far into it.
    let mut tabs: Vec<Span> = Vec::new();
    for (index, layer) in canvas.layers().iter().enumerate() {
        let style = if index == canvas.current() {
            Style::default().bg(ACCENT).fg(Color::Black)
        } else {
            Style::default().fg(DIM)
        };
        tabs.push(Span::styled(format!(" {} ", layer.name), style));
    }
    tabs.push(Span::styled(
        match canvas.expansion() {
            Some(_) => "  esc closes this range".to_string(),
            None => format!(
                "   enter follows a pointer on the highlighted line · {} lines",
                canvas.max_scroll(height) + height
            ),
        },
        Style::default().fg(DIM),
    ));
    frame.render_widget(Paragraph::new(Line::from(tabs)), rows[1]);
}

/// The prompt for opening an artifact that already exists (FR-016a).
fn open_artifact_pane(frame: &mut Frame, area: Rect, app: &App) {
    let lines = vec![
        Line::styled(" open a handoff artifact", Style::default().fg(ACCENT)),
        Line::default(),
        Line::raw(format!(" {}", app.open_path)),
        Line::default(),
        Line::styled(
            " a .sctxx/ directory, or a handoff.md / handoff.json file.",
            Style::default().fg(DIM),
        ),
        Line::styled(
            " this is how a previous run, or a colleague's artifact, is read.",
            Style::default().fg(DIM),
        ),
    ];
    frame.render_widget(
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" read an artifact "),
            )
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// What the extraction pane says about the run.
fn run_lines(run: &RunState) -> Vec<Line<'static>> {
    match run {
        RunState::Idle => vec![Line::styled(
            " ready · enter runs the extraction",
            Style::default().fg(DIM),
        )],
        RunState::Running {
            stage,
            message,
            lines,
        } => {
            let mut out = vec![Line::styled(
                format!(" running · {stage}"),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )];
            if !message.is_empty() {
                out.push(Line::styled(
                    format!(" {message}"),
                    Style::default().fg(DIM),
                ));
            }
            // The last few stages, so the pane shows movement rather than one
            // word that looks stuck (FR-013).
            for line in lines.iter().rev().take(3).rev() {
                out.push(Line::styled(format!(" {line}"), Style::default().fg(DIM)));
            }
            out
        }
        RunState::Done(outcome) => {
            let mut out = vec![Line::styled(
                format!(
                    " done · {} · {} of {} events live",
                    outcome.reference, outcome.live_events, outcome.total_events
                ),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )];
            for path in &outcome.paths {
                out.push(Line::styled(
                    format!(" wrote {}", path.display()),
                    Style::default().fg(DIM),
                ));
            }
            out.push(Line::raw(format!(
                " handoff  {}",
                outcome.handoff.display()
            )));
            for warning in &outcome.warnings {
                out.push(Line::styled(
                    format!(" warning: {warning}"),
                    Style::default().fg(WARN),
                ));
            }
            if let Some(warning) = &outcome.git_warning {
                // The CLI's own sentence, produced by the CLI's own function, so
                // the two cannot drift (FR-015).
                out.push(Line::styled(
                    format!(" {warning}"),
                    Style::default().fg(WARN),
                ));
            }
            out
        }
        RunState::Failed(reason) => vec![
            Line::styled(
                " failed",
                Style::default().fg(WARN).add_modifier(Modifier::BOLD),
            ),
            Line::styled(format!(" {reason}"), Style::default().fg(WARN)),
        ],
    }
}

fn footer(frame: &mut Frame, area: Rect, browser: &Browser, mode: Mode) {
    let keys = match mode {
        Mode::Search => " type to filter · enter keep · esc clear ",
        Mode::Form => " ↑/↓ field · space toggle · ←/→ choose · enter run · esc back ",
        Mode::Handoff => " ↑/↓ agent · enter choose · y run · esc back ",
        Mode::Canvas => " 1-4 layer · tab next · j/k scroll · enter follow pointer · esc back ",
        Mode::OpenArtifact => " type a path to a handoff artifact · enter open · esc cancel ",
        Mode::Browse => {
            " j/k move · / search · a agent · r date · p project · e extract · h handoff · q quit "
        }
    };
    let lines = vec![
        Line::styled(
            keys,
            Style::default().fg(if mode == Mode::Browse { DIM } else { ACCENT }),
        ),
        Line::from(vec![
            Span::styled(" filters: ", Style::default().fg(DIM)),
            Span::raw(browser.filter_summary()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn field(name: &str, value: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!(" {name:<8}"), Style::default().fg(DIM)),
        Span::raw(value),
    ])
}

/// A field whose value is long enough to need breaking, wrapped to `width`.
fn wrapped_field(name: &str, value: &str, width: usize) -> Vec<Line<'static>> {
    wrap_hard(value, width.saturating_sub(9))
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            if index == 0 {
                field(name, chunk)
            } else {
                sub(chunk)
            }
        })
        .collect()
}

/// Wrap text to `width`, breaking a token longer than the width.
///
/// The pane must not rely on the renderer's wrapper to show an *exact* command:
/// a filesystem path contains no spaces, and a command silently shortened at the
/// edge of the pane would misrepresent what is about to run (FR-021).
fn wrap_hard(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![text.to_string()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split(' ').flat_map(|word| hard_split(word, width)) {
        if line.is_empty() {
            line = word;
        } else if line.chars().count() + 1 + word.chars().count() <= width {
            line.push(' ');
            line.push_str(&word);
        } else {
            lines.push(std::mem::take(&mut line));
            line = word;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// Break one token across lines when it cannot fit on one.
fn hard_split(word: &str, width: usize) -> Vec<String> {
    if width == 0 || word.chars().count() <= width {
        return vec![word.to_string()];
    }
    word.chars()
        .collect::<Vec<char>>()
        .chunks(width)
        .map(|chunk| chunk.iter().collect())
        .collect()
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
    use crate::agents::Agent;
    use crate::cli::GlobalArgs;
    use crate::tui::PreviewState;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
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
        App::new(vec![summary()], 100, None, GlobalArgs::default())
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

    /// The screen with its line breaks removed, for asserting on a value the
    /// pane wraps at a space.
    fn unwrapped(text: &str) -> String {
        text.chars()
            .filter(|character| *character != '\n')
            .collect()
    }

    /// The screen with everything that is not part of a path or a flag removed,
    /// for asserting on a value the pane deliberately breaks mid-token.
    ///
    /// Borders and padding would otherwise land in the middle of a broken path
    /// and split the very thing under test.
    fn squashed(text: &str) -> String {
        text.chars()
            .filter(|character| character.is_ascii_alphanumeric() || "/._-'".contains(*character))
            .collect()
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
    fn the_form_lists_the_cli_flags_and_shows_the_focused_help() {
        let mut app = app();
        app.open_form();
        let text = screen(&app, 150, 44);

        assert!(text.contains("extract"), "{text}");
        for flag in ["--mode", "--llm", "--budget", "--out", "--max-bad-lines"] {
            assert!(text.contains(flag), "missing {flag} in:\n{text}");
        }
        // The first field is focused, so its help is on screen.
        assert!(text.contains("pipeline to run"), "{text}");
        // A choice field shows the value clap defaulted it to.
        assert!(text.contains("standard"), "{text}");
    }

    #[test]
    fn a_finished_run_says_where_the_artifact_went() {
        let mut app = app();
        app.open_form();
        app.run = RunState::Done(Box::new(crate::tui::work::Outcome {
            reference: "claude:1367d688".into(),
            handoff: PathBuf::from("/code/.sctxx/handoff.md"),
            paths: vec![PathBuf::from("/code/.sctxx/handoff.md")],
            warnings: vec!["2 events had no timestamp".into()],
            git_warning: Some("warning: /code/.sctxx is not ignored by git".into()),
            live_events: 6_753,
            total_events: 14_151,
        }));
        let text = screen(&app, 150, 44);
        assert!(text.contains("done"), "{text}");
        assert!(text.contains("claude:1367d688"), "{text}");
        assert!(text.contains("6753 of 14151"), "{text}");
        assert!(text.contains("/code/.sctxx/handoff.md"), "{text}");
        assert!(text.contains("no timestamp"), "{text}");
        assert!(
            text.contains("not ignored by git"),
            "the CLI's own warning must reach the pane (FR-015):\n{text}"
        );
    }

    #[test]
    fn a_failed_run_shows_the_reason_and_leaves_the_form_up() {
        let mut app = app();
        app.open_form();
        app.run = RunState::Failed("unknown --redact `sideways`".into());
        let text = screen(&app, 150, 44);
        assert!(text.contains("failed"), "{text}");
        assert!(text.contains("sideways"), "{text}");
        assert!(
            text.contains("--mode"),
            "the form must still be there to fix:\n{text}"
        );
    }

    #[test]
    fn progress_is_visible_while_a_run_is_going() {
        let mut app = app();
        app.open_form();
        app.run = RunState::Running {
            stage: "fold".into(),
            message: "chunk 3 of 9".into(),
            lines: vec![
                "[parse] 900 events".into(),
                "[ledgers] 12 files".into(),
                "[fold] chunk 3 of 9".into(),
            ],
        };
        let text = screen(&app, 150, 44);
        assert!(text.contains("running"), "{text}");
        assert!(text.contains("fold"), "{text}");
        assert!(text.contains("chunk 3 of 9"), "{text}");
    }

    /// An app that has extracted and is looking for an agent to hand off to.
    fn handoff_app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().expect("tempdir");
        let handoff = dir.path().join("handoff.md");
        std::fs::write(&handoff, "# handoff\n").expect("write");

        let mut app = app();
        app.run = RunState::Done(Box::new(crate::tui::work::Outcome {
            reference: "claude:1367d688".into(),
            handoff: handoff.clone(),
            paths: vec![handoff],
            warnings: Vec::new(),
            git_warning: None,
            live_events: 1,
            total_events: 2,
        }));
        app.agents = Some(vec![
            Agent {
                id: "claude",
                label: "Claude Code",
                program: Some(PathBuf::from("/usr/local/bin/claude")),
                version: Some("2.1.268".into()),
                store: PathBuf::from("/tmp/store"),
                store_exists: false,
                verified_against: "2.1.268",
            },
            Agent {
                id: "codex",
                label: "Codex CLI",
                program: None,
                version: None,
                store: PathBuf::from("/tmp/store"),
                store_exists: false,
                verified_against: "0.153.4",
            },
        ]);
        app.handoff = Some(HandoffState::Choosing);
        app.mode = Mode::Handoff;
        (dir, app)
    }

    #[test]
    fn a_long_token_is_broken_rather_than_lost() {
        // The bug this exists for: a path has no spaces, and the pane used to
        // keep the first line of it and drop the rest.
        let path = "/var/folders/4y/d6rxwlnj3jj0t_6xtjbkwt1w0000gn/T/.tmp70R9wP/handoff.md";
        let wrapped = wrap_hard(path, 30);
        assert!(
            wrapped.len() > 1,
            "a 70-character path must break at width 30"
        );
        assert_eq!(
            wrapped.concat(),
            path,
            "breaking must not lose or reorder a character"
        );
        assert!(wrapped.iter().all(|line| line.chars().count() <= 30));
    }

    #[test]
    fn wrapping_keeps_words_whole_when_it_can() {
        let wrapped = wrap_hard("read the handoff at /tmp/handoff.md", 20);
        assert_eq!(wrapped[0], "read the handoff at");
        assert_eq!(
            wrapped.concat().replace(" ", ""),
            "readthehandoffat/tmp/handoff.md".replace(" ", "")
        );
        assert!(wrapped.iter().all(|line| line.chars().count() <= 20));
        // A degenerate width still returns the text rather than nothing.
        assert_eq!(wrap_hard("hello", 0), vec!["hello".to_string()]);
        assert_eq!(hard_split("abc", 0), vec!["abc".to_string()]);
    }

    #[test]
    fn the_picker_shows_every_agent_and_why_it_can_or_cannot_be_used() {
        let (_dir, app) = handoff_app();
        let text = screen(&app, 150, 44);
        assert!(text.contains("handoff"), "{text}");
        assert!(text.contains("Claude Code"), "{text}");
        assert!(text.contains("seeding verified on this version"), "{text}");
        // An agent that is not installed is shown, with the reason — never
        // hidden silently (FR-017).
        assert!(text.contains("Codex CLI"), "{text}");
        assert!(text.contains("not installed"), "{text}");
        assert!(
            squashed(&text).contains("handoff.md"),
            "the artifact is named in full:\n{text}"
        );
    }

    #[test]
    fn the_confirmation_shows_the_exact_command_before_it_runs() {
        let (_dir, mut app) = handoff_app();
        let launch = app.build_launch(0).expect("a launch");
        app.handoff = Some(HandoffState::Confirming(Box::new(launch)));

        let text = screen(&app, 150, 44);
        let flat = squashed(&text);
        assert!(text.contains("this exact command will run"), "{text}");
        assert!(flat.contains("--append-system-prompt-file"), "{text}");
        assert!(flat.contains("/usr/local/bin/claude"), "{text}");
        assert!(flat.contains("systempromptfromtheartifact'spath"), "{text}");
        assert!(flat.contains("Readthehandoffat"), "{text}");
        // The whole path, not a prefix of it.
        assert!(
            flat.contains("handoff.md"),
            "the command must be complete:\n{text}"
        );
        // An agent whose version was verified is not warned about.
        assert!(!text.contains("no flags"), "{text}");
    }

    #[test]
    fn an_unverified_version_says_no_flags_will_be_used() {
        let (_dir, mut app) = handoff_app();
        // The same agent, detected at a version ADR 0004 never checked.
        if let Some(agents) = app.agents.as_mut()
            && let Some(claude) = agents.first_mut()
        {
            claude.version = Some("9.9.9".into());
        }
        let launch = app.build_launch(0).expect("a launch");
        assert!(launch.route.is_fallback());
        app.handoff = Some(HandoffState::Confirming(Box::new(launch)));

        let text = screen(&app, 150, 44);
        let flat = unwrapped(&text);
        assert!(flat.contains("not one the seeding channel"), "{text}");
        assert!(text.contains("no flags"), "{text}");
        // And the flag really is absent from the command.
        assert!(!flat.contains("--append-system-prompt-file"), "{text}");
    }

    #[test]
    fn a_handoff_with_no_agent_installed_says_what_to_do() {
        let (_dir, mut app) = handoff_app();
        if let Some(agents) = app.agents.as_mut() {
            for agent in agents.iter_mut() {
                agent.program = None;
                agent.version = None;
            }
        }
        let text = screen(&app, 150, 44);
        assert!(
            text.contains("none of claude, codex, or pi is on PATH"),
            "{text}"
        );
        assert!(text.contains("install one"), "{text}");
    }

    #[test]
    fn the_browse_footer_advertises_the_keys_that_matter() {
        // A feature nobody can find is not a feature: `e` and `h` are the two
        // that carry the whole flow.
        let text = screen(&app(), 150, 44);
        for key in ["/ search", "e extract", "h handoff", "q quit"] {
            assert!(
                text.contains(key),
                "missing {key:?} from the footer:\n{text}"
            );
        }
    }

    /// An app with an artifact open for reading.
    fn canvas_app() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("handoff.md");
        std::fs::write(
            &path,
            "# Handoff\n\nsource: {agent: claude, session: aaa}\n\n## L0 · Brief\n\nthe goal [evt 41]\nnext step\n\n## L1 · Items\n\n- a file [evt 12–14]\n\n## L3 · Retrieval\n\nsctxx expand .sctxx/ 41\n",
        )
        .expect("write");
        let mut app = app();
        app.canvas = Some(super::super::canvas::Canvas::load(&path).expect("load"));
        app.mode = Mode::Canvas;
        app.body_height = 24;
        (dir, app)
    }

    #[test]
    fn the_canvas_shows_the_artifact_and_the_layer_tabs() {
        let (_dir, app) = canvas_app();
        let text = screen(&app, 150, 40);
        assert!(text.contains("artifact"), "{text}");
        assert!(
            text.contains("the goal [evt 41]"),
            "L0 is on screen:\n{text}"
        );
        assert!(text.contains("handoff.md"), "and which file:\n{text}");
        // The tabs, so the reader knows there are three layers and which is current.
        for layer in ["L0", "L1", "L3"] {
            assert!(text.contains(layer), "missing {layer}:\n{text}");
        }
        // The footer explains the keys rather than leaving them to be guessed.
        assert!(text.contains("1-4 layer"), "{text}");
        assert!(text.contains("enter follow pointer"), "{text}");
    }

    #[test]
    fn switching_layers_changes_what_is_read() {
        let (_dir, mut app) = canvas_app();
        app.handle(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE));
        let text = screen(&app, 150, 40);
        assert!(
            text.contains("a file [evt 12–14]"),
            "L1 is on screen:\n{text}"
        );
        assert!(
            !text.contains("the goal [evt 41]"),
            "and L0 is not:\n{text}"
        );
    }

    #[test]
    fn a_range_being_read_says_so_rather_than_showing_a_stale_layer() {
        let (_dir, mut app) = canvas_app();
        app.handle(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
        app.handle(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        let text = screen(&app, 150, 40);
        assert!(text.contains("reading evt 41"), "{text}");
        assert!(text.contains("esc closes this range"), "{text}");
    }

    #[test]
    fn the_open_artifact_prompt_asks_for_a_path() {
        let mut app = app();
        app.mode = Mode::OpenArtifact;
        app.open_path = "/tmp/somewhere/.sctxx".to_string();
        let text = screen(&app, 150, 40);
        assert!(text.contains("open a handoff artifact"), "{text}");
        assert!(text.contains("/tmp/somewhere/.sctxx"), "{text}");
        assert!(
            text.contains("a previous run, or a colleague's artifact"),
            "{text}"
        );
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
