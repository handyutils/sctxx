//! Code ported from OpenAI Codex (Apache-2.0) at a pinned commit.
//!
//! Every file here carries an attribution header naming its upstream path and
//! the modifications made; `README.md` in this directory is the manifest, and
//! `scripts/check-vendor-headers.sh` fails CI if a header is missing. sctxx
//! never depends on a `codex-*` crate.

pub mod apply_patch_paths;
pub mod reconstruction;
pub mod secrets;
pub mod tiered_input;
pub mod truncate;

/// The upstream commit every file in this directory was ported from.
pub const UPSTREAM_COMMIT: &str = "818f1cca8ccf8899f0f4d59336baebaccf358eed";
/// The upstream repository.
pub const UPSTREAM_REPO: &str = "https://github.com/openai/codex";
