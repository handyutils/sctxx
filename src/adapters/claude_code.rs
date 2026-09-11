//! Claude Code adapter (spec §6.3).
//!
//! **Clean-room.** Claude Code is not open source. This adapter is written
//! only from (a) the shape of session files observed on the maintainers' own
//! machines, (b) public Anthropic documentation, and (c) fixtures contributed
//! from users' own sessions. No Claude Code source — or any fork or leak of it
//! — was read, copied, paraphrased, or ported. See
//! `.claude/rules/clean-room-adapters.md`.
//!
//! Entries form a tree through `uuid`/`parentUuid`: a rewind or an edited
//! message creates a sibling rather than truncating the file, so the live
//! conversation is the path from the newest conversational leaf back to the
//! root. `logicalParentUuid` bridges a compaction boundary, where the physical
//! parent is absent.

use super::{
    SessionBuilder, bool_field, content_text, first_str_field, output_text, str_field,
    string_field, tools,
};
use crate::error::Result;
use crate::ir::{
    AgentKind, Diagnostic, EventIdx, EventKind, LineRef, Phase, PlanItem, Session, StreamId,
};
use serde_json::Value;
use std::collections::HashMap;

/// One parsed line of a Claude Code session file, reduced to what branch
/// resolution needs plus the IR events it produced.
struct Entry {
    uuid: Option<String>,
    parent: Option<String>,
    logical_parent: Option<String>,
    is_sidechain: bool,
    kind: String,
    events: Vec<EventIdx>,
}

/// Parse a Claude Code session file into the IR.
pub fn parse(source: super::source::SourceText) -> Result<Session> {
    parse_with_sidechains(source, Vec::new())
}

/// Parse a session together with its subagent transcript files.
///
/// Newer Claude Code versions write each subagent to
/// `<project>/<session-id>/subagents/agent-<slug>.jsonl` instead of inlining
/// it, so a complete picture needs those files too. Each file becomes its own
/// stream; only the spawn and the result appear in the main conversation.
pub fn parse_with_sidechains(
    main: super::source::SourceText,
    sidechain_files: Vec<super::source::SourceText>,
) -> Result<Session> {
    let mut builder = SessionBuilder::new(AgentKind::ClaudeCode);
    let mut entries: Vec<Entry> = Vec::new();
    // uuid of an inlined sidechain entry -> the stream it belongs to.
    let mut streams: HashMap<String, StreamId> = HashMap::new();
    let mut next_stream: StreamId = 1;
    // tool_use id of a Task/Agent call -> the subagent stream it started.
    let mut subagent_calls: HashMap<String, StreamId> = HashMap::new();

    let mut sources = vec![main];
    sources.extend(sidechain_files);

    for (file_index, source) in sources.iter().enumerate() {
        // Every entry in a subagent file belongs to that file's own stream.
        let file_stream: Option<StreamId> = if file_index == 0 {
            None
        } else {
            let stream = next_stream;
            next_stream += 1;
            Some(stream)
        };

        for (offset, text) in source.lines.iter().enumerate() {
            let line = LineRef {
                path: file_index as u16,
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
            let is_sidechain = bool_field(&raw, "isSidechain") || file_stream.is_some();
            let uuid = string_field(&raw, "uuid");
            let parent = string_field(&raw, "parentUuid");
            let logical_parent = string_field(&raw, "logicalParentUuid");

            read_meta(&mut builder, &raw);

            // In the main file, an inlined sidechain entry inherits its
            // parent's stream or opens a new one.
            let stream = match file_stream {
                Some(stream) => stream,
                None if is_sidechain => {
                    let inherited = parent.as_ref().and_then(|p| streams.get(p).copied());
                    let stream = inherited.unwrap_or_else(|| {
                        let stream = next_stream;
                        next_stream += 1;
                        stream
                    });
                    if let Some(uuid) = &uuid {
                        streams.insert(uuid.clone(), stream);
                    }
                    stream
                }
                None => 0,
            };

            let events = emit_events(
                &mut builder,
                &raw,
                &kind,
                stream,
                line,
                uuid.clone(),
                parent.clone(),
                &mut subagent_calls,
                &mut next_stream,
            );
            entries.push(Entry {
                uuid,
                parent,
                logical_parent,
                is_sidechain,
                kind,
                events,
            });
        }
    }

    if builder.id.is_empty() {
        builder.id = sources
            .first()
            .and_then(|source| source.path.file_stem())
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| "unknown".to_string());
    }

    let active = active_branch(&mut builder, &entries);
    Ok(builder.finish(active, sources))
}

fn read_meta(builder: &mut SessionBuilder, raw: &Value) {
    if builder.id.is_empty()
        && let Some(id) = string_field(raw, "sessionId")
    {
        builder.id = id;
    }
    if builder.meta.cwd.is_none() {
        builder.meta.cwd = string_field(raw, "cwd").map(std::path::PathBuf::from);
    }
    if builder.meta.git_branch.is_none() {
        builder.meta.git_branch = string_field(raw, "gitBranch");
    }
    if builder.meta.agent_version.is_none() {
        builder.meta.agent_version = string_field(raw, "version");
    }
    if let Some(model) = raw.get("message").and_then(|m| str_field(m, "model")) {
        builder.note_model(model);
    }
    // `{"type":"summary","summary":"..."}` lines carry the session title.
    if str_field(raw, "type") == Some("summary")
        && let Some(summary) = string_field(raw, "summary")
    {
        builder.meta.title = Some(summary);
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_events(
    builder: &mut SessionBuilder,
    raw: &Value,
    kind: &str,
    stream: StreamId,
    line: LineRef,
    uuid: Option<String>,
    parent: Option<String>,
    subagent_calls: &mut HashMap<String, StreamId>,
    next_stream: &mut StreamId,
) -> Vec<EventIdx> {
    let ts = string_field(raw, "timestamp");
    let push = |builder: &mut SessionBuilder, event: EventKind| -> EventIdx {
        builder.push(
            event,
            uuid.clone(),
            parent.clone(),
            ts.clone(),
            stream,
            line,
        )
    };

    match kind {
        "user" => {
            let message = raw.get("message").cloned().unwrap_or(Value::Null);
            let content = message.get("content").cloned().unwrap_or(Value::Null);
            let mut events = Vec::new();

            // A compaction summary is written as a user entry.
            if bool_field(raw, "isCompactSummary") {
                let text = content_text(&content);
                return vec![push(builder, EventKind::NativeCompactionSummary { text })];
            }

            if let Value::Array(blocks) = &content {
                for block in blocks {
                    match str_field(block, "type") {
                        Some("tool_result") => {
                            let call_id = first_str_field(block, &["tool_use_id", "toolUseId"])
                                .unwrap_or_default();
                            let output = {
                                let from_block =
                                    output_text(block.get("content").unwrap_or(&Value::Null));
                                if from_block.is_empty() {
                                    output_text(raw.get("toolUseResult").unwrap_or(&Value::Null))
                                } else {
                                    from_block
                                }
                            };
                            let is_error = block.get("is_error").and_then(Value::as_bool);
                            if let Some(stream_id) = subagent_calls.remove(&call_id) {
                                events.push(push(
                                    builder,
                                    EventKind::SubagentResult {
                                        stream: stream_id,
                                        text: output,
                                    },
                                ));
                            } else {
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
                        }
                        _ => {
                            let text = content_text(block);
                            if !text.trim().is_empty() {
                                let is_meta =
                                    bool_field(raw, "isMeta") || is_harness_wrapper(&text);
                                events
                                    .push(push(builder, EventKind::UserMessage { text, is_meta }));
                            }
                        }
                    }
                }
            } else {
                let text = content_text(&content);
                if !text.trim().is_empty() {
                    let is_meta = bool_field(raw, "isMeta") || is_harness_wrapper(&text);
                    events.push(push(builder, EventKind::UserMessage { text, is_meta }));
                }
            }
            events
        }
        "assistant" => {
            let message = raw.get("message").cloned().unwrap_or(Value::Null);
            let content = message.get("content").cloned().unwrap_or(Value::Null);
            let mut events = Vec::new();
            match &content {
                Value::Array(blocks) => {
                    for block in blocks {
                        match str_field(block, "type") {
                            Some("thinking") => {
                                let text = first_str_field(block, &["thinking", "text"]);
                                events.push(push(
                                    builder,
                                    EventKind::Reasoning {
                                        redacted: text.is_none(),
                                        text,
                                    },
                                ));
                            }
                            Some("tool_use") => {
                                events.extend(emit_tool_use(
                                    builder,
                                    block,
                                    stream,
                                    line,
                                    &uuid,
                                    &parent,
                                    &ts,
                                    subagent_calls,
                                    next_stream,
                                ));
                            }
                            Some("redacted_thinking") => {
                                events.push(push(
                                    builder,
                                    EventKind::Reasoning {
                                        text: None,
                                        redacted: true,
                                    },
                                ));
                            }
                            _ => {
                                let text = content_text(block);
                                if !text.trim().is_empty() {
                                    events.push(push(
                                        builder,
                                        EventKind::AssistantText {
                                            text,
                                            phase: Phase::Final,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                }
                other => {
                    let text = content_text(other);
                    if !text.trim().is_empty() {
                        events.push(push(
                            builder,
                            EventKind::AssistantText {
                                text,
                                phase: Phase::Final,
                            },
                        ));
                    }
                }
            }
            events
        }
        "system" => {
            let subtype = str_field(raw, "subtype").unwrap_or("system").to_string();
            let text = first_str_field(raw, &["content", "message", "text"]);
            vec![push(builder, EventKind::System { subtype, text })]
        }
        "summary" => {
            let text = string_field(raw, "summary");
            vec![push(
                builder,
                EventKind::System {
                    subtype: "summary".into(),
                    text,
                },
            )]
        }
        // The header line of a subagent transcript file, naming the turn the
        // subagent forked from.
        "fork-context-ref" => {
            let text = first_str_field(raw, &["agentId", "parentLastUuid"]);
            vec![push(
                builder,
                EventKind::System {
                    subtype: "fork_context_ref".into(),
                    text,
                },
            )]
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

#[allow(clippy::too_many_arguments)]
fn emit_tool_use(
    builder: &mut SessionBuilder,
    block: &Value,
    stream: StreamId,
    line: LineRef,
    uuid: &Option<String>,
    parent: &Option<String>,
    ts: &Option<String>,
    subagent_calls: &mut HashMap<String, StreamId>,
    next_stream: &mut StreamId,
) -> Vec<EventIdx> {
    let name = str_field(block, "name").unwrap_or("unknown").to_string();
    let call_id = first_str_field(block, &["id", "tool_use_id"]).unwrap_or_default();
    let args = block.get("input").cloned().unwrap_or(Value::Null);
    let class = tools::classify(builder.agent(), &name);
    let mut events = Vec::new();

    if class == crate::ir::ToolClass::Subagent {
        let subagent_stream = *next_stream;
        *next_stream += 1;
        subagent_calls.insert(call_id, subagent_stream);
        let prompt = first_str_field(&args, &["prompt", "description", "task"]).unwrap_or_default();
        events.push(builder.push(
            EventKind::SubagentSpawn {
                stream: subagent_stream,
                prompt,
            },
            uuid.clone(),
            parent.clone(),
            ts.clone(),
            stream,
            line,
        ));
        return events;
    }

    events.push(builder.push(
        EventKind::ToolCall {
            call_id,
            name,
            class,
            args: args.clone(),
        },
        uuid.clone(),
        parent.clone(),
        ts.clone(),
        stream,
        line,
    ));

    // A plan tool additionally publishes the plan state.
    if class == crate::ir::ToolClass::Plan
        && let Some(items) = plan_items(&args)
    {
        events.push(builder.push(
            EventKind::PlanUpdate { items },
            uuid.clone(),
            parent.clone(),
            ts.clone(),
            stream,
            line,
        ));
    }
    events
}

/// Claude Code records slash commands, command output, and system reminders
/// as user-role entries wrapped in markers. A human typed the command, but the
/// wrapper is harness formatting: treating it as a stated goal turns `/login`
/// into the session's objective.
fn is_harness_wrapper(text: &str) -> bool {
    // Prose markers Claude Code uses that are not XML-ish tags.
    const MARKERS: &[&str] = &["Caveat: The messages below were generated"];
    super::looks_like_harness_fragment(text, MARKERS)
}

fn plan_items(args: &Value) -> Option<Vec<PlanItem>> {
    let todos = args.get("todos").or_else(|| args.get("plan"))?.as_array()?;
    let items: Vec<PlanItem> = todos
        .iter()
        .filter_map(|todo| {
            let text = first_str_field(todo, &["content", "text", "activeForm", "step"])?;
            let status = first_str_field(todo, &["status", "state"])
                .unwrap_or_else(|| "pending".to_string());
            Some(PlanItem { text, status })
        })
        .collect();
    (!items.is_empty()).then_some(items)
}

/// Resolve the live conversation: walk from the newest conversational leaf back
/// to the root, bridging compaction boundaries through `logicalParentUuid`.
fn active_branch(builder: &mut SessionBuilder, entries: &[Entry]) -> Vec<EventIdx> {
    let by_uuid: HashMap<&str, usize> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| entry.uuid.as_deref().map(|uuid| (uuid, index)))
        .collect();

    let leaf = entries.iter().rposition(|entry| {
        !entry.is_sidechain
            && matches!(entry.kind.as_str(), "user" | "assistant")
            && entry.uuid.is_some()
    });
    let Some(leaf) = leaf else {
        // No tree at all (e.g. a file of summaries): everything in file order.
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
            break; // cycle or malformed parent chain
        }
        guard -= 1;
        on_path[index] = true;
        let entry = &entries[index];
        let next = entry
            .parent
            .as_deref()
            .and_then(|parent| by_uuid.get(parent).copied())
            .or_else(|| {
                // Physical parent missing: a compaction boundary points past it.
                entry
                    .logical_parent
                    .as_deref()
                    .and_then(|parent| by_uuid.get(parent).copied())
            });
        if next.is_none()
            && let Some(parent) = &entry.parent
            && !by_uuid.contains_key(parent.as_str())
            && entry.logical_parent.is_none()
            && let Some(evt) = entry.events.first()
        {
            builder.note(Diagnostic::MissingParent {
                evt: *evt,
                parent: parent.clone(),
            });
        }
        cursor = next;
    }

    // Record abandoned branches so `show` and diagnostics can surface them.
    let mut abandoned_from: Option<EventIdx> = None;
    let mut abandoned_len = 0usize;
    for (index, entry) in entries.iter().enumerate() {
        let is_conversational = matches!(entry.kind.as_str(), "user" | "assistant");
        if !on_path[index] && is_conversational && !entry.is_sidechain && !entry.events.is_empty() {
            if abandoned_from.is_none() {
                abandoned_from = entry.events.first().copied();
            }
            abandoned_len += entry.events.len();
        } else if let Some(from) = abandoned_from.take() {
            builder.note(Diagnostic::AbandonedBranch {
                from,
                len: abandoned_len,
            });
            abandoned_len = 0;
        }
    }
    if let Some(from) = abandoned_from {
        builder.note(Diagnostic::AbandonedBranch {
            from,
            len: abandoned_len,
        });
    }

    let mut active: Vec<EventIdx> = entries
        .iter()
        .enumerate()
        .filter(|(index, entry)| on_path[*index] && !entry.is_sidechain)
        .flat_map(|(_, entry)| entry.events.clone())
        .collect();

    // Summary lines sit outside the tree but describe the session; keep the
    // native compaction summaries so the fold sees the low-trust seed.
    for entry in entries {
        if entry.uuid.is_none() {
            for evt in &entry.events {
                if let Some(event) = builder.events().get(*evt as usize)
                    && matches!(event.kind, EventKind::NativeCompactionSummary { .. })
                {
                    active.push(*evt);
                }
            }
        }
    }

    active.sort_unstable();
    active.dedup();
    active
}

#[cfg(test)]
mod harness_tests {
    use super::is_harness_wrapper;

    #[test]
    fn slash_commands_and_reminders_are_harness_wrappers() {
        for text in [
            "<command-name>/login</command-name>",
            "  <local-command-stdout>done</local-command-stdout>",
            "<system-reminder>remember the plan</system-reminder>",
            "<command-name>/login</command-name> <command-args></command-args>",
            "Caveat: The messages below were generated by the user while running local commands",
        ] {
            assert!(is_harness_wrapper(text), "{text}");
        }
    }

    #[test]
    fn real_requests_are_not_harness_wrappers() {
        for text in [
            "implement the manifest loader",
            "<div> in the template is broken",
            "\u{2014} start here",
            "",
        ] {
            assert!(!is_harness_wrapper(text), "{text}");
        }
    }
}
