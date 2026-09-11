//! `sctxx` — Session ConTeXt eXtractor.
//!
//! Reads a coding-agent session transcript from disk (Claude Code, Codex CLI,
//! or Pi) and produces a compact, verified, provenance-linked **handoff
//! artifact** that a different coding agent can load to continue the work.
//!
//! The design principle is deterministic-first: Rust computes branch
//! resolution, ledgers, masking, segmentation, budgets, validation, apply,
//! reconciliation, and rendering; a model is used only for semantic judgment,
//! and every model output is schema-validated and gated before it can change
//! state. `--llm none` is a supported product, not a degraded mode.
//!
//! ```no_run
//! use sctxx::adapters::discovery;
//! use sctxx::pipeline::{self, ExtractOptions};
//!
//! let reference = discovery::parse_reference("claude:last")?;
//! let options = discovery::ResolveOptions::default();
//! let session = discovery::resolve(&reference, &options)?;
//! let extraction =
//!     pipeline::extract(&session, &ExtractOptions::default(), &mut |_, _| {})?;
//! print!("{}", extraction.markdown(&ExtractOptions::default()));
//! # Ok::<(), sctxx::error::Error>(())
//! ```
//!
//! The library surface is public so hosts can embed the pipeline, but it is
//! **unstable before 1.0**: only the CLI, the exit codes, and the
//! `sctxx.handoff/v1`, `ops.v1`, and `state.v1` schemas are contracts.

#![forbid(unsafe_code)]
// Library code must not panic: a session file is untrusted input and sctxx is
// often run unattended inside another agent's loop.
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]
// `clippy::indexing_slicing` is deliberately not denied. Every index in this
// crate is bounded by the collection it came from (`enumerate`, `position`, a
// computed char boundary, or a range carved from the same slice), and rewriting
// those as `get()` chains would hide that reasoning behind noise. The
// panic-freedom that matters — no `unwrap` or `expect` on untrusted session
// input — is enforced above.
// Every module, type, and function carries documentation explaining *why* it
// exists. `missing_docs` is deliberately not enabled: it would demand a line
// on self-describing fields like `pub lines: usize` and bury the reasoning
// that matters. Revisit when the library surface is stabilized for 1.0.
#![warn(missing_debug_implementations)]

pub mod adapters;
pub mod cli;
pub mod error;
pub mod ir;
pub mod llm;
pub mod pipeline;
pub mod skill;
#[cfg(feature = "tui")]
pub mod tui;
pub mod vendor;

/// The published version of this build, as it appears in every artifact.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The JSON Schemas this build ships (`sctxx schema <name>`).
pub const SCHEMAS: &[(&str, &str)] = &[
    ("ops", include_str!("../schemas/ops.v1.json")),
    ("handoff", include_str!("../schemas/handoff.v1.json")),
    ("state", include_str!("../schemas/state.v1.json")),
    ("ir", include_str!("../schemas/ir.v1.json")),
];

#[cfg(test)]
mod tests {
    #[test]
    fn every_shipped_schema_is_valid_json_and_self_identifies() {
        for (name, body) in super::SCHEMAS {
            let value: serde_json::Value =
                serde_json::from_str(body).unwrap_or_else(|error| panic!("{name}: {error}"));
            let id = value["$id"].as_str().unwrap_or_default();
            assert!(id.contains(name), "{name}: $id is {id}");
        }
    }
}
