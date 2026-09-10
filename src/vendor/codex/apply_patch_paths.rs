// Portions derived from OpenAI Codex (https://github.com/openai/codex),
// commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file
// codex-rs/apply-patch/src/parser.rs.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
// Modified by the sctxx authors: kept only the hunk-header grammar and turned
// it into a path extractor. sctxx never applies a patch, so hunk bodies,
// context matching, and error recovery are not reproduced.

//! Extract the file operations from a Codex `apply_patch` argument.
//!
//! The artifact ledger needs to know which files an edit touched. Codex edits
//! arrive as a patch envelope whose header lines fully describe that, so
//! parsing headers alone is both sufficient and safe.

use std::path::PathBuf;

const BEGIN_PATCH_MARKER: &str = "*** Begin Patch";
const END_PATCH_MARKER: &str = "*** End Patch";
const ADD_FILE_MARKER: &str = "*** Add File: ";
const DELETE_FILE_MARKER: &str = "*** Delete File: ";
const UPDATE_FILE_MARKER: &str = "*** Update File: ";
const MOVE_TO_MARKER: &str = "*** Move to: ";

/// A file operation described by a patch header.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PatchOp {
    Add(PathBuf),
    Update(PathBuf),
    Delete(PathBuf),
    /// `*** Update File:` immediately followed by `*** Move to:`.
    Move {
        from: PathBuf,
        to: PathBuf,
    },
}

impl PatchOp {
    /// The path the operation starts from.
    pub fn path(&self) -> &PathBuf {
        match self {
            PatchOp::Add(p) | PatchOp::Update(p) | PatchOp::Delete(p) => p,
            PatchOp::Move { from, .. } => from,
        }
    }
}

/// Parse the header lines of a patch envelope. Text that is not a patch yields
/// an empty list rather than an error: adapters must never fail on one odd
/// tool call.
pub fn parse_ops(patch: &str) -> Vec<PatchOp> {
    let mut ops = Vec::new();
    let mut lines = patch.lines().peekable();

    // A shell invocation may wrap the envelope in a heredoc; skip to the marker.
    let mut seen_begin = false;
    while let Some(line) = lines.peek() {
        if line.trim_end() == BEGIN_PATCH_MARKER {
            seen_begin = true;
            lines.next();
            break;
        }
        lines.next();
    }
    if !seen_begin {
        return ops;
    }

    while let Some(line) = lines.next() {
        let line = line.trim_end();
        if line == END_PATCH_MARKER {
            break;
        }
        if let Some(path) = line.strip_prefix(ADD_FILE_MARKER) {
            ops.push(PatchOp::Add(PathBuf::from(path.trim())));
        } else if let Some(path) = line.strip_prefix(DELETE_FILE_MARKER) {
            ops.push(PatchOp::Delete(PathBuf::from(path.trim())));
        } else if let Some(path) = line.strip_prefix(UPDATE_FILE_MARKER) {
            let from = PathBuf::from(path.trim());
            match lines
                .peek()
                .map(|next| next.trim_end().strip_prefix(MOVE_TO_MARKER))
            {
                Some(Some(to)) => {
                    let to = PathBuf::from(to.trim());
                    lines.next();
                    ops.push(PatchOp::Move { from, to });
                }
                _ => ops.push(PatchOp::Update(from)),
            }
        }
    }
    ops
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_every_header_kind() {
        let patch = "\
*** Begin Patch
*** Add File: src/new.rs
+fn main() {}
*** Update File: src/old.rs
@@
-a
+b
*** Delete File: src/gone.rs
*** End Patch
";
        assert_eq!(
            parse_ops(patch),
            vec![
                PatchOp::Add(PathBuf::from("src/new.rs")),
                PatchOp::Update(PathBuf::from("src/old.rs")),
                PatchOp::Delete(PathBuf::from("src/gone.rs")),
            ]
        );
    }

    #[test]
    fn update_followed_by_move_is_one_move_op() {
        let patch = "\
*** Begin Patch
*** Update File: a.rs
*** Move to: b.rs
@@
-x
+y
*** End Patch
";
        assert_eq!(
            parse_ops(patch),
            vec![PatchOp::Move {
                from: PathBuf::from("a.rs"),
                to: PathBuf::from("b.rs")
            }]
        );
    }

    #[test]
    fn a_heredoc_wrapper_is_skipped() {
        let patch =
            "apply_patch <<'EOF'\n*** Begin Patch\n*** Add File: x.rs\n*** End Patch\nEOF\n";
        assert_eq!(parse_ops(patch), vec![PatchOp::Add(PathBuf::from("x.rs"))]);
    }

    #[test]
    fn patch_body_lines_are_never_mistaken_for_headers() {
        let patch = "\
*** Begin Patch
*** Update File: doc.md
+*** Add File: not-a-real-op.rs
*** End Patch
";
        assert_eq!(
            parse_ops(patch),
            vec![PatchOp::Update(PathBuf::from("doc.md"))]
        );
    }

    #[test]
    fn non_patch_text_yields_nothing() {
        assert!(parse_ops("cargo test --all-features").is_empty());
        assert!(parse_ops("").is_empty());
    }

    #[test]
    fn path_of_a_move_is_the_source() {
        let op = PatchOp::Move {
            from: PathBuf::from("a"),
            to: PathBuf::from("b"),
        };
        assert_eq!(op.path(), &PathBuf::from("a"));
    }
}
