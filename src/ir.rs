//! Canonical intermediate representation (`docs/SCTXX-SPEC.md` §5).
//!
//! The IR is the contract between adapters and the pipeline. Adapters must be
//! lossless enough that `sctxx show --view ir` can answer "what happened", and
//! must never drop a line silently: an unrecognized line becomes
//! [`EventKind::Unknown`] with its raw JSON retained, plus a [`Diagnostic`].

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Position of an event in the source stream.
pub type EventIdx = u32;
/// 0 is the main conversation; higher ids are subagent sidechains.
pub type StreamId = u32;

/// Which coding agent wrote the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum AgentKind {
    ClaudeCode,
    Codex,
    Pi,
}

impl AgentKind {
    /// The short name used in session references (`claude:<id>`).
    pub fn slug(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Pi => "pi",
        }
    }

    /// Human-facing name.
    pub fn label(self) -> &'static str {
        match self {
            AgentKind::ClaudeCode => "Claude Code",
            AgentKind::Codex => "Codex CLI",
            AgentKind::Pi => "Pi",
        }
    }

    /// Parse the reference prefix. `claude-code` is accepted as an alias.
    pub fn from_slug(slug: &str) -> Option<Self> {
        match slug {
            "claude" | "claude-code" | "claudecode" => Some(AgentKind::ClaudeCode),
            "codex" => Some(AgentKind::Codex),
            "pi" => Some(AgentKind::Pi),
            _ => None,
        }
    }

    /// Every agent sctxx can read, in reference-resolution order.
    pub const ALL: [AgentKind; 3] = [AgentKind::ClaudeCode, AgentKind::Codex, AgentKind::Pi];
}

/// Whether an assistant message was the turn's answer or intermediate chatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Phase {
    Final,
    Commentary,
}

/// Coarse classification of a tool, used by the ledgers and the mask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum ToolClass {
    Edit,
    Read,
    Search,
    Shell,
    Plan,
    Subagent,
    Web,
    Ask,
    Mcp,
    Other,
}

/// One item of a plan/todo list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanItem {
    pub text: String,
    /// Provider status verbatim (`pending`, `in_progress`, `completed`, …).
    pub status: String,
}

/// Where an event came from in the raw files, for `show --view raw`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRef {
    /// Index into [`Session::source_paths`].
    pub path: u16,
    /// 1-based line number.
    pub line: u32,
}

/// A normalized session event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub idx: EventIdx,
    /// Provider id (uuid, entry id, call id) when the format has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_id: Option<String>,
    /// Provider parent pointer, for tree-structured formats.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// RFC 3339 timestamp as recorded by the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ts: Option<String>,
    pub stream: StreamId,
    pub kind: EventKind,
    pub line: LineRef,
}

/// What happened at an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[non_exhaustive]
pub enum EventKind {
    /// A message in the user role. `is_meta` marks harness-injected text that
    /// no human typed, which must never be treated as a user instruction.
    UserMessage {
        text: String,
        is_meta: bool,
    },
    AssistantText {
        text: String,
        phase: Phase,
    },
    /// Model reasoning. `text` is `None` when the provider stored it encrypted.
    Reasoning {
        text: Option<String>,
        redacted: bool,
    },
    ToolCall {
        call_id: String,
        name: String,
        class: ToolClass,
        args: serde_json::Value,
    },
    ToolResult {
        call_id: String,
        output: String,
        is_error: Option<bool>,
        exit_code: Option<i32>,
    },
    /// A shell command recorded outside the tool-call protocol (Pi).
    ShellExecution {
        command: String,
        output: String,
        exit_code: Option<i32>,
    },
    PlanUpdate {
        items: Vec<PlanItem>,
    },
    /// A question the agent asked and the human's answer, paired.
    UserAnswer {
        question: String,
        answer: String,
    },
    /// A provider's own compaction summary. Low trust: lossy and possibly wrong.
    NativeCompactionSummary {
        text: String,
    },
    /// Pi's summary of an abandoned branch. Low trust.
    BranchSummary {
        text: String,
    },
    /// The user undid `num_turns` turns. Applied during branch resolution.
    Rollback {
        num_turns: u32,
    },
    ModelChange {
        model: String,
    },
    SubagentSpawn {
        stream: StreamId,
        prompt: String,
    },
    SubagentResult {
        stream: StreamId,
        text: String,
    },
    System {
        subtype: String,
        text: Option<String>,
    },
    /// An unrecognized line. Never dropped, so nothing is silently lost.
    Unknown {
        raw: serde_json::Value,
    },
}

impl EventKind {
    /// Short kind name for diagnostics and JSON summaries.
    pub fn name(&self) -> &'static str {
        match self {
            EventKind::UserMessage { .. } => "user_message",
            EventKind::AssistantText { .. } => "assistant_text",
            EventKind::Reasoning { .. } => "reasoning",
            EventKind::ToolCall { .. } => "tool_call",
            EventKind::ToolResult { .. } => "tool_result",
            EventKind::ShellExecution { .. } => "shell_execution",
            EventKind::PlanUpdate { .. } => "plan_update",
            EventKind::UserAnswer { .. } => "user_answer",
            EventKind::NativeCompactionSummary { .. } => "native_compaction_summary",
            EventKind::BranchSummary { .. } => "branch_summary",
            EventKind::Rollback { .. } => "rollback",
            EventKind::ModelChange { .. } => "model_change",
            EventKind::SubagentSpawn { .. } => "subagent_spawn",
            EventKind::SubagentResult { .. } => "subagent_result",
            EventKind::System { .. } => "system",
            EventKind::Unknown { .. } => "unknown",
        }
    }

    /// True for a message a human actually typed.
    pub fn is_human_message(&self) -> bool {
        matches!(
            self,
            EventKind::UserMessage { is_meta: false, .. } | EventKind::UserAnswer { .. }
        )
    }
}

/// Session-level facts recorded by the provider.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    /// Every model seen in the session, sorted and deduplicated.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub forked_from: Option<String>,
}

/// A provider compaction boundary and the summary it left behind.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeCompaction {
    pub evt: EventIdx,
    /// `None` when the provider stored the summary encrypted (hosted Codex).
    pub summary: Option<String>,
}

/// Something the adapter noticed but did not treat as fatal.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "diagnostic", rename_all = "snake_case")]
#[non_exhaustive]
pub enum Diagnostic {
    BadLine {
        path: u16,
        line: u32,
        message: String,
    },
    UnknownLineKind {
        kind: String,
        count: usize,
    },
    OrphanToolResult {
        evt: EventIdx,
        call_id: String,
    },
    AbandonedBranch {
        from: EventIdx,
        len: usize,
    },
    MissingParent {
        evt: EventIdx,
        parent: String,
    },
    UnseenAgentVersion {
        version: String,
    },
    Note {
        message: String,
    },
}

/// A parsed session: every event, plus which of them are still live.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub agent: AgentKind,
    pub id: String,
    pub source_paths: Vec<PathBuf>,
    /// sha256 of the decompressed source bytes, hex encoded.
    pub source_hash: String,
    pub meta: SessionMeta,
    /// Every event in file order; `events[i].idx == i`.
    pub events: Vec<Event>,
    /// The active branch: events surviving rewinds and rollbacks, chronological.
    pub active: Vec<EventIdx>,
    pub native_compactions: Vec<NativeCompaction>,
    pub diagnostics: Vec<Diagnostic>,
}

impl Session {
    /// Look up an event by index.
    pub fn event(&self, idx: EventIdx) -> Option<&Event> {
        self.events.get(idx as usize)
    }

    /// The active events, in order.
    pub fn active_events(&self) -> impl Iterator<Item = &Event> {
        self.active.iter().filter_map(|idx| self.event(*idx))
    }

    /// Number of human user turns on the active branch.
    pub fn user_turns(&self) -> usize {
        self.active_events()
            .filter(|e| e.kind.is_human_message())
            .count()
    }

    /// Count of each event kind on the active branch, for `report.json`.
    pub fn kind_counts(&self) -> BTreeMap<&'static str, usize> {
        let mut counts = BTreeMap::new();
        for event in self.active_events() {
            *counts.entry(event.kind.name()).or_insert(0) += 1;
        }
        counts
    }

    /// Check the invariants the pipeline relies on. Returns a list of
    /// violations; an empty list means the IR is well formed.
    ///
    /// Adapters are tolerant, so this is a test and `doctor` helper rather than
    /// a hard gate in the extract path.
    pub fn invariant_violations(&self) -> Vec<String> {
        let mut problems = Vec::new();
        for (position, event) in self.events.iter().enumerate() {
            if event.idx as usize != position {
                problems.push(format!("events[{position}].idx == {}", event.idx));
            }
        }
        if self.active.windows(2).any(|pair| pair[0] >= pair[1]) {
            problems.push("active is not strictly increasing".to_string());
        }
        if let Some(out_of_range) = self.active.iter().find(|idx| self.event(**idx).is_none()) {
            problems.push(format!("active contains out-of-range index {out_of_range}"));
        }
        problems
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(idx: EventIdx, kind: EventKind) -> Event {
        Event {
            idx,
            native_id: None,
            parent: None,
            ts: None,
            stream: 0,
            kind,
            line: LineRef {
                path: 0,
                line: idx + 1,
            },
        }
    }

    fn session(events: Vec<Event>, active: Vec<EventIdx>) -> Session {
        Session {
            agent: AgentKind::ClaudeCode,
            id: "s".into(),
            source_paths: vec![PathBuf::from("s.jsonl")],
            source_hash: String::new(),
            meta: SessionMeta::default(),
            events,
            active,
            native_compactions: Vec::new(),
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn agent_slugs_round_trip() {
        for agent in AgentKind::ALL {
            assert_eq!(AgentKind::from_slug(agent.slug()), Some(agent));
        }
        assert_eq!(
            AgentKind::from_slug("claude-code"),
            Some(AgentKind::ClaudeCode)
        );
        assert_eq!(AgentKind::from_slug("opencode"), None);
    }

    #[test]
    fn harness_injected_text_is_not_a_human_message() {
        assert!(
            EventKind::UserMessage {
                text: "hi".into(),
                is_meta: false
            }
            .is_human_message()
        );
        assert!(
            !EventKind::UserMessage {
                text: "hi".into(),
                is_meta: true
            }
            .is_human_message()
        );
    }

    #[test]
    fn well_formed_sessions_have_no_violations() {
        let session = session(
            vec![
                event(
                    0,
                    EventKind::UserMessage {
                        text: "a".into(),
                        is_meta: false,
                    },
                ),
                event(
                    1,
                    EventKind::AssistantText {
                        text: "b".into(),
                        phase: Phase::Final,
                    },
                ),
            ],
            vec![0, 1],
        );
        assert!(session.invariant_violations().is_empty());
        assert_eq!(session.user_turns(), 1);
    }

    #[test]
    fn violations_name_the_broken_invariant() {
        let mut broken = session(
            vec![event(
                0,
                EventKind::UserMessage {
                    text: "a".into(),
                    is_meta: false,
                },
            )],
            vec![0, 5],
        );
        assert!(
            broken
                .invariant_violations()
                .iter()
                .any(|p| p.contains("out-of-range"))
        );
        broken.active = vec![0, 0];
        assert!(
            broken
                .invariant_violations()
                .iter()
                .any(|p| p.contains("strictly increasing"))
        );
    }
}
