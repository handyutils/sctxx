# Vendored Codex code — manifest

Every file in this directory is derived from [openai/codex](https://github.com/openai/codex),
licensed under the Apache License 2.0, at pinned commit
`818f1cca8ccf8899f0f4d59336baebaccf358eed`.

**The pin was verified on 2026-09-11.** Fetching it directly
(`git fetch --depth 1 origin 818f1cca8ccf8899f0f4d59336baebaccf358eed`) confirms the commit exists
(dated `2026-09-10T17:03:11Z`), that every upstream path in the table below exists at it, and that the
symbols these ports keep are there too. See `specs/000-wayfinding/issues/11-codex-vendoring-pin.md` for
the full check.

⚠️ The `codex/` directory at the repository root is **not** provenance. It is an unversioned,
gitignored clone kept as a reading aid for current upstream behaviour, and it may be newer or older than
the pin. Vendored code is always described by the commit above, never by that directory.

sctxx **ports** this code rather than depending on it. The Codex workspace crates are versioned
`0.0.0`, depend on each other through `workspace = true`, and pull heavy transitive dependencies;
crates.io forbids git and path dependencies in published crates, so a dependency would make sctxx
unpublishable (spec §2.2, decision D-1).

Every file starts with the attribution header required by `AGENTS.md` and
`.claude/rules/vendor-and-licensing.md`. `scripts/check-vendor-headers.sh` fails if one is missing.

| sctxx file | Upstream file | Modifications |
| --- | --- | --- |
| `truncate.rs` | `codex-rs/utils/string/src/truncate.rs` | Kept UTF-8-safe middle truncation and the 4-bytes-per-token estimate; dropped the char-count marker variant; added head/tail helpers; stable marker text for snapshots. |
| `secrets.rs` | `codex-rs/secrets/src/sanitizer.rs` | Kept the four upstream patterns and the `[REDACTED_SECRET]` token; added the secret classes in spec §10.2; added `RedactMode::Strict`; replaced the panicking regex constructor with a non-panicking table. |
| `tiered_input.rs` | `codex-rs/memories/write/src/rollout_input.rs` (`serialize_tiered_input`) | Operates on sctxx masked rows instead of Codex `RolloutItem`s; adds the `PriorSummary` and `ToolResultError` tiers; gap markers carry the omitted event range. |
| `reconstruction.rs` | `codex-rs/core/src/session/rollout_reconstruction.rs` | Kept only the rollback semantics (drop the newest `num_turns` user-turn segments); output is the surviving event indices. No history, world-state, or window bookkeeping. |
| `apply_patch_paths.rs` | `codex-rs/apply-patch/src/parser.rs` | Kept only the hunk-header grammar, turned into a file-operation extractor. sctxx never applies a patch. |

Prompt templates derived from Codex carry the same notice as an HTML comment and are listed in
`prompts/README.md`.

## Adding a vendored file

1. Copy only what is needed from the pinned commit; strip Codex-internal types.
2. Add the attribution header, naming the upstream path and your modifications.
3. Add a row to this table and to `docs/SCTXX-SPEC.md` Appendix A.
4. Run `scripts/check-vendor-headers.sh`.
