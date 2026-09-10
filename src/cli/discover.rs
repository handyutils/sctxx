//! `list`, `find`, `show`, and `expand` (spec §3.5).
//!
//! `expand` is the one an agent uses most after reading an artifact: it turns
//! an `[evt a–b]` pointer back into the masked rows it came from, which is how
//! a compact artifact stays connected to the full transcript.

use super::{GlobalArgs, out, out_json};
use crate::adapters::{self, discovery};
use crate::error::{Error, Result};
use crate::ir::{AgentKind, EventIdx};
use crate::pipeline::mask;
use clap::Args;
use std::path::PathBuf;

/// `sctxx list`
#[derive(Debug, Args)]
pub struct ListArgs {
    /// Only this agent's store: claude, codex, or pi.
    #[arg(long, value_name = "AGENT")]
    agent: Option<String>,

    /// Include sessions from every project, not just this directory.
    #[arg(long)]
    any_project: bool,

    /// Maximum number of sessions to print.
    #[arg(long, default_value_t = 20)]
    limit: usize,
}

/// `sctxx find`
#[derive(Debug, Args)]
pub struct FindArgs {
    /// Text to look for, case-insensitively.
    query: String,

    #[arg(long, value_name = "AGENT")]
    agent: Option<String>,

    #[arg(long)]
    any_project: bool,

    #[arg(long, default_value_t = 20)]
    limit: usize,
}

/// How `show` renders a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum View {
    /// The original JSON lines.
    Raw,
    /// The compact rows an LLM would see.
    Masked,
    /// The canonical intermediate representation.
    Ir,
}

/// `sctxx show`
#[derive(Debug, Args)]
pub struct ShowArgs {
    /// Session reference: `[claude|codex|pi:]<id|prefix|last[:N]>` or a path.
    reference: String,

    #[arg(long, value_enum, default_value_t = View::Masked)]
    view: View,

    /// Event range, e.g. `1203..1240`.
    #[arg(long, value_name = "A..B")]
    range: Option<String>,

    /// Only events on the active branch.
    #[arg(long)]
    active_branch_only: bool,

    #[arg(long)]
    any_project: bool,
}

/// `sctxx expand`
#[derive(Debug, Args)]
pub struct ExpandArgs {
    /// Session reference, or a path to a handoff artifact or `.sctxx/` directory.
    reference: String,

    /// One or more event ranges, e.g. `4122..4381`.
    #[arg(value_name = "A..B", required = true)]
    ranges: Vec<String>,

    /// Extra events to show on each side of a range.
    #[arg(long, default_value_t = 0)]
    context: u32,

    #[arg(long)]
    any_project: bool,
}

fn parse_agent(value: &Option<String>) -> Result<Option<AgentKind>> {
    match value {
        None => Ok(None),
        Some(slug) => AgentKind::from_slug(slug).map(Some).ok_or_else(|| {
            Error::Usage(format!(
                "unknown agent `{slug}` (expected claude, codex, or pi)"
            ))
        }),
    }
}

/// Keep only sessions recorded in the current directory, unless asked not to.
fn filter_project(
    sessions: Vec<discovery::SessionSummary>,
    any_project: bool,
) -> Vec<discovery::SessionSummary> {
    if any_project {
        return sessions;
    }
    let Some(cwd) = std::env::current_dir()
        .ok()
        .map(|cwd| cwd.to_string_lossy().into_owned())
    else {
        return sessions;
    };
    let in_project: Vec<discovery::SessionSummary> = sessions
        .iter()
        .filter(|session| session.cwd.as_deref() == Some(cwd.as_str()))
        .cloned()
        .collect();
    // Falling back to everything beats printing nothing when a store records
    // no cwd.
    if in_project.is_empty() {
        sessions
    } else {
        in_project
    }
}

pub fn list(args: &ListArgs, global: &GlobalArgs) -> Result<i32> {
    let roots = global.roots();
    let sessions = match parse_agent(&args.agent)? {
        Some(agent) => discovery::list(agent, &roots),
        None => discovery::list_all(&roots),
    };
    let sessions = filter_project(sessions, args.any_project);
    let sessions: Vec<_> = sessions.into_iter().take(args.limit).collect();

    if sessions.is_empty() {
        global
            .note("no sessions found. `sctxx doctor` shows which store directories were searched.");
    }
    if global.json {
        return out_json(&sessions).map(|()| 0);
    }
    out(&table(&sessions));
    Ok(0)
}

pub fn find(args: &FindArgs, global: &GlobalArgs) -> Result<i32> {
    let roots = global.roots();
    let needle = args.query.to_lowercase();
    let sessions = match parse_agent(&args.agent)? {
        Some(agent) => discovery::list(agent, &roots),
        None => discovery::list_all(&roots),
    };
    let matched: Vec<discovery::SessionSummary> = filter_project(sessions, args.any_project)
        .into_iter()
        .filter(|session| {
            let haystack = format!(
                "{} {} {}",
                session.title.clone().unwrap_or_default(),
                session.first_message.clone().unwrap_or_default(),
                session.cwd.clone().unwrap_or_default()
            );
            haystack.to_lowercase().contains(&needle)
        })
        .take(args.limit)
        .collect();

    if global.json {
        return out_json(&matched).map(|()| 0);
    }
    if matched.is_empty() {
        global.note(&format!("nothing matched `{}`", args.query));
        return Ok(0);
    }
    out(&table(&matched));
    Ok(0)
}

fn table(sessions: &[discovery::SessionSummary]) -> String {
    if sessions.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    out.push_str("AGENT   ID                                    LINES   STARTED               FIRST MESSAGE\n");
    for session in sessions {
        out.push_str(&format!(
            "{:<7} {:<37} {:>6}   {:<21} {}\n",
            session.agent,
            truncate(&session.id, 37),
            session.lines,
            session.started_at.clone().unwrap_or_default(),
            truncate(
                session
                    .title
                    .as_deref()
                    .or(session.first_message.as_deref())
                    .unwrap_or(""),
                60
            )
        ));
    }
    out
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let kept: String = text.chars().take(max.saturating_sub(1)).collect();
    format!("{kept}\u{2026}")
}

pub fn show(args: &ShowArgs, global: &GlobalArgs) -> Result<i32> {
    let reference = discovery::parse_reference(&args.reference)?;
    let options = global.resolve_options(args.any_project, true);
    let summary = discovery::resolve(&reference, &options)?;
    let agent = AgentKind::from_slug(summary.agent)
        .ok_or_else(|| Error::UnknownFormat(summary.path.clone()))?;
    let source = adapters::source::read(&summary.path)?;

    // `raw` needs the original lines, so keep them before parsing consumes it.
    let raw_lines = source.lines.clone();
    let session = adapters::parse_as(agent, source, adapters::DEFAULT_MAX_BAD_LINE_RATE)?;
    let range = args.range.as_deref().map(parse_range).transpose()?;

    match args.view {
        View::Raw => {
            let mut out_text = String::new();
            for event in selected_events(&session, range, args.active_branch_only) {
                if let Some(line) = raw_lines.get(event.line.line.saturating_sub(1) as usize) {
                    out_text.push_str(line);
                    out_text.push('\n');
                }
            }
            out(&out_text);
        }
        View::Ir => {
            let events: Vec<&crate::ir::Event> =
                selected_events(&session, range, args.active_branch_only).collect();
            if global.json {
                out_json(&serde_json::json!({
                    "agent": session.agent,
                    "id": session.id,
                    "source_hash": session.source_hash,
                    "meta": session.meta,
                    "active": session.active,
                    "diagnostics": session.diagnostics,
                    "events": events,
                }))?;
            } else {
                out_json(&events)?;
            }
        }
        View::Masked => {
            let rows = mask::build(&session, &mask::MaskOptions::default());
            let filtered: Vec<mask::Row> = rows
                .into_iter()
                .filter(|row| range.is_none_or(|(a, b)| (a..=b).contains(&row.evt)))
                .collect();
            out(&mask::render(&filtered));
        }
    }

    if !session.diagnostics.is_empty() {
        global.note(&format!(
            "{} adapter diagnostic(s)",
            session.diagnostics.len()
        ));
    }
    Ok(0)
}

fn selected_events(
    session: &crate::ir::Session,
    range: Option<(EventIdx, EventIdx)>,
    active_only: bool,
) -> impl Iterator<Item = &crate::ir::Event> {
    let active: std::collections::BTreeSet<EventIdx> = session.active.iter().copied().collect();
    session.events.iter().filter(move |event| {
        let in_range = range.is_none_or(|(a, b)| (a..=b).contains(&event.idx));
        let on_branch = !active_only || active.contains(&event.idx);
        in_range && on_branch
    })
}

/// Parse `A..B` or `A..=B` or a single index.
fn parse_range(text: &str) -> Result<(EventIdx, EventIdx)> {
    let cleaned = text.replace("..=", "..");
    let (start, end) = match cleaned.split_once("..") {
        Some((start, end)) => (start.trim(), end.trim()),
        None => (cleaned.trim(), cleaned.trim()),
    };
    let parse = |value: &str, what: &str| -> Result<EventIdx> {
        value
            .parse::<EventIdx>()
            .map_err(|_| Error::Usage(format!("`{value}` is not a valid {what} event index")))
    };
    let start = parse(start, "start")?;
    let end = if end.is_empty() {
        start
    } else {
        parse(end, "end")?
    };
    if end < start {
        return Err(Error::Usage(format!(
            "range {start}..{end} ends before it starts"
        )));
    }
    Ok((start, end))
}

pub fn expand(args: &ExpandArgs, global: &GlobalArgs) -> Result<i32> {
    // An artifact knows which session it came from, so `sctxx expand .sctxx/`
    // works without the user repeating the reference.
    let reference_text = match resolve_artifact_reference(&args.reference) {
        Some(reference) => {
            global.note(&format!("resolved artifact to session {reference}"));
            reference
        }
        None => args.reference.clone(),
    };
    let reference = discovery::parse_reference(&reference_text)?;
    let options = global.resolve_options(args.any_project, true);
    let summary = discovery::resolve(&reference, &options)?;
    let agent = AgentKind::from_slug(summary.agent)
        .ok_or_else(|| Error::UnknownFormat(summary.path.clone()))?;
    let source = adapters::source::read(&summary.path)?;
    let session = adapters::parse_as(agent, source, adapters::DEFAULT_MAX_BAD_LINE_RATE)?;
    let rows = mask::build(&session, &mask::MaskOptions::default());

    let mut out_text = String::new();
    for text in &args.ranges {
        let (start, end) = parse_range(text)?;
        let start = start.saturating_sub(args.context);
        let end = end.saturating_add(args.context);
        let selected: Vec<mask::Row> = rows
            .iter()
            .filter(|row| (start..=end).contains(&row.evt))
            .cloned()
            .collect();
        out_text.push_str(&format!("=== evt {start}–{end} ===\n"));
        if selected.is_empty() {
            out_text.push_str("(no rows in this range; it may fall outside the active branch)\n");
        } else {
            out_text.push_str(&mask::render(&selected));
        }
    }
    out(&out_text);
    Ok(0)
}

/// Read the session reference out of a `handoff.md`, `handoff.json`, or a
/// `.sctxx/` directory.
fn resolve_artifact_reference(candidate: &str) -> Option<String> {
    let path = PathBuf::from(candidate);
    if !path.exists() {
        return None;
    }
    let file = if path.is_dir() {
        let json = path.join("handoff.json");
        if json.is_file() {
            json
        } else {
            path.join("handoff.md")
        }
    } else {
        path
    };
    let body = std::fs::read_to_string(&file).ok()?;

    if file.extension().is_some_and(|ext| ext == "json") {
        let value: serde_json::Value = serde_json::from_str(&body).ok()?;
        let agent = value["session"]["agent"].as_str()?;
        let id = value["session"]["id"].as_str()?;
        return Some(format!("{agent}:{id}"));
    }
    // The markdown front matter carries `source: {agent: ..., session: ...}`.
    let line = body
        .lines()
        .take(30)
        .find(|line| line.starts_with("source:"))?;
    let agent = field(line, "agent:")?;
    let session = field(line, "session:")?;
    Some(format!("{agent}:{session}"))
}

fn field(line: &str, key: &str) -> Option<String> {
    let after = line.split(key).nth(1)?;
    let value = after.trim_start().split([',', '}']).next()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_accept_the_forms_an_artifact_prints() {
        assert_eq!(parse_range("10..20").expect("parse"), (10, 20));
        assert_eq!(parse_range("10..=20").expect("parse"), (10, 20));
        assert_eq!(parse_range(" 7 ").expect("parse"), (7, 7));
        assert_eq!(parse_range("20..10").expect_err("reject").exit_code(), 2);
        assert_eq!(parse_range("a..b").expect_err("reject").exit_code(), 2);
    }

    #[test]
    fn an_artifact_directory_resolves_to_its_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("handoff.md"),
            "---\nschema: sctxx.handoff/v1\nsource: {agent: claude, session: 7c1e8f82, events: 10}\n---\n",
        )
        .expect("write");
        assert_eq!(
            resolve_artifact_reference(&dir.path().to_string_lossy()).as_deref(),
            Some("claude:7c1e8f82")
        );
    }

    #[test]
    fn an_artifact_json_resolves_to_its_session() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            dir.path().join("handoff.json"),
            r#"{"session":{"agent":"codex","id":"abc-123"}}"#,
        )
        .expect("write");
        assert_eq!(
            resolve_artifact_reference(&dir.path().to_string_lossy()).as_deref(),
            Some("codex:abc-123")
        );
    }

    #[test]
    fn a_plain_reference_is_left_alone() {
        assert!(resolve_artifact_reference("claude:abcdef").is_none());
    }

    #[test]
    fn unknown_agent_names_are_usage_errors() {
        assert_eq!(
            parse_agent(&Some("opencode".into()))
                .expect_err("reject")
                .exit_code(),
            2
        );
        assert_eq!(parse_agent(&None).expect("none"), None);
    }
}
