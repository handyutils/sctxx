# Pin the Codex vendoring source

Type: research
Status: resolved (2026-09-11) — option 1, verified at the pin

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

## Answer

**Option 1: the pin is real and the attribution is accurate. Nothing needed re-pinning.**

Verified on 2026-09-11 by fetching the pinned commit directly:

```sh
git init /tmp/codex-pin-check && cd /tmp/codex-pin-check
git remote add origin https://github.com/openai/codex
git fetch --depth 1 origin 818f1cca8ccf8899f0f4d59336baebaccf358eed
git checkout FETCH_HEAD
```

1. **The commit exists upstream.** `818f1cca8ccf8899f0f4d59336baebaccf358eed`, committed
   `2026-09-10T17:03:11Z` by `marksteinbrick-oai`, subject *"Remove `repo_url` from skill invocation
   analytics events (#44586)"*. The date matches the `2026-09-10` the spec states.
2. **Every upstream path the manifest names exists at the pin**: `utils/string/src/truncate.rs`,
   `secrets/src/sanitizer.rs`, `memories/write/src/rollout_input.rs`,
   `core/src/session/rollout_reconstruction.rs`, `apply-patch/src/parser.rs`,
   `memories/write/templates/memories/stage_one_system.md`,
   `prompts/templates/compact/summary_prefix.md`, `prompts/templates/compact/prompt.md`.
3. **The symbols the ports claim to keep exist there**: `approx_token_count` and
   `approx_bytes_for_tokens` (`truncate.rs:71,76`); `redact_secrets` with `sk-[A-Za-z0-9]{20,}` and
   `\bAKIA[0-9A-Z]{16}\b` (`sanitizer.rs:4-17`); `serialize_tiered_input` with
   `TOOL_OUTPUT_TOKENS = 2_000` and `MAX_ROW_BYTES = 10_000` (`rollout_input.rs:24,38`);
   `reconstruct_history_from_rollout` (`rollout_reconstruction.rs:134`); the `*** Add File:` grammar;
   and a `prompt.md` of exactly nine lines.
4. **The compaction behaviour this repository depends on is in the pin, not in a newer build.**
   `window_number` on `CompactedItem` (`history/src/lib.rs:196`), `core/src/compact_token_budget.rs`,
   and `core/src/compact_remote_v2.rs` are all present at the pin. This *corrects* an earlier claim in
   `specs/006-m2-codex-adapter/research.md` that those modules post-dated the pin.

**The real problem was narrower than the ticket assumed.** The vendored code and its headers were never
suspect; the unversioned `codex/` clone was. That clone is a *reading aid* for current upstream
behaviour and is explicitly **not** the provenance of any vendored file. `src/vendor/codex/README.md`
now says so, and the README of this repository no longer implies otherwise.

**Follow-up, not a blocker:** the local clone stays unversioned by design (it is gitignored and
disposable). If a future change needs to port newer Codex code, fetch that revision explicitly, record
its commit hash in the header, and update Appendix A — the same procedure, repeated.
