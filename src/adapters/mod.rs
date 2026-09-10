//! Provider adapters: session files on disk to the canonical IR (spec §6).
//!
//! Adapters are deliberately tolerant. A provider can change its format
//! without notice, so an unrecognized line becomes [`EventKind::Unknown`] plus
//! a [`Diagnostic`] instead of failing the session; only a bad-line *rate*
//! above the threshold is fatal (exit 5).

pub mod claude_code;
pub mod codex;
pub mod discovery;
pub mod pi;
pub mod source;
pub mod tools;

use crate::error::{Error, Result};
use crate::ir::{
    AgentKind, Diagnostic, Event, EventIdx, EventKind, LineRef, NativeCompaction, Session,
    SessionMeta,
};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;

/// Default fraction of unparseable lines that is still tolerated (spec §3.1).
pub const DEFAULT_MAX_BAD_LINE_RATE: f64 = 0.02;

/// Detect which agent wrote a session file, from its first meaningful line
/// (spec §6.1).
pub fn detect(first_line: &str) -> Option<AgentKind> {
    let value: Value = serde_json::from_str(first_line).ok()?;
    let object = value.as_object()?;

    match object.get("type").and_then(Value::as_str) {
        // Pi writes an explicit session header with a format version.
        Some("session") if object.contains_key("version") => return Some(AgentKind::Pi),
        // Codex rollout lines are envelopes with a snake_case type + payload.
        Some(
            "session_meta" | "response_item" | "event_msg" | "turn_context" | "compacted"
            | "token_usage_record" | "turn_diff",
        ) if object.contains_key("payload") || object.contains_key("timestamp") => {
            return Some(AgentKind::Codex);
        }
        _ => {}
    }

    // Claude Code entries are tree nodes keyed by uuid/parentUuid/sessionId.
    let looks_like_claude = object.contains_key("uuid")
        || object.contains_key("parentUuid")
        || object.contains_key("sessionId")
        || matches!(
            object.get("type").and_then(Value::as_str),
            Some("user" | "assistant" | "summary" | "system")
        );
    if looks_like_claude {
        return Some(AgentKind::ClaudeCode);
    }

    // Pi v1 files may start with a message entry rather than a header.
    if object.contains_key("message") && object.contains_key("id") {
        return Some(AgentKind::Pi);
    }
    None
}

/// Parse a session file, detecting the provider from its content.
pub fn parse_path(path: &Path, max_bad_line_rate: f64) -> Result<Session> {
    let source = source::read(path)?;
    let first = source
        .lines
        .iter()
        .find(|line| !line.trim().is_empty())
        .ok_or_else(|| Error::UnknownFormat(path.to_path_buf()))?;
    let agent = detect(first).ok_or_else(|| Error::UnknownFormat(path.to_path_buf()))?;
    parse_as(agent, source, max_bad_line_rate)
}

/// Parse a session file as a known provider.
pub fn parse_as(
    agent: AgentKind,
    source: source::SourceText,
    max_bad_line_rate: f64,
) -> Result<Session> {
    let session = match agent {
        AgentKind::ClaudeCode => claude_code::parse(source),
        AgentKind::Codex => codex::parse(source),
        AgentKind::Pi => pi::parse(source),
    }?;
    check_bad_line_rate(&session, max_bad_line_rate)?;
    Ok(session)
}

/// Parse a session together with its separate subagent transcript files.
///
/// Only Claude Code stores subagents separately today; other providers ignore
/// `sidechains` and behave exactly like [`parse_as`].
pub fn parse_with_sidechains(
    agent: AgentKind,
    source: source::SourceText,
    sidechains: Vec<source::SourceText>,
    max_bad_line_rate: f64,
) -> Result<Session> {
    let session = match agent {
        AgentKind::ClaudeCode => claude_code::parse_with_sidechains(source, sidechains),
        AgentKind::Codex => codex::parse(source),
        AgentKind::Pi => pi::parse(source),
    }?;
    check_bad_line_rate(&session, max_bad_line_rate)?;
    Ok(session)
}

fn check_bad_line_rate(session: &Session, max: f64) -> Result<()> {
    let bad = session
        .diagnostics
        .iter()
        .filter(|d| matches!(d, Diagnostic::BadLine { .. }))
        .count();
    if bad == 0 {
        return Ok(());
    }
    let total = session.events.len().max(bad);
    let rate = bad as f64 / total as f64;
    if rate > max {
        let path = session.source_paths.first().cloned().unwrap_or_default();
        return Err(Error::ParseFailureRate {
            path,
            bad,
            total,
            rate: rate * 100.0,
            max: max * 100.0,
        });
    }
    Ok(())
}

/// Shared accumulator every adapter builds its [`Session`] with.
///
/// Owning event construction here keeps `events[i].idx == i` and the
/// diagnostics bookkeeping identical across providers.
pub(crate) struct SessionBuilder {
    agent: AgentKind,
    pub(crate) id: String,
    pub(crate) meta: SessionMeta,
    events: Vec<Event>,
    diagnostics: Vec<Diagnostic>,
    unknown_kinds: BTreeMap<String, usize>,
    native_compactions: Vec<NativeCompaction>,
    models: Vec<String>,
}

impl SessionBuilder {
    pub(crate) fn new(agent: AgentKind) -> Self {
        Self {
            agent,
            id: String::new(),
            meta: SessionMeta::default(),
            events: Vec::new(),
            diagnostics: Vec::new(),
            unknown_kinds: BTreeMap::new(),
            native_compactions: Vec::new(),
            models: Vec::new(),
        }
    }

    pub(crate) fn agent(&self) -> AgentKind {
        self.agent
    }

    /// Append an event and return its index.
    pub(crate) fn push(
        &mut self,
        kind: EventKind,
        native_id: Option<String>,
        parent: Option<String>,
        ts: Option<String>,
        stream: u32,
        line: LineRef,
    ) -> EventIdx {
        let idx = self.events.len() as EventIdx;
        if let EventKind::NativeCompactionSummary { text } = &kind {
            self.native_compactions.push(NativeCompaction {
                evt: idx,
                summary: Some(text.clone()),
            });
        }
        if let EventKind::System { subtype, .. } = &kind
            && subtype == "native_compaction"
        {
            self.native_compactions.push(NativeCompaction {
                evt: idx,
                summary: None,
            });
        }
        self.events.push(Event {
            idx,
            native_id,
            parent,
            ts,
            stream,
            kind,
            line,
        });
        idx
    }

    pub(crate) fn note_bad_line(&mut self, line: LineRef, message: impl Into<String>) {
        self.diagnostics.push(Diagnostic::BadLine {
            path: line.path,
            line: line.line,
            message: message.into(),
        });
    }

    pub(crate) fn note_unknown_kind(&mut self, kind: impl Into<String>) {
        *self.unknown_kinds.entry(kind.into()).or_insert(0) += 1;
    }

    pub(crate) fn note(&mut self, diagnostic: Diagnostic) {
        self.diagnostics.push(diagnostic);
    }

    pub(crate) fn note_model(&mut self, model: impl Into<String>) {
        let model = model.into();
        if !model.is_empty() && !self.models.contains(&model) {
            self.models.push(model);
        }
    }

    pub(crate) fn events(&self) -> &[Event] {
        &self.events
    }

    /// Finish, recording the active branch and the timestamps observed.
    pub(crate) fn finish(
        mut self,
        active: Vec<EventIdx>,
        sources: Vec<source::SourceText>,
    ) -> Session {
        for (kind, count) in std::mem::take(&mut self.unknown_kinds) {
            self.diagnostics
                .push(Diagnostic::UnknownLineKind { kind, count });
        }
        self.models.sort();
        self.meta.models = std::mem::take(&mut self.models);

        let timestamps: Vec<&String> = self.events.iter().filter_map(|e| e.ts.as_ref()).collect();
        if self.meta.started_at.is_none() {
            self.meta.started_at = timestamps.iter().min().map(|ts| (*ts).clone());
        }
        self.meta.ended_at = timestamps.iter().max().map(|ts| (*ts).clone());

        let hash =
            source::combined_hash(&sources.iter().map(|s| s.hash.clone()).collect::<Vec<_>>());
        Session {
            agent: self.agent,
            id: self.id,
            source_paths: sources.into_iter().map(|s| s.path).collect(),
            source_hash: hash,
            meta: self.meta,
            events: self.events,
            active,
            native_compactions: self.native_compactions,
            diagnostics: self.diagnostics,
        }
    }
}

// ---- JSON projection helpers ------------------------------------------------
// Provider formats are decoded as `serde_json::Value` and projected into a
// tolerant view (spec §2.2 decision D-2), so unknown fields cost nothing.

/// A string field.
pub(crate) fn str_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

/// A string field, owned.
pub(crate) fn string_field(value: &Value, key: &str) -> Option<String> {
    str_field(value, key).map(str::to_string)
}

/// The first present string field among `keys`.
pub(crate) fn first_str_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| string_field(value, key))
}

/// A boolean field, defaulting to `false`.
pub(crate) fn bool_field(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// An integer field, narrowed to `i32`.
pub(crate) fn i32_field(value: &Value, key: &str) -> Option<i32> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .and_then(|n| i32::try_from(n).ok())
}

/// Render a message `content` field, which providers write either as a plain
/// string or as a list of typed blocks.
pub(crate) fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        // A single typed block, e.g. `{"type":"text","text":"..."}`. Reading
        // the `text` field keeps block JSON out of the transcript.
        Value::Object(_) => str_field(value, "text")
            .or_else(|| str_field(value, "thinking"))
            .map(str::to_string)
            .unwrap_or_default(),
        Value::Array(blocks) => {
            let mut out = String::new();
            for block in blocks {
                let text = match block {
                    Value::String(text) => Some(text.clone()),
                    Value::Object(_) => str_field(block, "text").map(str::to_string),
                    _ => None,
                };
                if let Some(text) = text {
                    if !out.is_empty() {
                        out.push('\n');
                    }
                    out.push_str(&text);
                }
            }
            out
        }
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// True when user-role text is a harness-injected fragment rather than
/// something a human typed as a request.
///
/// Every provider injects context, instructions, command echoes, and tool
/// scaffolding as user-role messages. They are still transcript — they are
/// never dropped — but treating them as stated intent turns `/login` or
/// `<environment_context>` into the session's goal. The shared rule is
/// structural: real requests are prose, injected fragments open with a tag.
pub(crate) fn looks_like_harness_fragment(text: &str, extra_markers: &[&str]) -> bool {
    let trimmed = text.trim_start();

    // `get` rather than a slice: a marker length can land inside a multi-byte
    // character, and session files are untrusted input.
    if extra_markers.iter().any(|marker| {
        trimmed
            .get(..marker.len())
            .is_some_and(|prefix| prefix.eq_ignore_ascii_case(marker))
    }) {
        return true;
    }

    // An opening XML-ish tag at position 0: `<environment_context>`,
    // `<command-name>`, `<recommended_plugins>`, `<system-reminder>`, …
    let Some(rest) = trimmed.strip_prefix('<') else {
        return false;
    };
    let name: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    if name.is_empty() {
        return false;
    }
    let after = rest.get(name.len()..).unwrap_or_default();
    // A tag, and the fragment closes it somewhere: `<x>…</x>` or `<x …>`.
    (after.starts_with('>') || after.starts_with(' ') || after.starts_with('\n'))
        && trimmed.contains(&format!("</{name}>"))
}

/// Render an arbitrary tool output value as text.
pub(crate) fn output_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Null => String::new(),
        Value::Array(_) | Value::Object(_) => {
            let rendered = content_text(value);
            if rendered.is_empty() {
                value.to_string()
            } else {
                rendered
            }
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_provider_from_its_first_line() {
        assert_eq!(
            detect(r#"{"type":"session","version":3,"id":"abc"}"#),
            Some(AgentKind::Pi)
        );
        assert_eq!(
            detect(r#"{"timestamp":"2026-01-01T00:00:00Z","type":"session_meta","payload":{}}"#),
            Some(AgentKind::Codex)
        );
        assert_eq!(
            detect(r#"{"type":"user","uuid":"u1","parentUuid":null,"sessionId":"s"}"#),
            Some(AgentKind::ClaudeCode)
        );
    }

    #[test]
    fn unrecognized_content_is_not_guessed() {
        assert_eq!(detect("not json"), None);
        assert_eq!(detect(r#"{"hello":"world"}"#), None);
    }

    #[test]
    fn content_blocks_and_plain_strings_both_render() {
        assert_eq!(content_text(&serde_json::json!("hi")), "hi");
        assert_eq!(
            content_text(
                &serde_json::json!([{"type":"text","text":"a"},{"type":"text","text":"b"}])
            ),
            "a\nb"
        );
        assert_eq!(content_text(&Value::Null), "");
    }

    #[test]
    fn tag_wrapped_user_text_is_recognized_as_a_harness_fragment() {
        for text in [
            "<environment_context>\ncwd=/x\n</environment_context>",
            "<command-name>/login</command-name>",
            "<recommended_plugins>a b c</recommended_plugins>",
            "  <system-reminder>keep going</system-reminder>",
        ] {
            assert!(looks_like_harness_fragment(text, &[]), "{text}");
        }
    }

    #[test]
    fn prose_is_never_mistaken_for_a_harness_fragment() {
        for text in [
            "implement the manifest loader",
            // A user talking about markup, with no closing tag.
            "the <div> in the template is broken",
            "<not a tag",
            "\u{2014} start here",
            "",
        ] {
            assert!(!looks_like_harness_fragment(text, &[]), "{text}");
        }
    }

    #[test]
    fn extra_markers_cover_fragments_that_are_not_tags() {
        assert!(looks_like_harness_fragment(
            "# AGENTS.md instructions\n…",
            &["# AGENTS.md instructions"]
        ));
        assert!(!looks_like_harness_fragment(
            "# AGENTS.md instructions\n…",
            &[]
        ));
    }

    #[test]
    fn a_multibyte_character_at_a_marker_boundary_does_not_panic() {
        for text in [
            "\u{273b} refused \u{2014} a firewall",
            "\u{1f600}",
            "<\u{2014}",
        ] {
            assert!(
                !looks_like_harness_fragment(text, &["<environment_context>"]),
                "{text}"
            );
        }
    }

    #[test]
    fn output_text_falls_back_to_json_for_structured_results() {
        let value = serde_json::json!({"stdout": "ok"});
        assert_eq!(output_text(&value), value.to_string());
    }
}
