//! Finding sessions on disk and resolving session references (spec §3.2, §6.1).
//!
//! Discovery never parses a whole store: each candidate file is streamed once
//! ([`super::source::scan`]) to recover its id, cwd, title, and first human
//! message. That keeps `list` and `find` fast without a cache to invalidate.

use super::source::{self, FileScan};
use crate::error::{Error, Result};
use crate::ir::AgentKind;
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Where a provider keeps its sessions.
#[derive(Debug, Clone)]
pub struct Store {
    pub agent: AgentKind,
    pub roots: Vec<PathBuf>,
}

/// Overrides for the store roots, from CLI flags.
#[derive(Debug, Clone, Default)]
pub struct Roots {
    pub claude: Option<PathBuf>,
    pub codex: Option<PathBuf>,
    pub pi: Option<PathBuf>,
}

/// One discovered session, without parsing it.
#[derive(Debug, Clone, Serialize)]
pub struct SessionSummary {
    pub agent: &'static str,
    pub id: String,
    pub path: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// Lines in the file, a cheap proxy for size that needs no parsing.
    pub lines: usize,
    pub bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// First human message, truncated for display.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_message: Option<String>,
    /// Modification time as seconds since the Unix epoch, for recency sorting.
    #[serde(skip)]
    pub mtime: u64,
}

impl SessionSummary {
    /// The reference that resolves back to exactly this session.
    pub fn reference(&self) -> String {
        format!("{}:{}", self.agent, self.id)
    }
}

fn home() -> Option<PathBuf> {
    // `HOME` on Unix, `USERPROFILE` on Windows. Avoids a dependency for the
    // one path lookup sctxx needs.
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty())
}

/// The store layout for one agent, honoring env overrides and CLI roots.
pub fn store(agent: AgentKind, roots: &Roots) -> Store {
    let mut store = Store {
        agent,
        roots: Vec::new(),
    };
    match agent {
        AgentKind::ClaudeCode => {
            if let Some(root) = &roots.claude {
                store.roots.push(root.clone());
            } else if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
                store.roots.push(PathBuf::from(dir).join("projects"));
            } else if let Some(home) = home() {
                store.roots.push(home.join(".claude").join("projects"));
            }
        }
        AgentKind::Codex => {
            let bases: Vec<PathBuf> = match &roots.codex {
                Some(root) => vec![root.clone()],
                None => match std::env::var_os("CODEX_HOME") {
                    Some(dir) => vec![PathBuf::from(dir)],
                    None => home()
                        .map(|home| vec![home.join(".codex")])
                        .unwrap_or_default(),
                },
            };
            for base in bases {
                // A root given explicitly may already be the sessions dir.
                if base.file_name().is_some_and(|name| name == "sessions") {
                    store.roots.push(base);
                } else {
                    store.roots.push(base.join("sessions"));
                    store.roots.push(base.join("archived_sessions"));
                }
            }
        }
        AgentKind::Pi => {
            if let Some(root) = &roots.pi {
                store.roots.push(root.clone());
            } else if let Some(home) = home() {
                store
                    .roots
                    .push(home.join(".pi").join("agent").join("sessions"));
            }
        }
    }
    store
}

/// The directory Claude Code writes subagent transcripts into, beside the
/// session file: `<project>/<session-id>/subagents/agent-<slug>.jsonl`.
const CLAUDE_SUBAGENT_DIR: &str = "subagents";

/// True when a path is a subagent transcript rather than a session of its own.
///
/// These files carry the *parent's* `sessionId`, so listing them as sessions
/// would make every reference to a session with subagents ambiguous.
fn is_sidechain_file(path: &Path) -> bool {
    path.parent()
        .and_then(|parent| parent.file_name())
        .is_some_and(|name| name == CLAUDE_SUBAGENT_DIR)
}

/// Subagent transcript files belonging to a session, in a stable order.
///
/// Empty for providers that inline their subagents.
pub fn sidechain_paths(agent: AgentKind, session_path: &Path) -> Vec<PathBuf> {
    if agent != AgentKind::ClaudeCode {
        return Vec::new();
    }
    let Some(stem) = session_path.file_stem() else {
        return Vec::new();
    };
    let Some(parent) = session_path.parent() else {
        return Vec::new();
    };
    let dir = parent.join(stem).join(CLAUDE_SUBAGENT_DIR);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut paths: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path.file_name().is_some_and(|name| {
                    let name = name.to_string_lossy();
                    name.ends_with(".jsonl") || name.ends_with(".jsonl.zst")
                })
        })
        .collect();
    paths.sort();
    paths
}

/// True when a path looks like a session file for `agent`.
fn is_session_file(agent: AgentKind, path: &Path) -> bool {
    let name = match path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
    {
        Some(name) => name,
        None => return false,
    };
    if name.starts_with('.') {
        return false;
    }
    let is_jsonl = name.ends_with(".jsonl") || name.ends_with(".jsonl.zst");
    if !is_jsonl {
        return false;
    }
    if is_sidechain_file(path) {
        return false;
    }
    match agent {
        // Codex names every rollout `rollout-<timestamp>-<uuid>`.
        AgentKind::Codex => name.starts_with("rollout-"),
        _ => true,
    }
}

/// List every session in `agent`'s store, newest first.
pub fn list(agent: AgentKind, roots: &Roots) -> Vec<SessionSummary> {
    let store = store(agent, roots);
    let mut found = Vec::new();
    for root in &store.roots {
        if !root.is_dir() {
            continue;
        }
        for entry in walkdir::WalkDir::new(root)
            .follow_links(false)
            .into_iter()
            .flatten()
        {
            if !entry.file_type().is_file() || !is_session_file(agent, entry.path()) {
                continue;
            }
            if let Some(summary) = summarize(agent, entry.path()) {
                found.push(summary);
            }
        }
    }
    found.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    found
}

/// List every session across every store, newest first.
pub fn list_all(roots: &Roots) -> Vec<SessionSummary> {
    let mut all: Vec<SessionSummary> = AgentKind::ALL
        .iter()
        .flat_map(|agent| list(*agent, roots))
        .collect();
    all.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.path.cmp(&b.path)));
    all
}

/// Summarize one session file without parsing it into the IR.
pub fn summarize(agent: AgentKind, path: &Path) -> Option<SessionSummary> {
    let scan = source::scan(path).ok()?;
    if scan.lines == 0 {
        return None;
    }
    let mtime = std::fs::metadata(path)
        .and_then(|meta| meta.modified())
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_secs())
        .unwrap_or(0);

    let mut summary = SessionSummary {
        agent: agent.slug(),
        id: fallback_id(agent, path),
        path: path.to_path_buf(),
        cwd: None,
        started_at: None,
        ended_at: timestamp_of(&scan.last_line),
        lines: scan.lines,
        bytes: scan.bytes,
        title: None,
        first_message: None,
        mtime,
    };
    enrich(agent, &scan, &mut summary);
    Some(summary)
}

fn fallback_id(agent: AgentKind, path: &Path) -> String {
    match agent {
        AgentKind::Codex => super::codex::id_from_filename(path),
        AgentKind::Pi => super::pi::id_from_filename(path),
        AgentKind::ClaudeCode => path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

fn timestamp_of(line: &str) -> Option<String> {
    let value: Value = serde_json::from_str(line).ok()?;
    super::first_str_field(&value, &["timestamp", "time"])
}

/// Pull id, cwd, title, and first human message out of the scanned head.
fn enrich(agent: AgentKind, scan: &FileScan, summary: &mut SessionSummary) {
    for line in &scan.head {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if summary.started_at.is_none() {
            summary.started_at = super::first_str_field(&value, &["timestamp", "time"]);
        }
        match agent {
            AgentKind::ClaudeCode => {
                if let Some(cwd) = super::string_field(&value, "cwd") {
                    summary.cwd.get_or_insert(cwd);
                }
                if let Some(id) = super::string_field(&value, "sessionId") {
                    summary.id = id;
                }
                if let Some(title) = super::string_field(&value, "summary") {
                    summary.title.get_or_insert(title);
                }
                if super::str_field(&value, "type") == Some("user")
                    && !super::bool_field(&value, "isMeta")
                    && !super::bool_field(&value, "isCompactSummary")
                {
                    let text = value
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .map(super::content_text)
                        .unwrap_or_default();
                    set_first_message(summary, text);
                }
            }
            AgentKind::Codex => {
                let payload = value.get("payload").cloned().unwrap_or(Value::Null);
                if super::str_field(&value, "type") == Some("session_meta") {
                    let meta = payload.get("meta").unwrap_or(&payload);
                    if let Some(id) =
                        super::first_str_field(meta, &["id", "session_id", "conversation_id"])
                    {
                        summary.id = id;
                    }
                    if let Some(cwd) = super::string_field(meta, "cwd") {
                        summary.cwd.get_or_insert(cwd);
                    }
                }
                if super::str_field(&value, "type") == Some("response_item")
                    && super::str_field(&payload, "type") == Some("message")
                    && super::str_field(&payload, "role") == Some("user")
                {
                    let text = super::content_text(payload.get("content").unwrap_or(&Value::Null));
                    if !text.trim_start().starts_with('<') && !text.starts_with('#') {
                        set_first_message(summary, text);
                    }
                }
            }
            AgentKind::Pi => {
                if super::str_field(&value, "type") == Some("session") {
                    if let Some(id) = super::string_field(&value, "id") {
                        summary.id = id;
                    }
                    if let Some(cwd) = super::string_field(&value, "cwd") {
                        summary.cwd.get_or_insert(cwd);
                    }
                }
                if let Some(title) = super::first_str_field(&value, &["title", "label"]) {
                    summary.title.get_or_insert(title);
                }
                if super::str_field(&value, "type") == Some("message")
                    && value
                        .get("message")
                        .and_then(|m| super::str_field(m, "role"))
                        == Some("user")
                {
                    let text = value
                        .get("message")
                        .and_then(|message| message.get("content"))
                        .map(super::content_text)
                        .unwrap_or_default();
                    set_first_message(summary, text);
                }
            }
        }
    }
}

fn set_first_message(summary: &mut SessionSummary, text: String) {
    if summary.first_message.is_some() {
        return;
    }
    let text = text.trim();
    if text.is_empty() {
        return;
    }
    let preview: String = text.chars().take(120).collect();
    let preview = preview.replace('\n', " ");
    summary.first_message = Some(preview);
}

/// What a session reference selected.
#[derive(Debug)]
pub enum Selector {
    Path(PathBuf),
    Id(String),
    /// `last` / `last:N`, 1-based.
    Last(usize),
}

/// A parsed session reference: `[<agent>:]<selector>`.
#[derive(Debug)]
pub struct Reference {
    pub agent: Option<AgentKind>,
    pub selector: Selector,
    pub raw: String,
}

/// Parse a session reference (spec §3.2).
pub fn parse_reference(raw: &str) -> Result<Reference> {
    if raw.trim().is_empty() {
        return Err(Error::Usage("empty session reference".into()));
    }
    // A filesystem path wins, even one that contains a colon.
    let as_path = Path::new(raw);
    if as_path.exists() {
        return Ok(Reference {
            agent: None,
            selector: Selector::Path(as_path.to_path_buf()),
            raw: raw.to_string(),
        });
    }

    let (agent, rest) = match raw.split_once(':') {
        Some((prefix, rest)) => match AgentKind::from_slug(prefix) {
            Some(agent) => (Some(agent), rest),
            // `last:3` has no agent prefix.
            None if prefix == "last" => (None, raw),
            None => {
                return Err(Error::Usage(format!(
                    "unknown agent prefix `{prefix}` (expected claude, codex, or pi)"
                )));
            }
        },
        None => (None, raw),
    };

    let selector = if let Some(n) = rest.strip_prefix("last") {
        let n = n.trim_start_matches(':');
        if n.is_empty() {
            Selector::Last(1)
        } else {
            let n: usize = n
                .parse()
                .map_err(|_| Error::Usage(format!("`last:{n}` needs a positive number")))?;
            Selector::Last(n.max(1))
        }
    } else if rest.len() < 6 {
        return Err(Error::Usage(format!(
            "session id `{rest}` is too short; use at least 6 characters"
        )));
    } else {
        Selector::Id(rest.to_string())
    };

    Ok(Reference {
        agent,
        selector,
        raw: raw.to_string(),
    })
}

/// Options that affect which sessions a reference may select.
#[derive(Debug, Clone, Default)]
pub struct ResolveOptions {
    pub roots: Roots,
    /// Drop the "session cwd equals the current directory" filter for `last`.
    pub any_project: bool,
    /// Also accept sessions recorded in an ancestor of the current directory.
    pub up: bool,
    pub cwd: Option<PathBuf>,
}

/// Resolve a reference to exactly one session file (spec §3.2).
pub fn resolve(reference: &Reference, options: &ResolveOptions) -> Result<SessionSummary> {
    if let Selector::Path(path) = &reference.selector {
        let scan = source::scan(path)?;
        let agent =
            super::detect(&scan.first_line).ok_or_else(|| Error::UnknownFormat(path.clone()))?;
        return summarize(agent, path).ok_or_else(|| Error::SessionNotFound(reference.raw.clone()));
    }

    let candidates: Vec<SessionSummary> = match reference.agent {
        Some(agent) => list(agent, &options.roots),
        None => list_all(&options.roots),
    };
    if candidates.is_empty() {
        return Err(Error::SessionNotFound(format!(
            "{} (no sessions found; run `sctxx doctor` to check store locations)",
            reference.raw
        )));
    }

    let matches: Vec<SessionSummary> = match &reference.selector {
        // Handled above; returning empty keeps this arm total without a panic.
        Selector::Path(_) => Vec::new(),
        Selector::Id(id) => candidates
            .into_iter()
            .filter(|candidate| {
                candidate.id == *id
                    || candidate.id.starts_with(id)
                    || candidate
                        .path
                        .file_stem()
                        .is_some_and(|stem| stem.to_string_lossy().contains(id))
            })
            .collect(),
        Selector::Last(n) => {
            let filtered: Vec<SessionSummary> = if options.any_project {
                candidates
            } else {
                let cwd = options
                    .cwd
                    .clone()
                    .or_else(|| std::env::current_dir().ok())
                    .unwrap_or_default();
                let in_project: Vec<SessionSummary> = candidates
                    .iter()
                    .filter(|candidate| matches_project(candidate, &cwd, options.up))
                    .cloned()
                    .collect();
                // A store with no cwd information must not make `last` useless.
                if in_project.is_empty() {
                    candidates
                } else {
                    in_project
                }
            };
            return filtered.into_iter().nth(n - 1).ok_or_else(|| {
                Error::SessionNotFound(format!("{} (fewer than {n} sessions)", reference.raw))
            });
        }
    };

    match matches.len() {
        0 => Err(Error::SessionNotFound(reference.raw.clone())),
        1 => matches
            .into_iter()
            .next()
            .ok_or_else(|| Error::SessionNotFound(reference.raw.clone())),
        count => {
            let candidates_json =
                serde_json::to_string_pretty(&matches).unwrap_or_else(|_| "[]".to_string());
            Err(Error::Ambiguous {
                reference: reference.raw.clone(),
                count,
                candidates_json,
            })
        }
    }
}

fn matches_project(candidate: &SessionSummary, cwd: &Path, up: bool) -> bool {
    let Some(session_cwd) = &candidate.cwd else {
        return false;
    };
    let session_cwd = Path::new(session_cwd);
    if session_cwd == cwd {
        return true;
    }
    up && cwd.ancestors().any(|ancestor| ancestor == session_cwd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_parse_agent_prefixes() {
        let reference = parse_reference("claude:7c1e8f82f8238f").expect("parse");
        assert_eq!(reference.agent, Some(AgentKind::ClaudeCode));
        assert!(matches!(reference.selector, Selector::Id(id) if id == "7c1e8f82f8238f"));
    }

    #[test]
    fn last_and_last_n_are_recognized_with_and_without_a_prefix() {
        assert!(matches!(
            parse_reference("last").expect("parse").selector,
            Selector::Last(1)
        ));
        assert!(matches!(
            parse_reference("last:3").expect("parse").selector,
            Selector::Last(3)
        ));
        let reference = parse_reference("codex:last").expect("parse");
        assert_eq!(reference.agent, Some(AgentKind::Codex));
        assert!(matches!(reference.selector, Selector::Last(1)));
    }

    #[test]
    fn a_short_id_is_a_usage_error_not_a_wide_match() {
        let error = parse_reference("abc").expect_err("should reject");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn an_unknown_agent_prefix_is_a_usage_error() {
        let error = parse_reference("opencode:abcdef").expect_err("should reject");
        assert_eq!(error.exit_code(), 2);
    }

    #[test]
    fn store_roots_honor_explicit_overrides() {
        let roots = Roots {
            codex: Some(PathBuf::from("/tmp/cx")),
            ..Roots::default()
        };
        let store = store(AgentKind::Codex, &roots);
        assert!(store.roots.contains(&PathBuf::from("/tmp/cx/sessions")));
        assert!(
            store
                .roots
                .contains(&PathBuf::from("/tmp/cx/archived_sessions"))
        );
    }

    #[test]
    fn codex_only_accepts_rollout_files() {
        assert!(is_session_file(
            AgentKind::Codex,
            Path::new("rollout-a-b.jsonl")
        ));
        assert!(!is_session_file(AgentKind::Codex, Path::new("notes.jsonl")));
        assert!(is_session_file(
            AgentKind::ClaudeCode,
            Path::new("uuid.jsonl")
        ));
        assert!(!is_session_file(
            AgentKind::ClaudeCode,
            Path::new("uuid.json")
        ));
    }

    #[test]
    fn a_subagent_transcript_is_not_listed_as_its_own_session() {
        // Observed on Claude Code 2.x: these carry the parent's sessionId, so
        // listing them would make every such reference ambiguous.
        assert!(is_session_file(
            AgentKind::ClaudeCode,
            Path::new("/p/-proj/7c1e.jsonl")
        ));
        let sidechain = Path::new("/p/-proj/7c1e/subagents/agent-survey-abc.jsonl");
        assert!(is_sidechain_file(sidechain));
        assert!(!is_session_file(AgentKind::ClaudeCode, sidechain));
    }

    #[test]
    fn a_sessions_subagent_files_are_found_beside_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session = dir.path().join("7c1e.jsonl");
        std::fs::write(&session, "{}").expect("write");
        let subagents = dir.path().join("7c1e/subagents");
        std::fs::create_dir_all(&subagents).expect("mkdir");
        std::fs::write(subagents.join("agent-b.jsonl"), "{}").expect("write");
        std::fs::write(subagents.join("agent-a.jsonl"), "{}").expect("write");
        // The sidecar metadata file must not be mistaken for a transcript.
        std::fs::write(subagents.join("agent-a.meta.json"), "{}").expect("write");

        let found = sidechain_paths(AgentKind::ClaudeCode, &session);
        assert_eq!(found.len(), 2, "{found:?}");
        assert!(found[0].ends_with("agent-a.jsonl"), "{found:?}");
        assert!(sidechain_paths(AgentKind::Codex, &session).is_empty());
    }
}
