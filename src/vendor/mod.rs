//! Third-party code, ported rather than depended on.
//!
//! crates.io forbids git and path dependencies in published crates, and the
//! Codex workspace crates are all versioned `0.0.0` with heavy transitive
//! dependencies, so depending on them would make sctxx unpublishable
//! (spec §2.2, decision D-1). Each ported file keeps its Apache-2.0
//! attribution header and a row in the manifest.

pub mod codex;
