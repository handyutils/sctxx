//! S2 — masking (spec §7.3).
//!
//! A masked row is what an LLM (and `sctxx show --view masked`) is allowed to
//! see: one compact line per active event, with tool output replaced by a
//! placeholder unless it failed. Failing output is the useful kind, so it
//! survives up to Codex's 2,000-token tool cap. Every row is redacted before
//! it leaves this module.

use crate::ir::{Event, EventKind, Phase, Session, ToolClass};
use crate::vendor::codex::secrets::{RedactMode, redact};
use crate::vendor::codex::tiered_input::{Tier, TieredRow};
use crate::vendor::codex::truncate::{approx_token_count, truncate_middle_tokens};
use serde_json::Value;

/// Per-row token caps (spec §7.3).
const ASSISTANT_FINAL_TOKENS: usize = 1_000;
const COMMENTARY_TOKENS: usize = 300;
const REASONING_TOKENS: usize = 300;
const USER_TOKENS: usize = 1_500;
const PRIOR_SUMMARY_TOKENS: usize = 1_500;
/// Codex `TOOL_OUTPUT_TOKENS`.
const TOOL_ERROR_TOKENS: usize = 2_000;
const ARG_VALUE_BYTES: usize = 200;

/// How reasoning text is treated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum ReasoningPolicy {
    /// Omit reasoning rows entirely.
    #[default]
    Drop,
    /// Include readable reasoning, truncated.
    Keep,
}

/// Options for building masked rows.
#[derive(Debug, Clone, Copy)]
pub struct MaskOptions {
    pub reasoning: ReasoningPolicy,
    /// Keep `System`/`Unknown` events as rows.
    pub keep_system: bool,
    /// Include subagent (sidechain) streams.
    pub include_sidechains: bool,
    pub redact: RedactMode,
}

impl Default for MaskOptions {
    fn default() -> Self {
        Self {
            reasoning: ReasoningPolicy::Drop,
            keep_system: false,
            include_sidechains: false,
            redact: RedactMode::Default,
        }
    }
}

/// One masked transcript line.
#[derive(Debug, Clone)]
pub struct Row {
    pub evt: u32,
    pub tier: Tier,
    pub text: String,
    pub tokens: usize,
    /// True for a human message: episode boundaries start here.
    pub is_human_turn: bool,
}

impl Row {
    /// Convert to the tiered-budget representation.
    pub fn tiered(&self) -> TieredRow {
        TieredRow {
            tier: self.tier,
            evt: self.evt,
            text: self.text.clone(),
        }
    }
}

/// Build masked rows for the session's active branch.
pub fn build(session: &Session, options: &MaskOptions) -> Vec<Row> {
    session
        .active_events()
        .filter(|event| options.include_sidechains || event.stream == 0)
        .filter_map(|event| row_for(event, options))
        .collect()
}

fn row_for(event: &Event, options: &MaskOptions) -> Option<Row> {
    let (tier, text) = match &event.kind {
        EventKind::UserMessage { text, is_meta } => {
            let label = if *is_meta { "user·harness" } else { "user" };
            let tier = if *is_meta {
                Tier::Commentary
            } else {
                Tier::User
            };
            (
                tier,
                format!("[{label}] {}", truncate_middle_tokens(text, USER_TOKENS)),
            )
        }
        EventKind::UserAnswer { question, answer } => (
            Tier::User,
            format!(
                "[user·answer] (asked: {}) {}",
                truncate_middle_tokens(question, 120),
                truncate_middle_tokens(answer, USER_TOKENS)
            ),
        ),
        EventKind::AssistantText { text, phase } => match phase {
            Phase::Final => (
                Tier::AssistantFinal,
                format!(
                    "[assistant] {}",
                    truncate_middle_tokens(text, ASSISTANT_FINAL_TOKENS)
                ),
            ),
            Phase::Commentary => (
                Tier::Commentary,
                format!(
                    "[assistant·commentary] {}",
                    truncate_middle_tokens(text, COMMENTARY_TOKENS)
                ),
            ),
        },
        EventKind::Reasoning { text, .. } => {
            if options.reasoning == ReasoningPolicy::Drop {
                return None;
            }
            let text = text.as_ref()?;
            (
                Tier::Commentary,
                format!(
                    "[reasoning] {}",
                    truncate_middle_tokens(text, REASONING_TOKENS)
                ),
            )
        }
        EventKind::ToolCall {
            call_id,
            name,
            class,
            args,
        } => {
            // A plan tool also emits a `PlanUpdate`, which renders the same
            // content readably. Two rows for one act is noise.
            if *class == ToolClass::Plan {
                return None;
            }
            (
                Tier::ToolCall,
                format!(
                    "[call {name} #{}] {}",
                    short_id(call_id),
                    render_args(*class, args)
                ),
            )
        }
        EventKind::ToolResult {
            call_id,
            output,
            is_error,
            exit_code,
        } => {
            let failed = *is_error == Some(true) || exit_code.is_some_and(|code| code != 0);
            if failed {
                // Not every provider records an exit code; say nothing rather
                // than print a placeholder.
                let exit = exit_code
                    .map(|code| format!(" exit={code}"))
                    .unwrap_or_default();
                (
                    Tier::ToolResultError,
                    format!(
                        "[result #{} ERROR{exit}] {}",
                        short_id(call_id),
                        head_and_tail(output, TOOL_ERROR_TOKENS)
                    ),
                )
            } else {
                (
                    Tier::ToolResultOk,
                    format!("[result #{}] {}", short_id(call_id), placeholder(output)),
                )
            }
        }
        EventKind::ShellExecution {
            command,
            output,
            exit_code,
        } => {
            let failed = exit_code.is_some_and(|code| code != 0);
            let tier = if failed {
                Tier::ToolResultError
            } else {
                Tier::ToolCall
            };
            let rendered = if failed {
                head_and_tail(output, TOOL_ERROR_TOKENS)
            } else {
                placeholder(output)
            };
            let exit = exit_code
                .map(|code| format!(" exit={code}"))
                .unwrap_or_default();
            (
                tier,
                format!(
                    "[shell{exit}] {} → {rendered}",
                    truncate_middle_tokens(command, 120)
                ),
            )
        }
        EventKind::PlanUpdate { items } => {
            let rendered: Vec<String> = items
                .iter()
                .map(|item| {
                    let box_char = match item.status.as_str() {
                        "completed" | "done" => "☑",
                        "in_progress" | "active" => "▸",
                        _ => "☐",
                    };
                    format!("{box_char} {}", item.text)
                })
                .collect();
            (Tier::Commentary, format!("[plan] {}", rendered.join(" · ")))
        }
        EventKind::NativeCompactionSummary { text } | EventKind::BranchSummary { text } => (
            Tier::PriorSummary,
            format!(
                "[prior-summary low-trust] {}",
                truncate_middle_tokens(text, PRIOR_SUMMARY_TOKENS)
            ),
        ),
        EventKind::SubagentSpawn { stream, prompt } => (
            Tier::Subagent,
            format!(
                "[subagent#{stream} spawn] {}",
                truncate_middle_tokens(prompt, 300)
            ),
        ),
        EventKind::SubagentResult { stream, text } => (
            Tier::Subagent,
            format!(
                "[subagent#{stream} result] {}",
                truncate_middle_tokens(text, 600)
            ),
        ),
        EventKind::ModelChange { model } => (Tier::Commentary, format!("[model] {model}")),
        // Already applied during branch resolution.
        EventKind::Rollback { .. } => return None,
        EventKind::System { subtype, text } => {
            if !options.keep_system {
                return None;
            }
            let text = text.clone().unwrap_or_default();
            (
                Tier::Commentary,
                format!("[system {subtype}] {}", truncate_middle_tokens(&text, 200)),
            )
        }
        EventKind::Unknown { .. } => {
            if !options.keep_system {
                return None;
            }
            (Tier::Commentary, "[unknown line]".to_string())
        }
    };

    let text = redact(&collapse_blank_lines(&text), options.redact);
    Some(Row {
        evt: event.idx,
        tier,
        tokens: approx_token_count(&text),
        is_human_turn: event.kind.is_human_message() && event.stream == 0,
        text,
    })
}

/// Tool output that succeeded is replaced by a one-line shape description: the
/// content is recoverable through `sctxx expand`, and it is almost never what
/// the next agent needs.
fn placeholder(output: &str) -> String {
    let lines = output.lines().count();
    let bytes = output.len();
    if output.trim().is_empty() {
        return "[empty]".to_string();
    }
    let first = output.lines().next().unwrap_or("").trim();
    let preview: String = first.chars().take(80).collect();
    if lines <= 1 && bytes <= 80 {
        format!("[{preview}]")
    } else {
        format!("[{lines} lines, {bytes} bytes: {preview}…]")
    }
}

/// Keep 20 lines from each end of failing output, under the token cap.
fn head_and_tail(output: &str, max_tokens: usize) -> String {
    let lines: Vec<&str> = output.lines().collect();
    let rendered = if lines.len() <= 40 {
        output.to_string()
    } else {
        let head = lines[..20].join("\n");
        let tail = lines[lines.len() - 20..].join("\n");
        format!("{head}\n[… {} lines omitted …]\n{tail}", lines.len() - 40)
    };
    truncate_middle_tokens(&rendered, max_tokens)
}

/// Render call arguments as `key=value`, with edit tools reduced to the path.
fn render_args(class: ToolClass, args: &Value) -> String {
    if class == ToolClass::Edit {
        if let Some(path) = crate::adapters::tools::PATH_ARG_KEYS
            .iter()
            .find_map(|key| args.get(key).and_then(Value::as_str))
        {
            let stat = edit_stat(args);
            return format!("{path}{stat}");
        }
        // A patch envelope: name the files, never the hunks.
        let ops = crate::vendor::codex::apply_patch_paths::parse_ops(
            args.get("input")
                .and_then(Value::as_str)
                .unwrap_or_default(),
        );
        if !ops.is_empty() {
            let paths: Vec<String> = ops
                .iter()
                .map(|op| op.path().to_string_lossy().into_owned())
                .collect();
            return paths.join(", ");
        }
    }

    match args {
        Value::Object(map) => {
            let mut parts: Vec<String> = Vec::new();
            for (key, value) in map {
                let rendered = match value {
                    Value::String(text) => {
                        crate::vendor::codex::truncate::truncate_middle_bytes(text, ARG_VALUE_BYTES)
                    }
                    Value::Null => continue,
                    other => crate::vendor::codex::truncate::truncate_middle_bytes(
                        &other.to_string(),
                        ARG_VALUE_BYTES,
                    ),
                };
                parts.push(format!("{key}={rendered}"));
            }
            parts.join(" ")
        }
        Value::String(text) => {
            crate::vendor::codex::truncate::truncate_middle_bytes(text, ARG_VALUE_BYTES)
        }
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A diff stat for an edit, without the diff itself.
fn edit_stat(args: &Value) -> String {
    let added = args
        .get("new_string")
        .or_else(|| args.get("content"))
        .and_then(Value::as_str)
        .map(|text| text.lines().count());
    let removed = args
        .get("old_string")
        .and_then(Value::as_str)
        .map(|text| text.lines().count());
    match (added, removed) {
        (Some(added), Some(removed)) => format!(" (+{added} −{removed})"),
        (Some(added), None) => format!(" (+{added})"),
        _ => String::new(),
    }
}

/// Call ids are long and never read by a human; keep enough to pair a result.
fn short_id(call_id: &str) -> String {
    if call_id.len() <= 12 {
        return call_id.to_string();
    }
    call_id
        .chars()
        .rev()
        .take(8)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect()
}

fn collapse_blank_lines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut blanks = 0;
    for line in text.lines() {
        if line.trim().is_empty() {
            blanks += 1;
            if blanks > 1 {
                continue;
            }
        } else {
            blanks = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end().to_string()
}

/// Render rows as the masked transcript text `sctxx show --view masked` prints.
pub fn render(rows: &[Row]) -> String {
    rows.iter()
        .map(|row| format!("{}  (evt {})\n", row.text, row.evt))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{AgentKind, EventIdx, LineRef, SessionMeta};

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

    fn session(events: Vec<Event>) -> Session {
        let active = events.iter().map(|event| event.idx).collect();
        Session {
            agent: AgentKind::ClaudeCode,
            id: "s".into(),
            source_paths: vec![],
            source_hash: String::new(),
            meta: SessionMeta::default(),
            events,
            active,
            native_compactions: vec![],
            diagnostics: vec![],
        }
    }

    #[test]
    fn successful_tool_output_becomes_a_placeholder() {
        let session = session(vec![event(
            0,
            EventKind::ToolResult {
                call_id: "toolu_abcdefghijkl".into(),
                output: (0..500).map(|i| format!("line {i}\n")).collect(),
                is_error: Some(false),
                exit_code: Some(0),
            },
        )]);
        let rows = build(&session, &MaskOptions::default());
        assert_eq!(rows.len(), 1);
        assert!(rows[0].text.contains("500 lines"), "{}", rows[0].text);
        assert!(
            !rows[0].text.contains("line 250"),
            "body leaked: {}",
            rows[0].text
        );
        assert_eq!(rows[0].tier, Tier::ToolResultOk);
    }

    #[test]
    fn a_plan_tool_call_is_not_duplicated_beside_its_plan_row() {
        let session = session(vec![
            event(
                0,
                EventKind::ToolCall {
                    call_id: "c1".into(),
                    name: "TodoWrite".into(),
                    class: ToolClass::Plan,
                    args: serde_json::json!({"todos": []}),
                },
            ),
            event(
                1,
                EventKind::PlanUpdate {
                    items: vec![crate::ir::PlanItem {
                        text: "wire tier 3".into(),
                        status: "in_progress".into(),
                    }],
                },
            ),
        ]);
        let rows = build(&session, &MaskOptions::default());
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert!(rows[0].text.starts_with("[plan]"), "{}", rows[0].text);
    }

    #[test]
    fn an_unknown_exit_code_is_omitted_rather_than_printed_as_a_placeholder() {
        let session = session(vec![event(
            0,
            EventKind::ToolResult {
                call_id: "c1".into(),
                output: "boom".into(),
                is_error: Some(true),
                exit_code: None,
            },
        )]);
        let rows = build(&session, &MaskOptions::default());
        assert!(rows[0].text.contains("ERROR]"), "{}", rows[0].text);
        assert!(!rows[0].text.contains("exit="), "{}", rows[0].text);
    }

    #[test]
    fn failing_tool_output_is_kept_because_it_is_the_useful_kind() {
        let session = session(vec![event(
            0,
            EventKind::ToolResult {
                call_id: "c1".into(),
                output: "error[E0308]: mismatched types".into(),
                is_error: Some(true),
                exit_code: Some(101),
            },
        )]);
        let rows = build(&session, &MaskOptions::default());
        assert!(rows[0].text.contains("E0308"), "{}", rows[0].text);
        assert!(rows[0].text.contains("exit=101"));
        assert_eq!(rows[0].tier, Tier::ToolResultError);
    }

    #[test]
    fn reasoning_is_dropped_by_default_and_kept_on_request() {
        let session = session(vec![event(
            0,
            EventKind::Reasoning {
                text: Some("thinking hard".into()),
                redacted: false,
            },
        )]);
        assert!(build(&session, &MaskOptions::default()).is_empty());
        let keep = MaskOptions {
            reasoning: ReasoningPolicy::Keep,
            ..MaskOptions::default()
        };
        assert_eq!(build(&session, &keep).len(), 1);
    }

    #[test]
    fn secrets_never_reach_a_row() {
        let session = session(vec![event(
            0,
            EventKind::UserMessage {
                text: "use token=ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".into(),
                is_meta: false,
            },
        )]);
        let rows = build(&session, &MaskOptions::default());
        assert!(
            rows[0].text.contains("[REDACTED_SECRET]"),
            "{}",
            rows[0].text
        );
        assert!(!rows[0].text.contains("ghp_ABCDEF"));
    }

    #[test]
    fn an_edit_call_shows_the_path_and_a_stat_not_the_diff() {
        let session = session(vec![event(
            0,
            EventKind::ToolCall {
                call_id: "c1".into(),
                name: "Edit".into(),
                class: ToolClass::Edit,
                args: serde_json::json!({
                    "file_path": "src/auth.ts",
                    "old_string": "a\nb",
                    "new_string": "a\nb\nc"
                }),
            },
        )]);
        let rows = build(&session, &MaskOptions::default());
        assert!(rows[0].text.contains("src/auth.ts"), "{}", rows[0].text);
        assert!(rows[0].text.contains("(+3 −2)"), "{}", rows[0].text);
        assert!(!rows[0].text.contains("old_string"));
    }

    #[test]
    fn harness_messages_are_labelled_and_not_human_turns() {
        let session = session(vec![event(
            0,
            EventKind::UserMessage {
                text: "<environment_context>".into(),
                is_meta: true,
            },
        )]);
        let rows = build(&session, &MaskOptions::default());
        assert!(rows[0].text.starts_with("[user·harness]"));
        assert!(!rows[0].is_human_turn);
    }
}
