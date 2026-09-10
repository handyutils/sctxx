//! Recovering JSON from a model that almost produced it (spec §9.1).
//!
//! Models fence their JSON, add a sentence before it, or trail a comma. One
//! cheap repair pass here saves a round trip; anything it cannot fix goes back
//! to the model with the parse error.

use serde_json::Value;

/// Extract and parse the JSON object in `text`, repairing common damage.
pub fn parse_object(text: &str) -> Result<Value, String> {
    let candidate = extract(text);
    match serde_json::from_str::<Value>(&candidate) {
        Ok(value) => Ok(value),
        Err(first) => {
            let repaired = repair(&candidate);
            serde_json::from_str::<Value>(&repaired)
                .map_err(|second| format!("{first}; after repair: {second}"))
        }
    }
}

/// Strip prose and code fences, leaving the outermost JSON object.
fn extract(text: &str) -> String {
    let text = text.trim();
    // ```json ... ``` or ``` ... ```
    let unfenced = match text.find("```") {
        Some(start) => {
            let after = &text[start + 3..];
            let after = after.strip_prefix("json").unwrap_or(after);
            let after = after.trim_start_matches(['\n', '\r']);
            match after.find("```") {
                Some(end) => &after[..end],
                None => after,
            }
        }
        None => text,
    };

    // The outermost braces.
    match (unfenced.find('{'), unfenced.rfind('}')) {
        (Some(start), Some(end)) if end > start => unfenced[start..=end].to_string(),
        _ => unfenced.trim().to_string(),
    }
}

/// Remove trailing commas and balance unclosed braces and brackets.
fn repair(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 8);
    let mut stack: Vec<char> = Vec::new();
    let mut in_string = false;
    let mut escaped = false;

    for ch in text.chars() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => {
                in_string = true;
                out.push(ch);
            }
            '{' | '[' => {
                stack.push(ch);
                out.push(ch);
            }
            '}' | ']' => {
                // Drop a comma that now precedes a closer.
                trim_trailing_comma(&mut out);
                stack.pop();
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    if in_string {
        out.push('"');
    }
    trim_trailing_comma(&mut out);
    while let Some(open) = stack.pop() {
        out.push(if open == '{' { '}' } else { ']' });
    }
    out
}

fn trim_trailing_comma(out: &mut String) {
    let trimmed = out.trim_end();
    if trimmed.ends_with(',') {
        let keep = trimmed.len() - 1;
        out.truncate(keep);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_json_parses_unchanged() {
        let value = parse_object(r#"{"chunk_id":"c0","ops":[]}"#).expect("parse");
        assert_eq!(value["chunk_id"], "c0");
    }

    #[test]
    fn fenced_json_with_a_preamble_parses() {
        let text = "Here are the ops:\n```json\n{\"chunk_id\":\"c1\",\"ops\":[]}\n```\nDone.";
        assert_eq!(parse_object(text).expect("parse")["chunk_id"], "c1");
    }

    #[test]
    fn a_trailing_comma_is_repaired() {
        let text = r#"{"chunk_id":"c2","ops":[{"op":"confirm","id":"C1","evt":3},],}"#;
        let value = parse_object(text).expect("parse");
        assert_eq!(value["ops"].as_array().map(Vec::len), Some(1));
    }

    #[test]
    fn a_truncated_response_is_closed() {
        let text = r#"{"chunk_id":"c3","ops":[{"op":"resolve","id":"O1","evt":9}"#;
        let value = parse_object(text).expect("parse");
        assert_eq!(value["ops"][0]["id"], "O1");
    }

    #[test]
    fn braces_inside_strings_are_not_treated_as_structure() {
        let text = r#"{"chunk_id":"c4","ops":[],"note":"a } and a \" quote"}"#;
        assert_eq!(
            parse_object(text).expect("parse")["note"],
            "a } and a \" quote"
        );
    }

    #[test]
    fn hopeless_output_reports_both_parse_errors() {
        let error = parse_object("I cannot help with that.").expect_err("should fail");
        assert!(error.contains("after repair"), "{error}");
    }
}
