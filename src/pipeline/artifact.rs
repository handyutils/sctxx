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

/// How a range is delivered back to the reader.
#[derive(Debug, Clone, Copy)]
pub enum Delivery {
    /// Everything the range covers. The canvas wants this.
    Whole,
    /// One page of an exactly partitioned range, and how many pages there are.
    ///
    /// Recovery is paged rather than windowed, which is the point: a head/tail
    /// window over a recovered body irreversibly discards its interior, so a
    /// reader who needs events 4400–4410 of a 600-event range cannot ask for
    /// them. Every page here is an exact, non-overlapping slice of the range, so
    /// the union of the pages is the range and nothing is unreachable.
    Page {
        /// 1-based.
        index: usize,
        /// Token ceiling per page.
        tokens: usize,
    },
}

impl Delivery {
    /// Which page, and how big, if this is a paged delivery.
    fn page(self) -> Option<(usize, usize)> {
        match self {
            Delivery::Whole => None,
            Delivery::Page { index, tokens } => Some((index.max(1), tokens.max(1))),
        }
    }
}

/// The masked rows covering each range, with a header per range.
///
/// The text is what `sctxx expand` prints and what the canvas shows, so the
/// reader sees the same evidence either way.
pub fn expand_ranges(
    session: &Session,
    ranges: &[(EventIdx, EventIdx)],
    context: EventIdx,
    delivery: Delivery,
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
        match delivery.page() {
            None => {
                out.push_str(&format!("=== evt {start}–{end} ===\n"));
                if selected.is_empty() {
                    out.push_str(
                        "(no rows in this range; it may fall outside the active branch)\n",
                    );
                } else {
                    out.push_str(&mask::render(&selected));
                }
            }
            Some((index, tokens)) => {
                out.push_str(&page_of(&selected, start, end, index, tokens));
            }
        }
    }
    out
}

/// Pack the rows into exact consecutive pages and render one of them.
fn page_of(
    rows: &[mask::Row],
    start: EventIdx,
    end: EventIdx,
    index: usize,
    tokens: usize,
) -> String {
    let pages = paginate(rows, tokens);
    let count = pages.len().max(1);
    let mut out = String::new();
    if rows.is_empty() {
        out.push_str(&format!("=== evt {start}–{end} ===\n"));
        out.push_str("(no rows in this range; it may fall outside the active branch)\n");
        return out;
    }
    let index = index.min(count);
    let range = &pages[index - 1];
    let slice = &rows[range.clone()];
    out.push_str(&format!(
        "=== evt {start}–{end} · page {index}/{count} · evt {}–{} · {} token(s) ===\n",
        slice.first().map(|row| row.evt).unwrap_or(start),
        slice.last().map(|row| row.evt).unwrap_or(end),
        slice.iter().map(|row| row.tokens).sum::<usize>(),
    ));
    out.push_str(&mask::render(slice));
    if count > 1 {
        let next = if index == count { 1 } else { index + 1 };
        out.push_str(&format!(
            "\n--- page {index} of {count}; page {} is `sctxx expand <ref> {start}..{end} --page {next}` ---\n",
            next
        ));
    }
    out
}

/// Split rows into consecutive, non-overlapping pages of at most `tokens` each.
///
/// A single row larger than the ceiling is its own page rather than being
/// dropped or cut: the ceiling is a delivery size, not a content filter.
fn paginate(rows: &[mask::Row], tokens: usize) -> Vec<std::ops::Range<usize>> {
    let mut pages: Vec<std::ops::Range<usize>> = Vec::new();
    let mut start = 0;
    let mut used = 0;
    for (index, row) in rows.iter().enumerate() {
        if index > start && used + row.tokens > tokens {
            pages.push(start..index);
            start = index;
            used = 0;
        }
        used += row.tokens;
    }
    pages.push(start..rows.len());
    pages
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

    fn rows(tokens: &[usize]) -> Vec<mask::Row> {
        tokens
            .iter()
            .enumerate()
            .map(|(index, tokens)| mask::Row {
                evt: index as EventIdx,
                tier: crate::vendor::codex::tiered_input::Tier::User,
                text: format!("[user] line {index}"),
                tokens: *tokens,
                is_human_turn: true,
            })
            .collect()
    }

    #[test]
    fn pages_partition_the_range_exactly() {
        let rows = rows(&[10, 10, 10, 10, 10]);
        let pages = paginate(&rows, 25);
        assert_eq!(pages, vec![0..2, 2..4, 4..5]);
        // The union is the whole range: nothing is unreachable, which is what a
        // head/tail window over the same rows cannot promise.
        let covered: Vec<usize> = pages.iter().flat_map(|page| page.clone()).collect();
        assert_eq!(covered, (0..rows.len()).collect::<Vec<_>>());
    }

    #[test]
    fn a_row_larger_than_the_ceiling_gets_its_own_page() {
        let rows = rows(&[10, 900, 10]);
        assert_eq!(paginate(&rows, 100), vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn a_page_says_where_the_rest_is() {
        let rows = rows(&[10, 10, 10, 10]);
        let first = page_of(&rows, 0, 3, 1, 20);
        assert!(first.contains("page 1/2"), "{first}");
        assert!(
            first.contains("--page 2"),
            "no way to reach page 2:\n{first}"
        );
        let second = page_of(&rows, 0, 3, 2, 20);
        assert!(second.contains("page 2/2"), "{second}");
        assert!(second.contains("--page 1"), "no way back:\n{second}");
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
