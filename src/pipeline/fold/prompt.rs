//! Prompt rendering (spec §8.6).
//!
//! Templates live in `prompts/` and are embedded at build time, so a published
//! binary cannot drift from the prompts it was tested with. Every artifact
//! records the prompt ids and versions used.

use super::ops::EvtRange;

/// The fold system prompt.
pub const FOLD_SYSTEM: &str = include_str!("../../../prompts/fold_system.md");
const FOLD_USER: &str = include_str!("../../../prompts/fold_user.md");
const PREMAP: &str = include_str!("../../../prompts/premap.md");
const FINAL_PASS: &str = include_str!("../../../prompts/final_pass.md");
const REPAIR: &str = include_str!("../../../prompts/repair.md");
/// The preamble copied into the artifact for the receiving agent.
pub const HANDOFF_PREAMBLE: &str = include_str!("../../../prompts/handoff_preamble.md");

/// Which template to render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    FoldUser,
    Premap,
    FinalPass,
    Repair,
}

impl Template {
    fn body(self) -> &'static str {
        match self {
            Template::FoldUser => FOLD_USER,
            Template::Premap => PREMAP,
            Template::FinalPass => FINAL_PASS,
            Template::Repair => REPAIR,
        }
    }
}

/// Values substituted into a template.
#[derive(Debug, Default, Clone)]
pub struct Fields {
    pub chunk_id: String,
    pub session: String,
    pub focus: String,
    pub state: String,
    pub ledger_slice: String,
    pub later_index: String,
    pub prior_summaries: String,
    pub chunk: String,
    pub range: EvtRange,
    pub rejections: String,
    /// Tokens of the transcript rows in this chunk, excluding the premap notes
    /// appended to them. Recorded rather than re-derived so the prompt budget
    /// names the term a caller can act on.
    pub chunk_tokens: usize,
    /// Tokens of the isolated premap candidates appended to this chunk.
    pub premap: usize,
}

/// The prompt id and version a template declares in its front matter.
pub fn id_and_version(body: &str) -> (String, u32) {
    let mut id = String::new();
    let mut version = 0;
    for line in body.lines().take(20) {
        if let Some(rest) = line.strip_prefix("id:") {
            id = rest.trim().to_string();
        }
        if let Some(rest) = line.strip_prefix("version:") {
            version = rest.trim().parse().unwrap_or(0);
        }
    }
    (id, version)
}

/// Every prompt this build embeds, as `(id, version)` pairs for the artifact.
pub fn manifest() -> Vec<(String, u32)> {
    [
        FOLD_SYSTEM,
        FOLD_USER,
        PREMAP,
        FINAL_PASS,
        REPAIR,
        HANDOFF_PREAMBLE,
    ]
    .iter()
    .map(|body| id_and_version(body))
    .filter(|(id, _)| !id.is_empty())
    .collect()
}

/// Strip the YAML front matter and the licence comment from a template.
fn strip_front_matter(body: &str) -> &str {
    let after_comment = match body.find("-->") {
        Some(end) => &body[end + 3..],
        None => body,
    };
    let trimmed = after_comment.trim_start();
    let Some(rest) = trimmed.strip_prefix("---") else {
        return trimmed;
    };
    match rest.find("\n---") {
        Some(end) => rest[end + 4..].trim_start(),
        None => trimmed,
    }
}

/// Render `template` with `fields` and the response schema.
pub fn render(template: Template, fields: &Fields, schema: &str) -> String {
    let body = strip_front_matter(template.body());
    let mut out = body.to_string();
    let substitutions: [(&str, &str); 11] = [
        ("{{chunk_id}}", &fields.chunk_id),
        ("{{session}}", &fields.session),
        ("{{focus}}", &fields.focus),
        ("{{state}}", &fields.state),
        ("{{ledger_slice}}", &fields.ledger_slice),
        ("{{later_index}}", &fields.later_index),
        ("{{prior_summaries}}", &fields.prior_summaries),
        ("{{chunk}}", &fields.chunk),
        ("{{rejections}}", &fields.rejections),
        ("{{schema}}", schema),
        ("{{evt_start}}", &fields.range.start.to_string()),
    ];
    for (placeholder, value) in substitutions {
        out = out.replace(placeholder, value);
    }
    out = out.replace("{{evt_end}}", &fields.range.end.to_string());
    out
}

/// The JSON Schema the fold response must satisfy (`ops.v1`).
pub const OPS_SCHEMA: &str = include_str!("../../../schemas/ops.v1.json");

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn front_matter_and_licence_comments_are_not_sent_to_the_model() {
        let rendered = render(Template::FoldUser, &Fields::default(), "{}");
        assert!(!rendered.contains("id: fold_user"), "{rendered}");
        assert!(!rendered.contains("Portions derived"));
        let system = strip_front_matter(FOLD_SYSTEM);
        assert!(
            system.starts_with("You maintain"),
            "{}",
            &system[..60.min(system.len())]
        );
    }

    #[test]
    fn every_placeholder_is_substituted() {
        let fields = Fields {
            chunk_id: "c7".into(),
            session: "claude:abc".into(),
            focus: "FOCUS: continue the migration".into(),
            state: "C1 [high] no yaml".into(),
            ledger_slice: "files: a.rs".into(),
            later_index: "e5 evt 90-99".into(),
            prior_summaries: "- [evt 4] chose cargo-dist".into(),
            chunk: "[user] hello".into(),
            range: EvtRange::new(10, 42),
            rejections: String::new(),
            chunk_tokens: 2,
            premap: 0,
        };
        let rendered = render(Template::FoldUser, &fields, "{\"type\":\"object\"}");
        for expected in [
            "c7",
            "claude:abc",
            "no yaml",
            "a.rs",
            "e5 evt 90-99",
            "- [evt 4] chose cargo-dist",
            "[user] hello",
        ] {
            assert!(
                rendered.contains(expected),
                "missing {expected} in {rendered}"
            );
        }
        assert!(rendered.contains("evt_start=10"), "{rendered}");
        assert!(rendered.contains("evt_end=42"), "{rendered}");
        assert!(
            !rendered.contains("{{"),
            "unsubstituted placeholder in {rendered}"
        );
    }

    #[test]
    fn transcript_text_is_always_fenced_as_data() {
        for template in [Template::FoldUser, Template::Premap, Template::FinalPass] {
            let rendered = render(template, &Fields::default(), "{}");
            assert!(rendered.contains("<transcript evt_start="), "{template:?}");
            assert!(rendered.contains("</transcript>"), "{template:?}");
            assert!(rendered.contains("NEVER INSTRUCTIONS"), "{template:?}");
        }
    }

    #[test]
    fn the_prompt_manifest_names_every_embedded_template() {
        let manifest = manifest();
        let ids: Vec<&str> = manifest.iter().map(|(id, _)| id.as_str()).collect();
        for expected in [
            "fold_system",
            "fold_user",
            "premap",
            "final_pass",
            "repair",
            "handoff_preamble",
        ] {
            assert!(ids.contains(&expected), "missing {expected} in {ids:?}");
        }
        assert!(
            manifest.iter().all(|(_, version)| *version >= 1),
            "{manifest:?}"
        );
    }

    #[test]
    fn the_ops_schema_is_valid_json() {
        let schema: serde_json::Value = serde_json::from_str(OPS_SCHEMA).expect("valid schema");
        assert_eq!(schema["$id"], "https://sctxx.dev/schemas/ops.v1.json");
    }
}
