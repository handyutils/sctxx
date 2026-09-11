//! Reading an artifact back: what it came from, and the evidence behind a pointer.
//!
//! Two callers need this — `sctxx expand` and the TUI's canvas — and they must
//! agree, because FR-016 requires that following an `[evt a–b]` pointer in the
//! canvas goes through *the same* path as `sctxx expand` rather than a second
//! implementation of it.

use crate::ir::{EventIdx, Session};
use crate::pipeline::mask;
use std::path::{Path, PathBuf};

/// The session reference an artifact was written from.
///
/// An artifact knows its own provenance, which is what lets `sctxx expand
/// .sctxx/` work without the developer repeating the reference — and what lets
/// the canvas follow a pointer in an artifact somebody else produced.
pub fn source_reference(candidate: &Path) -> Option<String> {
    if !candidate.exists() {
        return None;
    }
    let file = artifact_file(candidate);
    let body = std::fs::read_to_string(&file).ok()?;

    if file
        .extension()
        .is_some_and(|extension| extension == "json")
    {
        let value: serde_json::Value = serde_json::from_str(&body).ok()?;
        let agent = value["session"]["agent"].as_str()?;
        let id = value["session"]["id"].as_str()?;
        return Some(format!("{agent}:{id}"));
    }

    // The markdown front matter carries `source: {agent: …, session: …}`.
    let line = body
        .lines()
        .take(30)
        .find(|line| line.starts_with("source:"))?;
    let agent = front_matter_field(line, "agent:")?;
    let session = front_matter_field(line, "session:")?;
    Some(format!("{agent}:{session}"))
}

/// The file inside an artifact directory: the JSON when it is there, the
/// markdown otherwise.
pub fn artifact_file(candidate: &Path) -> PathBuf {
    if !candidate.is_dir() {
        return candidate.to_path_buf();
    }
    let json = candidate.join("handoff.json");
    if json.is_file() {
        return json;
    }
    candidate.join("handoff.md")
}

/// The masked rows covering each range, with a header per range.
///
/// The text is what `sctxx expand` prints and what the canvas shows, so the
/// reader sees the same evidence either way.
pub fn expand_ranges(
    session: &Session,
    ranges: &[(EventIdx, EventIdx)],
    context: EventIdx,
) -> String {
    let rows = mask::build(session, &mask::MaskOptions::default());
    let mut out = String::new();
    for (start, end) in ranges {
        let start = start.saturating_sub(context);
        let end = end.saturating_add(context);
        let selected: Vec<mask::Row> = rows
            .iter()
            .filter(|row| (start..=end).contains(&row.evt))
            .cloned()
            .collect();
        out.push_str(&format!("=== evt {start}–{end} ===\n"));
        if selected.is_empty() {
            out.push_str("(no rows in this range; it may fall outside the active branch)\n");
        } else {
            out.push_str(&mask::render(&selected));
        }
    }
    out
}

/// Read one `key: value` out of an artifact's markdown front matter.
fn front_matter_field(line: &str, key: &str) -> Option<String> {
    let after = line.split(key).nth(1)?;
    let value = after.trim_start().split([',', '}']).next()?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact_dir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    #[test]
    fn the_source_reference_comes_out_of_the_markdown_front_matter() {
        let dir = artifact_dir();
        std::fs::write(
            dir.path().join("handoff.md"),
            "# Handoff\n\n---\nsource: {agent: claude, session: 1367d688-7dcd}\n---\n\n## L0 · Brief\n",
        )
        .expect("write");
        assert_eq!(
            source_reference(dir.path()).as_deref(),
            Some("claude:1367d688-7dcd")
        );
    }

    #[test]
    fn the_json_is_preferred_when_both_are_present() {
        // `handoff.json` is precise; the markdown front matter is a summary of
        // it, so the JSON wins where it exists.
        let dir = artifact_dir();
        std::fs::write(
            dir.path().join("handoff.md"),
            "source: {agent: claude, session: from-markdown}\n",
        )
        .expect("write");
        std::fs::write(
            dir.path().join("handoff.json"),
            r#"{"session":{"agent":"codex","id":"01a04823"}}"#,
        )
        .expect("write");
        assert_eq!(
            source_reference(dir.path()).as_deref(),
            Some("codex:01a04823")
        );
    }

    #[test]
    fn a_bare_markdown_file_is_readable_too() {
        let dir = artifact_dir();
        let file = dir.path().join("colleague-handoff.md");
        std::fs::write(&file, "source: {agent: pi, session: abc123}\n").expect("write");
        assert_eq!(source_reference(&file).as_deref(), Some("pi:abc123"));
    }

    #[test]
    fn an_artifact_with_no_source_is_not_a_guess() {
        let dir = artifact_dir();
        std::fs::write(dir.path().join("handoff.md"), "# no front matter here\n").expect("write");
        assert_eq!(source_reference(dir.path()), None);
        assert_eq!(source_reference(&dir.path().join("absent.md")), None);
    }
}
