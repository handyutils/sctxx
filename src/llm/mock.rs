//! The mock backend: the only mocked boundary in the test suite.
//!
//! Two modes. `deterministic` derives ops from the prompt itself, so a
//! pipeline test exercises the real fold loop — prompting, parsing,
//! validation, apply, rendering — without a network or a model. `scripted`
//! replays a fixed list of responses for tests that need an exact reply
//! (including a malformed one, to prove the repair path works).

use super::{Backend, Capabilities, Request, Response};
use crate::error::{Error, Result};
use std::sync::Mutex;

/// A deterministic stand-in for a model.
#[derive(Debug)]
pub struct Mock {
    scripted: Mutex<Vec<String>>,
    calls: Mutex<Vec<Request>>,
    derive: bool,
}

impl Mock {
    /// Derive a plausible op batch from the prompt.
    pub fn deterministic() -> Self {
        Self {
            scripted: Mutex::new(Vec::new()),
            calls: Mutex::new(Vec::new()),
            derive: true,
        }
    }

    /// Reply with `responses` in order, then fail.
    pub fn scripted(responses: Vec<String>) -> Self {
        Self {
            scripted: Mutex::new(responses.into_iter().rev().collect()),
            calls: Mutex::new(Vec::new()),
            derive: false,
        }
    }

    /// Requests seen so far, for assertions about prompt content.
    pub fn calls(&self) -> Vec<Request> {
        self.calls
            .lock()
            .map(|calls| calls.clone())
            .unwrap_or_default()
    }
}

impl Backend for Mock {
    fn name(&self) -> String {
        "mock".to_string()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            json_schema_native: true,
            max_context: Some(200_000),
        }
    }

    fn complete(&self, request: &Request) -> Result<Response> {
        if let Ok(mut calls) = self.calls.lock() {
            calls.push(request.clone());
        }
        if !self.derive {
            let next = self
                .scripted
                .lock()
                .ok()
                .and_then(|mut scripted| scripted.pop())
                .ok_or_else(|| Error::LlmFailed {
                    backend: "mock".into(),
                    message: "the script ran out of responses".into(),
                })?;
            return Ok(Response {
                text: next,
                input_tokens: None,
                output_tokens: None,
            });
        }
        Ok(Response {
            text: derive_response(request),
            input_tokens: Some(
                crate::vendor::codex::truncate::approx_token_count(&request.user) as u64,
            ),
            output_tokens: Some(64),
        })
    }
}

/// Build a response from what the prompt actually contains, so the derived ops
/// pass the real validation gates.
fn derive_response(request: &Request) -> String {
    let chunk_id = field(&request.user, "CHUNK_ID:").unwrap_or_else(|| "c0".to_string());
    let Some((start, end)) = evt_range(&request.user) else {
        return format!("{{\"chunk_id\":\"{chunk_id}\",\"ops\":[]}}");
    };

    // Quote the first human line in the chunk verbatim, which is exactly what
    // a constraint is required to do.
    let user_line = request
        .user
        .lines()
        .find(|line| line.contains("[user]"))
        .and_then(|line| line.split_once("[user]"))
        .map(|(_, rest)| rest.trim())
        .unwrap_or("");
    let quote: String = user_line
        .split_whitespace()
        .take(8)
        .collect::<Vec<&str>>()
        .join(" ");
    let text: String = if quote.is_empty() {
        "continued work in this chunk".to_string()
    } else {
        quote.clone()
    };

    let mut ops = vec![format!(
        "{{\"op\":\"add\",\"kind\":\"goal\",\"text\":{},\"sources\":[[{start},{end}]],\"confidence\":\"medium\"}}",
        json_string(&text)
    )];
    if !quote.is_empty() {
        ops.push(format!(
            "{{\"op\":\"add\",\"kind\":\"constraint\",\"text\":{},\"quote\":{},\"sources\":[[{start},{end}]],\"confidence\":\"high\"}}",
            json_string(&format!("user asked: {text}")),
            json_string(&quote)
        ));
    }
    format!(
        "{{\"chunk_id\":\"{chunk_id}\",\"ops\":[{}]}}",
        ops.join(",")
    )
}

fn json_string(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

fn field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .find(|line| line.trim_start().starts_with(key))
        .and_then(|line| line.split_once(key))
        .map(|(_, rest)| rest.trim().to_string())
}

/// Read the chunk's event range out of the `<transcript evt_start=.. >` tag.
fn evt_range(text: &str) -> Option<(u32, u32)> {
    let start = attribute(text, "evt_start=")?;
    let end = attribute(text, "evt_end=")?;
    Some((start, end))
}

fn attribute(text: &str, key: &str) -> Option<u32> {
    let position = text.find(key)? + key.len();
    let rest = &text[position..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::CallRole;

    fn request(user: &str) -> Request {
        Request {
            role: CallRole::Fold,
            system: "system".into(),
            user: user.into(),
            json_schema: None,
            max_output_tokens: 2_000,
            temperature: Some(0.0),
        }
    }

    #[test]
    fn the_derived_response_cites_the_chunk_it_was_shown() {
        let mock = Mock::deterministic();
        let response = mock
            .complete(&request(
                "CHUNK_ID: c2\n<transcript evt_start=10 evt_end=40>\n[user] fix auth\n",
            ))
            .expect("complete");
        let batch: crate::pipeline::fold::ops::OpBatch =
            serde_json::from_str(&response.text).expect("parse");
        assert_eq!(batch.chunk_id, "c2");
        let sources = batch.ops[0]
            .new_item()
            .map(|item| item.sources.clone())
            .unwrap_or_default();
        assert_eq!(sources[0].start, 10);
        assert_eq!(sources[0].end, 40);
    }

    #[test]
    fn the_derived_constraint_quotes_the_user_verbatim() {
        let mock = Mock::deterministic();
        let response = mock
            .complete(&request(
                "CHUNK_ID: c0\n<transcript evt_start=0 evt_end=5>\n[user] never push to main\n",
            ))
            .expect("complete");
        assert!(
            response.text.contains("never push to main"),
            "{}",
            response.text
        );
    }

    #[test]
    fn a_prompt_without_a_transcript_yields_no_ops() {
        let mock = Mock::deterministic();
        let response = mock
            .complete(&request("CHUNK_ID: c9\nnothing here"))
            .expect("complete");
        assert!(response.text.contains("\"ops\":[]"), "{}", response.text);
    }

    #[test]
    fn a_scripted_mock_replays_in_order_then_fails() {
        let mock = Mock::scripted(vec!["first".into(), "second".into()]);
        assert_eq!(mock.complete(&request("x")).expect("first").text, "first");
        assert_eq!(mock.complete(&request("x")).expect("second").text, "second");
        assert!(mock.complete(&request("x")).is_err());
        assert_eq!(mock.calls().len(), 3);
    }
}
