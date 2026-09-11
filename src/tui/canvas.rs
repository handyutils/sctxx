//! The canvas: the artifact itself, read inside the TUI.
//!
//! FR-016's premise is that a developer who has to leave the TUI to read the
//! artifact will not read it — so extraction opens the artifact by itself, at L0,
//! without another keypress. It is a **viewer**, not only a receipt: an artifact
//! from an earlier run, or a colleague's, opens the same way (FR-016a).
//!
//! The layers are the ones the renderer wrote (`## L0 · Brief`, `## L1 · Items`,
//! `## L2 · Recent activity`, `## L3 · Retrieval`), so this reads the artifact
//! rather than re-rendering it: what is on screen is what is on disk, including
//! for an artifact sctxx did not produce. L1 carries the ledgers and L3 the
//! retrieval commands, which is why the reader does not need a second pane for
//! them.
//!
//! This module owns no I/O beyond reading the file, and no session: following a
//! pointer needs a session parse, which belongs on the worker thread. The canvas
//! says what to ask for; `mod.rs` asks.

use crate::error::{Error, Result};
use crate::ir::EventIdx;
use crate::pipeline::artifact;
use std::path::{Path, PathBuf};

/// One layer of an artifact.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// `L0`, `L1`, … as the artifact names it.
    pub name: String,
    /// The heading as written, e.g. `L2 · Recent activity (masked, evt 1–9)`.
    pub heading: String,
    pub lines: Vec<String>,
}

/// A range the reader asked to see, and its answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expansion {
    /// Asked for; the session is being read on the worker thread.
    Pending { label: String },
    /// The rows behind the pointer, exactly as `sctxx expand` would print them.
    Ready {
        label: String,
        lines: Vec<String>,
        scroll: usize,
        cursor: usize,
    },
}

impl Expansion {
    pub fn label(&self) -> &str {
        match self {
            Expansion::Pending { label } | Expansion::Ready { label, .. } => label,
        }
    }
}

/// An artifact, open for reading.
#[derive(Debug, Clone)]
pub struct Canvas {
    /// Where it was read from.
    path: PathBuf,
    layers: Vec<Layer>,
    current: usize,
    /// The top line of the window.
    scroll: usize,
    /// The line the reader is on, which is where `enter` acts. Separate from the
    /// scroll position because a document shorter than the window still has to
    /// be navigable — with the cursor pinned to the top line, a pointer on the
    /// second line of a short artifact can never be reached.
    cursor: usize,
    expansion: Option<Expansion>,
}

impl Canvas {
    /// Read an artifact from a file or a `.sctxx/` directory.
    pub fn load(path: &Path) -> Result<Self> {
        let file = artifact::artifact_file(path);
        let body = std::fs::read_to_string(&file).map_err(|source| Error::io(&file, source))?;
        let layers = split_layers(&body);
        if layers.is_empty() {
            return Err(Error::Usage(format!(
                "{} has no L0 layer, so it is not a handoff artifact sctxx can read",
                file.display()
            )));
        }
        Ok(Self {
            path: file,
            layers,
            current: 0,
            scroll: 0,
            cursor: 0,
            expansion: None,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn layers(&self) -> &[Layer] {
        &self.layers
    }

    pub fn current(&self) -> usize {
        self.current
    }

    pub fn expansion(&self) -> Option<&Expansion> {
        self.expansion.as_ref()
    }

    /// Switch layers by position, clamped. The scroll position is per-layer
    /// reading position, so it starts again at the top.
    pub fn select_layer(&mut self, index: usize) {
        if index < self.layers.len() {
            self.current = index;
            self.scroll = 0;
            self.cursor = 0;
        }
    }

    pub fn step_layer(&mut self, delta: isize) {
        let last = self.layers.len().saturating_sub(1) as isize;
        let next = (self.current as isize + delta).clamp(0, last) as usize;
        self.select_layer(next);
    }

    /// The lines currently on screen: an expansion when one is open, otherwise
    /// the current layer.
    pub fn window(&self, height: usize) -> &[String] {
        let lines = self.all_lines();
        let start = self.active_scroll().min(lines.len());
        let end = (start + height.max(1)).min(lines.len());
        &lines[start..end]
    }

    /// Every line of whatever is being read — the layer, or the expansion on top
    /// of it.
    fn all_lines(&self) -> &[String] {
        match &self.expansion {
            Some(Expansion::Ready { lines, .. }) => lines,
            _ => self
                .layers
                .get(self.current)
                .map(|layer| layer.lines.as_slice())
                .unwrap_or(&[]),
        }
    }

    /// How far down the reader can go before the bottom.
    pub fn max_scroll(&self, height: usize) -> usize {
        self.all_lines().len().saturating_sub(height.max(1))
    }

    /// Move the reader by `delta` lines, scrolling the window to follow.
    pub fn move_cursor(&mut self, delta: isize, height: usize) {
        let len = self.all_lines().len();
        if len == 0 {
            return;
        }
        let next = (self.active_cursor() as isize + delta).clamp(0, len as isize - 1) as usize;
        self.set_active_cursor(next);
        self.keep_cursor_visible(height);
    }

    pub fn move_page(&mut self, down: bool, height: usize) {
        let page = height.max(1) as isize;
        self.move_cursor(if down { page } else { -page }, height);
    }

    pub fn move_to(&mut self, end: bool, height: usize) {
        let len = self.all_lines().len();
        if len == 0 {
            return;
        }
        self.set_active_cursor(if end { len - 1 } else { 0 });
        self.keep_cursor_visible(height);
    }

    /// Where the cursor is inside the visible window, for drawing it.
    pub fn cursor_row(&self, height: usize) -> Option<usize> {
        let cursor = self.active_cursor();
        let scroll = self.active_scroll();
        (cursor >= scroll && cursor < scroll + height.max(1)).then(|| cursor - scroll)
    }

    /// The line the reader is on, which is where `enter` acts.
    pub fn line_at_cursor(&self) -> Option<&str> {
        self.all_lines()
            .get(self.active_cursor())
            .map(String::as_str)
    }

    /// The `[evt a–b]` pointer under the reader, if the line has one.
    pub fn pointer_under_cursor(&self) -> Option<(EventIdx, EventIdx)> {
        pointer_on(self.line_at_cursor()?)
    }

    /// Show that a range is being read.
    pub fn expansion_pending(&mut self, label: String) {
        self.expansion = Some(Expansion::Pending { label });
    }

    /// Scroll the window the least it can to keep the cursor in view, and never
    /// past the end of the text.
    fn keep_cursor_visible(&mut self, height: usize) {
        let height = height.max(1);
        let cursor = self.active_cursor();
        let scroll = self.active_scroll();
        if cursor < scroll {
            self.set_active_scroll(cursor);
        } else if cursor >= scroll + height {
            self.set_active_scroll(cursor + 1 - height);
        }
        let max = self.max_scroll(height);
        let scroll = self.active_scroll().min(max);
        self.set_active_scroll(scroll);
    }

    /// Show the rows behind a pointer.
    pub fn expansion_ready(&mut self, label: String, text: &str) {
        self.expansion = Some(Expansion::Ready {
            label,
            lines: text.lines().map(str::to_string).collect(),
            scroll: 0,
            cursor: 0,
        });
    }

    /// A range that could not be read, so the reader is told rather than shown
    /// a blank pane.
    pub fn expansion_failed(&mut self, label: String, reason: String) {
        self.expansion = Some(Expansion::Ready {
            label,
            lines: vec![format!("could not read this range: {reason}")],
            scroll: 0,
            cursor: 0,
        });
    }

    /// Close the expansion, back to the layer.
    pub fn dismiss_expansion(&mut self) -> bool {
        self.expansion.take().is_some()
    }

    fn active_scroll(&self) -> usize {
        match &self.expansion {
            Some(Expansion::Ready { scroll, .. }) => *scroll,
            _ => self.scroll,
        }
    }

    fn set_active_scroll(&mut self, value: usize) {
        match &mut self.expansion {
            Some(Expansion::Ready { scroll, .. }) => *scroll = value,
            _ => self.scroll = value,
        }
    }

    fn active_cursor(&self) -> usize {
        match &self.expansion {
            // An expansion is a short answer to a specific question, so the
            // reader starts at its first line.
            Some(Expansion::Ready { cursor, .. }) => *cursor,
            _ => self.cursor,
        }
    }

    fn set_active_cursor(&mut self, value: usize) {
        match &mut self.expansion {
            Some(Expansion::Ready { cursor, .. }) => *cursor = value,
            _ => self.cursor = value,
        }
    }
}

/// Split an artifact on the layer headings the renderer writes.
///
/// Anything before the first heading is the front matter and the title, which
/// the canvas shows in its own header rather than as a layer.
pub fn split_layers(body: &str) -> Vec<Layer> {
    let mut layers: Vec<Layer> = Vec::new();
    for line in body.lines() {
        if let Some((name, heading)) = layer_heading(line) {
            layers.push(Layer {
                name,
                heading,
                lines: Vec::new(),
            });
            continue;
        }
        if let Some(layer) = layers.last_mut() {
            layer.lines.push(line.to_string());
        }
    }
    // A layer's heading is not repeated in its body, and a trailing blank line
    // is an artefact of the next heading.
    for layer in &mut layers {
        while layer
            .lines
            .last()
            .is_some_and(|line| line.trim().is_empty())
        {
            layer.lines.pop();
        }
    }
    layers
}

/// `## L0 · Brief` → `("L0", "L0 · Brief")`.
///
/// Only a real layer heading counts: `## Something else` is body text, because
/// an artifact may legitimately contain second-level headings of its own.
fn layer_heading(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("## ")?;
    let name = rest.split(' ').next()?;
    let digits = name.strip_prefix('L')?;
    if digits.is_empty() || !digits.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    Some((name.to_string(), rest.to_string()))
}

/// The `[evt a–b]` pointer in a line.
///
/// The renderer writes an en dash; a hyphen is accepted too, because a
/// hand-edited artifact, or one from an older version, may have one.
pub fn pointer_on(line: &str) -> Option<(EventIdx, EventIdx)> {
    let start = line.find("[evt ")? + "[evt ".len();
    let rest = &line[start..];
    let end = rest.find(']')?;
    let inside = rest[..end].trim();
    let (a, b) = match inside.split_once(['–', '-']) {
        Some((a, b)) => (a.trim(), b.trim()),
        None => (inside, inside),
    };
    let a: EventIdx = a.parse().ok()?;
    let b: EventIdx = b.parse().ok()?;
    Some((a.min(b), a.max(b)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A miniature artifact with the shape the renderer produces.
    const ARTIFACT: &str = "\
# Handoff

---
source: {agent: claude, session: 1367d688}
---

## L0 · Brief

The goal was to stop the parser eating brackets. [evt 41]
Next: run the tests. [evt 41–52]

## L1 · Items

### Files
- `src/lexer.rs` ×3 [evt 12–14]

## L2 · Recent activity (masked, evt 80–99)

```text
some rows  (evt 80)
```

## L3 · Retrieval

sctxx expand .sctxx/ 41..52
";

    fn canvas() -> Canvas {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("handoff.md");
        std::fs::write(&path, ARTIFACT).expect("write");
        // The directory is kept alive by the returned canvas only for reading,
        // so load before it drops.
        Canvas::load(&path).expect("load")
    }

    #[test]
    fn the_layers_are_the_ones_the_artifact_was_rendered_with() {
        let canvas = canvas();
        let names: Vec<&str> = canvas
            .layers()
            .iter()
            .map(|layer| layer.name.as_str())
            .collect();
        assert_eq!(names, vec!["L0", "L1", "L2", "L3"]);
        assert_eq!(canvas.current(), 0, "the reader starts at L0");
        assert!(
            canvas.layers()[0].heading.starts_with("L0 · Brief"),
            "{}",
            canvas.layers()[0].heading
        );
    }

    #[test]
    fn the_front_matter_is_not_a_layer() {
        // `source: {…}` and the title would otherwise be the first thing shown.
        let canvas = canvas();
        assert!(
            !canvas.layers()[0]
                .lines
                .iter()
                .any(|line| line.contains("source:")),
            "front matter belongs in the header, not a layer"
        );
    }

    #[test]
    fn a_second_level_heading_of_its_own_is_body_text() {
        // `### Files` is a heading; `## Something else` would be one too, and
        // must not be mistaken for a layer.
        let canvas = canvas();
        assert!(
            canvas.layers()[1]
                .lines
                .iter()
                .any(|line| line.contains("Files")),
            "L1's own headings stay in its body"
        );
        let split = split_layers("## Notes\n\ntext\n");
        assert!(split.is_empty(), "only L<n> headings start a layer");
    }

    #[test]
    fn a_pointer_is_read_from_a_line_in_either_dash() {
        assert_eq!(pointer_on("the goal [evt 41]"), Some((41, 41)));
        assert_eq!(pointer_on("a range [evt 41–52]"), Some((41, 52)));
        assert_eq!(pointer_on("an older one [evt 41-52]"), Some((41, 52)));
        assert_eq!(pointer_on("reversed [evt 52–41]"), Some((41, 52)));
        assert_eq!(pointer_on("no pointer here"), None);
        assert_eq!(pointer_on("[evt not-a-number]"), None);
        assert_eq!(pointer_on("[evt 41"), None);
    }

    #[test]
    fn the_pointer_under_the_cursor_is_the_line_the_reader_sees() {
        // The cursor is the top visible line, so a one-line window walks every
        // line in order — which is also how a reader reaches a line they mean.
        let mut canvas = canvas();
        let height = 1;
        let total = canvas.layers()[0].lines.len();
        let mut found = Vec::new();
        for _ in 0..total {
            if let Some(pointer) = canvas.pointer_under_cursor() {
                found.push(pointer);
            }
            canvas.move_cursor(1, height);
        }
        assert_eq!(
            found,
            vec![(41, 41), (41, 52)],
            "L0 names two pointers, and each is reachable by scrolling to it"
        );
        // A line with no pointer is not a pointer.
        canvas.move_cursor(-100, height);
        assert_eq!(
            canvas.pointer_under_cursor(),
            None,
            "the first line is blank"
        );
    }

    #[test]
    fn moving_stops_at_both_ends_and_the_window_follows() {
        let mut canvas = canvas();
        let height = 2;
        canvas.move_to(true, height);
        let bottom = canvas.window(height).to_vec();
        assert!(!bottom.is_empty());
        // Past the end is the end, not an empty window.
        canvas.move_cursor(100, height);
        assert_eq!(canvas.window(height).to_vec(), bottom);
        // The cursor is inside the window wherever it is.
        let cursor_row = canvas.cursor_row(height).expect("visible");
        assert_eq!(
            canvas.window(height)[cursor_row],
            canvas.line_at_cursor().unwrap_or_default()
        );

        canvas.move_to(false, height);
        assert_eq!(canvas.window(height)[0], canvas.layers()[0].lines[0]);
        canvas.move_cursor(-10, height);
        assert_eq!(canvas.window(height)[0], canvas.layers()[0].lines[0]);
    }

    #[test]
    fn every_line_is_reachable_even_in_a_document_shorter_than_the_window() {
        // The bug this exists for: with the cursor pinned to the top line, a
        // pointer on the second line of a short artifact could never be reached,
        // because there was nothing to scroll.
        let mut canvas = canvas();
        let height = 40;
        assert!(canvas.layers()[0].lines.len() < height);
        let mut seen = Vec::new();
        for _ in 0..canvas.layers()[0].lines.len() {
            if let Some(text) = canvas.line_at_cursor() {
                seen.push(text.to_string());
            }
            canvas.move_cursor(1, height);
        }
        assert_eq!(seen, canvas.layers()[0].lines, "every line in order");
    }

    #[test]
    fn switching_layers_starts_at_the_top_of_the_new_one() {
        let mut canvas = canvas();
        canvas.move_cursor(2, 2);
        canvas.step_layer(1);
        assert_eq!(canvas.current(), 1);
        assert_eq!(canvas.window(2)[0], canvas.layers()[1].lines[0]);
        canvas.step_layer(-1);
        assert_eq!(canvas.current(), 0);
        // Clamped at both ends.
        canvas.step_layer(-5);
        assert_eq!(canvas.current(), 0);
        canvas.step_layer(99);
        assert_eq!(canvas.current(), canvas.layers().len() - 1);
    }

    #[test]
    fn an_expansion_replaces_the_layer_until_it_is_dismissed() {
        let mut canvas = canvas();
        canvas.expansion_pending("evt 41–52".to_string());
        assert!(matches!(
            canvas.expansion(),
            Some(Expansion::Pending { .. })
        ));
        assert_eq!(canvas.expansion().map(Expansion::label), Some("evt 41–52"));

        canvas.expansion_ready("evt 41–52".to_string(), "row one  (evt 41)\n");
        assert_eq!(
            canvas.window(10),
            &["row one  (evt 41)".to_string()],
            "the expansion replaces the layer on screen"
        );

        assert!(canvas.dismiss_expansion(), "esc closes it");
        assert!(canvas.expansion().is_none());
        assert!(!canvas.dismiss_expansion(), "and a second esc does nothing");
    }

    #[test]
    fn a_range_that_cannot_be_read_says_so_rather_than_showing_nothing() {
        let mut canvas = canvas();
        canvas.expansion_failed(
            "evt 900–999".to_string(),
            "outside the active branch".into(),
        );
        let shown = canvas.window(10).join("\n");
        assert!(shown.contains("could not read this range"), "{shown}");
        assert!(shown.contains("outside the active branch"), "{shown}");
    }

    #[test]
    fn something_that_is_not_an_artifact_is_refused_by_name() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("notes.md");
        std::fs::write(&path, "# just some notes\n").expect("write");
        let error = Canvas::load(&path).expect_err("not an artifact");
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("no L0 layer"), "{error}");

        let missing = dir.path().join("absent.md");
        assert!(Canvas::load(&missing).is_err());
    }
}
