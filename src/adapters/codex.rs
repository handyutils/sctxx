//! Codex CLI adapter (spec §6.2).
//!
//! Rollout lines are envelopes: `{"timestamp", "type", "payload"}`. sctxx
//! decodes the payload as `serde_json::Value` and projects it into the IR
//! rather than modelling Codex's `ResponseItem` enum (decision D-2), so a new
//! item type costs a diagnostic instead of a parse failure.
//!
//! Codex records an undo as an `event_msg` rather than by rewriting the file,
//! so the active branch comes from replaying rollbacks with
//! [`crate::vendor::codex::reconstruction`].

use super::{
    SessionBuilder, content_text, first_str_field, output_text, str_field, string_field, tools,
};
use crate::error::Result;
use crate::ir::{AgentKind, EventIdx, EventKind, LineRef, Phase, PlanItem, Session, ToolClass};
use crate::vendor::codex::reconstruction::{ReplayEvent, surviving_indices};
use serde_json::Value;
use std::collections::HashMap;

/// Parse a Codex rollout file into the IR.
pub fn parse(source: super::source::SourceText) -> Result<Session> {
    let mut builder = SessionBuilder::new(AgentKind::Codex);
    // One replay marker per emitted event, in the same order.
    let mut replay: Vec<ReplayEvent> = Vec::new();
    // `request_user_input` call id -> the question asked, so the answer pairs.
    let mut pending_questions: HashMap<String, String> = HashMap::new();

    for (offset, text) in source.lines.iter().enumerate() {
        let line = LineRef {
            path: 0,
            line: offset as u32 + 1,
        };
        if text.trim().is_empty() {
            continue;
        }
        let raw: Value = match serde_json::from_str(text) {
            Ok(value) => value,
            Err(error) => {
                builder.note_bad_line(line, error.to_string());
                continue;
            }
        };
        let ts = string_field(&raw, "timestamp");
        let kind = str_field(&raw, "type").unwrap_or("").to_string();
        let payload = raw.get("payload").cloned().unwrap_or(Value::Null);

        match kind.as_str() {
            "session_meta" => read_session_meta(&mut builder, &payload),
            "turn_context" => {
                if let Some(model) = str_field(&payload, "model") {
                    builder.note_model(model);
                }
                if builder.meta.cwd.is_none() {
                    builder.meta.cwd = string_field(&payload, "cwd").map(std::path::PathBuf::from);
                }
            }
            "response_item" => {
                emit_response_item(
                    &mut builder,
                    &mut replay,
                    &payload,
                    ts,
                    line,
                    &mut pending_questions,
                );
            }
            "event_msg" => {
                // The only event message that changes history is a rollback.
                if str_field(&payload, "type") == Some("thread_rolled_back")
                    || payload.get("num_turns").is_some()
                        && str_field(&payload, "type").is_some_and(|t| t.contains("rolled_back"))
                {
                    let num_turns = payload
                        .get("num_turns")
                        .and_then(Value::as_u64)
                        .unwrap_or(1)
                        .min(u32::MAX as u64) as u32;
                    push(
                        &mut builder,
                        &mut replay,
                        EventKind::Rollback { num_turns },
                        None,
                        ts,
                        line,
                        ReplayEvent::Rollback { num_turns },
                    );
                }
            }
            "compacted" => {
                // A local compaction keeps readable replacement text; a hosted
                // one is encrypted and only marks the boundary.
                let text =
                    first_str_field(&payload, &["message", "summary", "text"]).or_else(|| {
                        payload
                            .get("replacement_history")
                            .map(content_text)
                            .filter(|t| !t.is_empty())
                    });
                let event = match text {
                    Some(text) => EventKind::NativeCompactionSummary { text },
                    None => EventKind::System {
                        subtype: "native_compaction".into(),
                        text: None,
                    },
                };
                let idx = push(
                    &mut builder,
                    &mut replay,
                    event,
                    None,
                    ts,
                    line,
                    ReplayEvent::Other,
                );
                // `window_number` means Codex replaced the context window and
                // kept the transcript; without it the item is a legacy history
                // reset. The difference decides where `--since-compact` starts
                // (`docs/adr/0002-codex-compaction-algorithm-reuse.md`).
                if payload
                    .get("window_number")
                    .and_then(Value::as_u64)
                    .is_some()
                {
                    builder.mark_compaction_windowed(idx);
                }
            }
            "token_usage_record"
            | "world_state"
            | "security_risk_score"
            | "retained_context"
            | "realtime_item"
            | "turn_diff"
            | "inter_agent_communication_metadata" => {
                // Accounting and internal state: not part of the transcript.
                builder.note_unknown_kind(kind);
            }
            other => {
                builder.note_unknown_kind(if other.is_empty() {
                    "<missing type>"
                } else {
                    other
                });
                push(
                    &mut builder,
                    &mut replay,
                    EventKind::Unknown { raw: raw.clone() },
                    None,
                    ts,
                    line,
                    ReplayEvent::Other,
                );
            }
        }
    }

    if builder.id.is_empty() {
        builder.id = id_from_filename(&source.path);
    }

    let active = surviving_indices(&replay);
    Ok(builder.finish(active, vec![source]))
}

/// Codex rollout files are named `rollout-<timestamp>-<uuid>.jsonl[.zst]`.
pub fn id_from_filename(path: &std::path::Path) -> String {
    let stem = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .trim_end_matches(".zst")
        .trim_end_matches(".jsonl")
        .to_string();
    // The uuid is the last five hyphen-separated groups.
    let parts: Vec<&str> = stem.split('-').collect();
    if parts.len() >= 5 {
        return parts[parts.len() - 5..].join("-");
    }
    stem
}

fn read_session_meta(builder: &mut SessionBuilder, payload: &Value) {
    // Some versions nest the record under `meta`.
    let meta = payload.get("meta").unwrap_or(payload);
    if builder.id.is_empty()
        && let Some(id) = first_str_field(meta, &["id", "session_id", "conversation_id"])
    {
        builder.id = id;
    }
    if builder.meta.cwd.is_none() {
        builder.meta.cwd = string_field(meta, "cwd").map(std::path::PathBuf::from);
    }
    builder.meta.started_at = builder
        .meta
        .started_at
        .take()
        .or_else(|| first_str_field(meta, &["timestamp", "started_at"]));
    builder.meta.agent_version = builder
        .meta
        .agent_version
        .take()
        .or_else(|| string_field(meta, "cli_version"));
    builder.meta.forked_from = builder
        .meta
        .forked_from
        .take()
        .or_else(|| string_field(meta, "forked_from_id"));
    if let Some(instructions) = payload.get("git").and_then(|git| str_field(git, "branch")) {
        builder.meta.git_branch = Some(instructions.to_string());
    }
    if let Some(model) = first_str_field(meta, &["model"]) {
        builder.note_model(model);
    }
}

#[allow(clippy::too_many_arguments)]
fn push(
    builder: &mut SessionBuilder,
    replay: &mut Vec<ReplayEvent>,
    kind: EventKind,
    native_id: Option<String>,
    ts: Option<String>,
    line: LineRef,
    marker: ReplayEvent,
) -> EventIdx {
    let idx = builder.push(kind, native_id, None, ts, 0, line);
    debug_assert_eq!(idx as usize, replay.len());
    replay.push(marker);
    idx
}

fn emit_response_item(
    builder: &mut SessionBuilder,
    replay: &mut Vec<ReplayEvent>,
    payload: &Value,
    ts: Option<String>,
    line: LineRef,
    pending_questions: &mut HashMap<String, String>,
) {
    let item_type = str_field(payload, "type").unwrap_or("").to_string();
    let id = string_field(payload, "id");

    match item_type.as_str() {
        "message" => {
            let role = str_field(payload, "role").unwrap_or("");
            let text = content_text(payload.get("content").unwrap_or(&Value::Null));
            if text.trim().is_empty() {
                return;
            }
            match role {
                // Developer messages are harness instructions, not transcript.
                "developer" | "system" => {}
                "user" => {
                    let is_meta = is_harness_context(&text);
                    let marker = if is_meta {
                        ReplayEvent::Other
                    } else {
                        ReplayEvent::UserTurnBoundary
                    };
                    push(
                        builder,
                        replay,
                        EventKind::UserMessage { text, is_meta },
                        id,
                        ts,
                        line,
                        marker,
                    );
                }
                _ => {
                    let phase = match str_field(payload, "phase") {
                        Some("commentary") => Phase::Commentary,
                        _ => Phase::Final,
                    };
                    push(
                        builder,
                        replay,
                        EventKind::AssistantText { text, phase },
                        id,
                        ts,
                        line,
                        ReplayEvent::Other,
                    );
                }
            }
        }
        "reasoning" => {
            let text = payload
                .get("summary")
                .map(content_text)
                .filter(|text| !text.trim().is_empty())
                .or_else(|| {
                    payload
                        .get("content")
                        .map(content_text)
                        .filter(|text| !text.trim().is_empty())
                });
            let redacted = text.is_none();
            push(
                builder,
                replay,
                EventKind::Reasoning { text, redacted },
                id,
                ts,
                line,
                ReplayEvent::Other,
            );
        }
        "function_call" | "custom_tool_call" | "local_shell_call" => {
            let name = first_str_field(payload, &["name", "tool_name"])
                .unwrap_or_else(|| item_type.clone());
            let call_id = first_str_field(payload, &["call_id", "id"]).unwrap_or_default();
            let args = parse_arguments(payload);
            if name == "request_user_input" {
                pending_questions.insert(call_id.clone(), question_text(&args));
            }
            let class = tools::classify(AgentKind::Codex, &name);
            push(
                builder,
                replay,
                EventKind::ToolCall {
                    call_id,
                    name,
                    class,
                    args: args.clone(),
                },
                id,
                ts.clone(),
                line,
                ReplayEvent::Other,
            );
            if class == ToolClass::Plan
                && let Some(items) = plan_items(&args)
            {
                push(
                    builder,
                    replay,
                    EventKind::PlanUpdate { items },
                    None,
                    ts,
                    line,
                    ReplayEvent::Other,
                );
            }
        }
        "function_call_output" | "custom_tool_call_output" => {
            let call_id = first_str_field(payload, &["call_id", "id"]).unwrap_or_default();
            let output_value = payload.get("output").unwrap_or(&Value::Null);
            let output = output_text(output_value);
            let is_error = output_value
                .get("success")
                .and_then(Value::as_bool)
                .map(|success| !success)
                .or_else(|| payload.get("is_error").and_then(Value::as_bool));
            let exit_code = output_value
                .get("exit_code")
                .and_then(Value::as_i64)
                .and_then(|c| i32::try_from(c).ok());

            // A `request_user_input` answer is a human turn, not tool output.
            if let Some(question) = pending_questions.remove(&call_id) {
                let answer = answer_text(output_value, &output);
                push(
                    builder,
                    replay,
                    EventKind::UserAnswer { question, answer },
                    id,
                    ts,
                    line,
                    ReplayEvent::UserTurnBoundary,
                );
                return;
            }
            push(
                builder,
                replay,
                EventKind::ToolResult {
                    call_id,
                    output,
                    is_error,
                    exit_code,
                },
                id,
                ts,
                line,
                ReplayEvent::Other,
            );
        }
        "web_search_call" => {
            let query = first_str_field(payload, &["query", "action"]).unwrap_or_default();
            push(
                builder,
                replay,
                EventKind::ToolCall {
                    call_id: string_field(payload, "id").unwrap_or_default(),
                    name: "web_search_call".into(),
                    class: ToolClass::Web,
                    args: serde_json::json!({ "query": query }),
                },
                None,
                ts,
                line,
                ReplayEvent::Other,
            );
        }
        "compaction" | "context_compaction" => {
            push(
                builder,
                replay,
                EventKind::System {
                    subtype: "native_compaction".into(),
                    text: None,
                },
                id,
                ts,
                line,
                ReplayEvent::Other,
            );
        }
        other => {
            builder.note_unknown_kind(format!("response_item:{other}"));
            push(
                builder,
                replay,
                EventKind::Unknown {
                    raw: payload.clone(),
                },
                id,
                ts,
                line,
                ReplayEvent::Other,
            );
        }
    }
}

/// Codex stores tool arguments as a JSON string; decode it when possible so the
/// ledgers can read named fields.
fn parse_arguments(payload: &Value) -> Value {
    match payload.get("arguments").or_else(|| payload.get("input")) {
        Some(Value::String(text)) => {
            serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.clone()))
        }
        Some(other) => other.clone(),
        None => payload.get("action").cloned().unwrap_or(Value::Null),
    }
}

fn question_text(args: &Value) -> String {
    if let Some(text) = first_str_field(args, &["question", "prompt", "message"]) {
        return text;
    }
    if let Some(questions) = args.get("questions").and_then(Value::as_array) {
        let joined: Vec<String> = questions
            .iter()
            .filter_map(|q| first_str_field(q, &["question", "prompt", "text", "header"]))
            .collect();
        if !joined.is_empty() {
            return joined.join(" · ");
        }
    }
    content_text(args)
}

fn answer_text(output: &Value, rendered: &str) -> String {
    if let Some(answers) = output.get("answers").and_then(Value::as_object) {
        let mut collected: Vec<String> = Vec::new();
        for value in answers.values() {
            if let Some(list) = value.get("answers").and_then(Value::as_array) {
                collected.extend(list.iter().filter_map(Value::as_str).map(str::to_string));
            }
        }
        if !collected.is_empty() {
            return collected.join("; ");
        }
    }
    rendered.to_string()
}

/// Codex injects environment and instruction fragments as user-role messages;
/// they must never be read as something a human asked for.
fn is_harness_context(text: &str) -> bool {
    // Prose markers Codex uses that are not XML-ish tags. Each was observed in
    // a real rollout.
    const MARKERS: &[&str] = &[
        "# AGENTS.md instructions",
        "## My request for Codex:",
        "# Files mentioned by the user:",
    ];
    super::looks_like_harness_fragment(text, MARKERS)
}

fn plan_items(args: &Value) -> Option<Vec<PlanItem>> {
    let plan = args.get("plan").or_else(|| args.get("steps"))?.as_array()?;
    let items: Vec<PlanItem> = plan
        .iter()
        .filter_map(|step| {
            let text = first_str_field(step, &["step", "text", "content"])?;
            let status =
                first_str_field(step, &["status", "state"]).unwrap_or_else(|| "pending".into());
            Some(PlanItem { text, status })
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_recovered_from_a_rollout_filename() {
        let path = std::path::Path::new(
            "rollout-2026-09-10T12-00-00-6f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8.jsonl",
        );
        assert_eq!(
            id_from_filename(path),
            "6f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8"
        );
    }

    #[test]
    fn a_compressed_rollout_name_resolves_the_same() {
        let path = std::path::Path::new("rollout-x-6f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8.jsonl.zst");
        assert_eq!(
            id_from_filename(path),
            "6f1a2b3c-4d5e-6f70-8192-a3b4c5d6e7f8"
        );
    }

    #[test]
    fn harness_fragments_are_not_human_messages() {
        assert!(is_harness_context(
            "<environment_context>\ncwd=/x\n</environment_context>"
        ));
        assert!(is_harness_context("  # AGENTS.md instructions"));
        // Observed in real rollouts: an injected plugin catalogue and an
        // attachment manifest.
        assert!(is_harness_context(
            "<recommended_plugins>a</recommended_plugins>"
        ));
        assert!(is_harness_context(
            "# Files mentioned by the user: ## clip.png: /tmp/clip.png"
        ));
        assert!(!is_harness_context("please fix the auth migration"));
    }

    #[test]
    fn a_multibyte_character_at_a_marker_boundary_does_not_panic() {
        // Observed in a real rollout: box-drawing and em-dash characters land
        // exactly where a marker prefix would be sliced.
        for text in [
            "\u{273b} Connection refused \u{2014} a firewall may be blocking it",
            "\u{2500}",
            "\u{1f600}",
            "<\u{2014}",
            "",
        ] {
            assert!(!is_harness_context(text), "{text}");
        }
    }

    #[test]
    fn string_arguments_are_decoded_to_json() {
        let payload = serde_json::json!({"arguments": "{\"command\":\"cargo test\"}"});
        assert_eq!(
            str_field(&parse_arguments(&payload), "command"),
            Some("cargo test")
        );
    }

    #[test]
    fn unparseable_arguments_survive_as_a_string() {
        let payload = serde_json::json!({"arguments": "not json"});
        assert_eq!(parse_arguments(&payload), Value::String("not json".into()));
    }
}
