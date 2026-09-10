//! Pi adapter (spec §6.4).
//!
//! Written against Pi's published session-format documentation
//! (`packages/coding-agent/docs/session-format.md`); no Pi code is copied.
//! Session versions 1-3 are supported: v1 is linear, v2+ is a tree through
//! `id`/`parentId`, so the active branch is the path from the newest leaf back
//! to its root. Unlike Pi's own context builder, sctxx does **not** collapse
//! compacted ranges — it wants the full history and treats a compaction
//! summary as one low-trust event among the real ones.

use super::{
    SessionBuilder, content_text, first_str_field, i32_field, output_text, str_field, string_field,
    tools,
};
use crate::error::Result;
use crate::ir::{
    AgentKind, Diagnostic, EventIdx, EventKind, LineRef, Phase, PlanItem, Session, ToolClass,
};
use serde_json::Value;
use std::collections::HashMap;

struct Entry {
    id: Option<String>,
    parent: Option<String>,
    events: Vec<EventIdx>,
    in_tree: bool,
}

/// Parse a Pi session file into the IR.
pub fn parse(source: super::source::SourceText) -> Result<Session> {
    let mut builder = SessionBuilder::new(AgentKind::Pi);
    let mut entries: Vec<Entry> = Vec::new();
    let mut version = 1u64;

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
        let kind = str_field(&raw, "type").unwrap_or("").to_string();
        let ts = first_str_field(&raw, &["timestamp", "time"]);
        let id = first_str_field(&raw, &["id", "entryId"]);
        let parent = first_str_field(&raw, &["parentId", "parent"]);

        if kind == "session" {
            version = raw.get("version").and_then(Value::as_u64).unwrap_or(1);
            builder.id = id.clone().unwrap_or_default();
            builder.meta.cwd = string_field(&raw, "cwd").map(std::path::PathBuf::from);
            builder.meta.started_at = ts.clone();
            builder.meta.forked_from = first_str_field(&raw, &["parentSession", "parentSessionId"]);
            builder.meta.agent_version = string_field(&raw, "agentVersion");
            builder.note(Diagnostic::Note {
                message: format!("pi session format v{version}"),
            });
            continue;
        }

        let events = emit(&mut builder, &raw, &kind, &id, &parent, ts, line);
        let in_tree = id.is_some() && !events.is_empty();
        entries.push(Entry {
            id,
            parent,
            events,
            in_tree,
        });
    }

    if builder.id.is_empty() {
        builder.id = id_from_filename(&source.path);
    }

    let active = if version >= 2 {
        tree_branch(&mut builder, &entries)
    } else {
        entries
            .iter()
            .flat_map(|entry| entry.events.clone())
            .collect()
    };
    Ok(builder.finish(active, vec![source]))
}

/// Pi session files are named `<timestamp>_<session-id>.jsonl`.
pub fn id_from_filename(path: &std::path::Path) -> String {
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unknown".to_string());
    stem.split_once('_')
        .map(|(_, id)| id.to_string())
        .unwrap_or(stem)
}

fn emit(
    builder: &mut SessionBuilder,
    raw: &Value,
    kind: &str,
    id: &Option<String>,
    parent: &Option<String>,
    ts: Option<String>,
    line: LineRef,
) -> Vec<EventIdx> {
    let push = |builder: &mut SessionBuilder, event: EventKind| -> EventIdx {
        builder.push(event, id.clone(), parent.clone(), ts.clone(), 0, line)
    };

    match kind {
        "message" => emit_message(builder, raw, id, parent, ts, line),
        "bashExecution" | "bash_execution" => {
            if super::bool_field(raw, "excludeFromContext") {
                return Vec::new();
            }
            let command = first_str_field(raw, &["command", "cmd"]).unwrap_or_default();
            let output = output_text(
                raw.get("output")
                    .or_else(|| raw.get("result"))
                    .unwrap_or(&Value::Null),
            );
            let exit_code = i32_field(raw, "exitCode").or_else(|| i32_field(raw, "exit_code"));
            vec![push(
                builder,
                EventKind::ShellExecution {
                    command,
                    output,
                    exit_code,
                },
            )]
        }
        "compaction" | "compactionSummary" | "compaction_summary" => {
            let text = first_str_field(raw, &["summary", "text", "message"])
                .or_else(|| {
                    raw.get("details")
                        .map(content_text)
                        .filter(|text| !text.is_empty())
                })
                .unwrap_or_default();
            vec![push(builder, EventKind::NativeCompactionSummary { text })]
        }
        "branchSummary" | "branch_summary" => {
            let text = first_str_field(raw, &["summary", "text"]).unwrap_or_default();
            vec![push(builder, EventKind::BranchSummary { text })]
        }
        "modelChange" | "model_change" => {
            let model = first_str_field(raw, &["model", "to"]).unwrap_or_default();
            builder.note_model(model.clone());
            vec![push(builder, EventKind::ModelChange { model })]
        }
        "label" | "sessionInfo" | "session_info" => {
            if let Some(title) = first_str_field(raw, &["title", "label", "name"]) {
                builder.meta.title = Some(title);
            }
            Vec::new()
        }
        "custom" | "customMessage" | "custom_message" => {
            if super::bool_field(raw, "excludeFromContext") {
                return Vec::new();
            }
            let subtype = first_str_field(raw, &["customType", "custom_type", "subtype"])
                .unwrap_or_else(|| "custom".to_string());
            let text = first_str_field(raw, &["text", "content", "message"]);
            vec![push(builder, EventKind::System { subtype, text })]
        }
        other => {
            builder.note_unknown_kind(if other.is_empty() {
                "<missing type>"
            } else {
                other
            });
            vec![push(builder, EventKind::Unknown { raw: raw.clone() })]
        }
    }
}

fn emit_message(
    builder: &mut SessionBuilder,
    raw: &Value,
    id: &Option<String>,
    parent: &Option<String>,
    ts: Option<String>,
    line: LineRef,
) -> Vec<EventIdx> {
    let message = raw.get("message").cloned().unwrap_or(Value::Null);
    let role = str_field(&message, "role").unwrap_or("");
    let content = message.get("content").cloned().unwrap_or(Value::Null);
    let mut events = Vec::new();
    let push = |builder: &mut SessionBuilder, event: EventKind| -> EventIdx {
        builder.push(event, id.clone(), parent.clone(), ts.clone(), 0, line)
    };

    if let Some(model) = str_field(&message, "model") {
        builder.note_model(model);
    }

    match role {
        "user" => {
            let text = content_text(&content);
            if !text.trim().is_empty() {
                events.push(push(
                    builder,
                    EventKind::UserMessage {
                        text,
                        is_meta: super::bool_field(&message, "isContext")
                            || super::bool_field(raw, "isContext"),
                    },
                ));
            }
        }
        "toolResult" | "tool_result" => {
            let call_id = first_str_field(&message, &["toolCallId", "id"]).unwrap_or_default();
            let output = output_text(&content);
            let is_error = message.get("isError").and_then(Value::as_bool);
            events.push(push(
                builder,
                EventKind::ToolResult {
                    call_id,
                    output,
                    is_error,
                    exit_code: None,
                },
            ));
        }
        _ => {
            // Assistant messages carry text, thinking, and tool calls together.
            if let Some(thinking) = first_str_field(&message, &["thinking", "reasoning"]) {
                events.push(push(
                    builder,
                    EventKind::Reasoning {
                        text: Some(thinking),
                        redacted: false,
                    },
                ));
            } else if super::bool_field(&message, "redactedThinking") {
                events.push(push(
                    builder,
                    EventKind::Reasoning {
                        text: None,
                        redacted: true,
                    },
                ));
            }

            let text = content_text(&content);
            if !text.trim().is_empty() {
                events.push(push(
                    builder,
                    EventKind::AssistantText {
                        text,
                        phase: Phase::Final,
                    },
                ));
            }

            for call in tool_calls(&message) {
                let name = first_str_field(&call, &["name", "tool"]).unwrap_or_default();
                let call_id = first_str_field(&call, &["id", "toolCallId"]).unwrap_or_default();
                let args = call
                    .get("arguments")
                    .or_else(|| call.get("input"))
                    .cloned()
                    .unwrap_or(Value::Null);
                let class = tools::classify(AgentKind::Pi, &name);
                events.push(push(
                    builder,
                    EventKind::ToolCall {
                        call_id,
                        name,
                        class,
                        args: args.clone(),
                    },
                ));
                if class == ToolClass::Plan
                    && let Some(items) = plan_items(&args)
                {
                    events.push(push(builder, EventKind::PlanUpdate { items }));
                }
            }
        }
    }
    events
}

fn tool_calls(message: &Value) -> Vec<Value> {
    if let Some(call) = message.get("toolCall").filter(|call| !call.is_null()) {
        return vec![call.clone()];
    }
    message
        .get("toolCalls")
        .and_then(Value::as_array)
        .map(|calls| calls.to_vec())
        .unwrap_or_default()
}

fn plan_items(args: &Value) -> Option<Vec<PlanItem>> {
    let list = args
        .get("todos")
        .or_else(|| args.get("items"))?
        .as_array()?;
    let items: Vec<PlanItem> = list
        .iter()
        .filter_map(|todo| {
            let text = first_str_field(todo, &["content", "text", "title"])?;
            let status =
                first_str_field(todo, &["status", "state"]).unwrap_or_else(|| "pending".into());
            Some(PlanItem { text, status })
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

/// Walk from the newest tree leaf back to its root (Pi v2+).
fn tree_branch(builder: &mut SessionBuilder, entries: &[Entry]) -> Vec<EventIdx> {
    let by_id: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.id.as_deref().map(|id| (id, index)))
        .collect();

    let Some(leaf) = entries.iter().rposition(|entry| entry.in_tree) else {
        return entries
            .iter()
            .flat_map(|entry| entry.events.clone())
            .collect();
    };

    let mut on_path = vec![false; entries.len()];
    let mut cursor = Some(leaf);
    let mut guard = entries.len() + 1;
    while let Some(index) = cursor {
        if on_path[index] || guard == 0 {
            break;
        }
        guard -= 1;
        on_path[index] = true;
        cursor = entries[index]
            .parent
            .as_deref()
            .and_then(|parent| by_id.get(parent).copied());
    }

    let abandoned: usize = entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| !on_path[*index] && entry.in_tree)
        .map(|(_, entry)| entry.events.len())
        .sum();
    if abandoned > 0 {
        let from = entries
            .iter()
            .enumerate()
            .find(|(index, entry)| !on_path[*index] && entry.in_tree)
            .and_then(|(_, entry)| entry.events.first().copied())
            .unwrap_or(0);
        builder.note(Diagnostic::AbandonedBranch {
            from,
            len: abandoned,
        });
    }

    let mut active: Vec<EventIdx> = entries
        .iter()
        .enumerate()
        .filter(|(index, _)| on_path[*index])
        .flat_map(|(_, entry)| entry.events.clone())
        .collect();
    // Entries outside the tree (labels, model changes) still describe the run.
    active.extend(
        entries
            .iter()
            .filter(|entry| !entry.in_tree)
            .flat_map(|entry| entry.events.clone()),
    );
    active.sort_unstable();
    active.dedup();
    active
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_is_recovered_from_a_pi_filename() {
        let path = std::path::Path::new("2026-09-10T12-00-00_abc123.jsonl");
        assert_eq!(id_from_filename(path), "abc123");
    }

    #[test]
    fn a_filename_without_a_timestamp_prefix_is_used_whole() {
        assert_eq!(
            id_from_filename(std::path::Path::new("abc123.jsonl")),
            "abc123"
        );
    }
}
