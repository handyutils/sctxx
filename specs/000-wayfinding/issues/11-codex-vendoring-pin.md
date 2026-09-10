# Pin the Codex vendoring source

Type: research
Status: open

## Question

The Codex reference clone at `codex/` cannot be identified as the commit that sctxx claims to vendor
from. Which is true, and what do we do about it?

Observed on 2026-09-11:

- `codex/codex-rs/Cargo.toml:153-154` → `[workspace.package] version = "0.0.0"`; every crate is
  `version.workspace = true`.
- `codex/codex-cli/package.json` → `"@openai/codex"`, `"version": "0.0.0-dev"`.
- `codex/` has **no `.git`**, and the worktree gitignores it (`/codex/`).
- `codex-rs/build-info/src/lib.rs:18-27` stamps `STABLE_GIT_COMMIT` through `option_env!` and reports
  `is_source_build()` when the version is `0.0.0`, so the checkout carries no commit id at runtime
  either.
- `codex/announcement_tip.toml` marks the range `^0.(0..119).` as outdated (effective 2026-05-08),
  which places the tree on the **≥0.120 development line**.
- The tree contains compaction modules that post-date the documented surface of
  `818f1cca8ccf8899f0f4d59336baebaccf358eed` (`compact_remote_v2.rs`, `compact_token_budget.rs`,
  `state/auto_compact_window.rs`, relocated prompt templates).

Meanwhile the pinned hash `818f1cca8ccf8899f0f4d59336baebaccf358eed` is asserted in six places:
`AGENTS.md` ("Hard rules" 1 and "Common tasks"), `docs/SCTXX-SPEC.md` §2.1/§2.3/Appendix A,
`src/vendor/codex/README.md`, every `src/vendor/codex/*.rs` header, and the `derived_from` front-matter
of `prompts/fold_system.md` and `prompts/handoff_preamble.md`.

Decide one of:

1. **Re-clone at the pin** — fetch `openai/codex` at `818f1cca8ccf8899f0f4d59336baebaccf358eed`, re-verify
   every vendored file against it, and keep the current headers. Cheapest to defend legally; the
   headers stay true.
2. **Re-pin to the snapshot** — identify the real commit of the existing clone (e.g. by re-cloning and
   matching tree hashes) and update all six places plus the spec's Appendix A modification notes.
3. **Record the snapshot as unversioned** and add a provenance note stating the version line
   (`0.0.0-dev`, ≥0.120) with no commit claim.

Whichever is chosen, `scripts/check-vendor-headers.sh` must keep passing and Appendix A must stay
accurate. This is a **high-risk** item per the methodology (§4): it touches licence/NOTICE and the
pre-publish gate.

Blocks nothing today, but must be resolved before the M4 publish step (roadmap M4, `specs/014-*`).
Spec references: §2.1, §2.3, §19 item 2. Related:
[ADR 0002](../../../docs/adr/0002-codex-compaction-algorithm-reuse.md).
