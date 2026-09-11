# Tasks: 006 - M2 Codex CLI adapter (compaction slice)

**Status**: Active · **Spec**: [`spec.md`](spec.md) · **Plan**: in the spec's Scope/Success criteria
**Evidence**: [`evidence/`](evidence/) · **Research**: [`research.md`](research.md)

The ID space here is authoritative. `research.md` proposed provisional IDs (T0601–T0607); the mapping is
noted per task. Tasks are checked only after a RED/GREEN loop, the gate, and an evidence file exist.

## Done

- [x] **T0601** [US1/US2] Pin both compaction kinds in a fixture — `tests/fixtures/codex/windowed-compaction.jsonl`
  - Why: the provider's `compacted` item means two different things and nothing in the corpus distinguished them
  - Depends on: nothing
  - RED/GREEN proof: `cargo test --all-features --test adapters codex_windowed_compaction`
  - Acceptance: `tests/snapshots/adapters__codex_windowed_compaction.snap` shows `windowed: false` for the
    legacy reset (evt 1) and `windowed: true` with `summary: null` for the window marker (evt 3)
  - Covers provisional T0601

- [x] **T0602** [US1] Record the compaction kind in the IR and set it from the Codex payload
  - Why: `NativeCompaction` could not distinguish "the provider re-anchored its window" from "the
    provider discarded everything before this point" (ADR 0002)
  - Depends on: T0601
  - Touches: `src/ir.rs` (`NativeCompaction.windowed`), `src/adapters/mod.rs`
    (`SessionBuilder::mark_compaction_windowed`), `src/adapters/codex.rs`
  - RED/GREEN proof: `cargo test --all-features --test adapters a_codex_window_marker_is_not_a_history_reset`
  - Acceptance: an empty `message` yields `windowed: true` with no summary text; `Session.active` is unchanged
  - Covers provisional T0602 + T0603

- [x] **T0603** [US2/US3] `--since-compact`: pick the boundary and start there
  - Why: spec §3.4 defines the flag and it did not exist; the boundary rule also needed the IR flag to be correct
  - Depends on: T0602
  - Touches: `src/pipeline/mod.rs` (`since_compact_boundary`, `ExtractOptions.since_compact`),
    `src/cli/extract.rs`, `skill/references/cli.md`
  - RED/GREEN proof: `cargo test --all-features --lib since_compact` and
    `cargo test --all-features --test cli since_compact`
  - Acceptance: reset beats a later re-anchor; re-anchor-only uses the earliest; no compaction is a
    stderr notice with exit 0; pre-boundary history leaves the artifact and post-boundary history stays
  - Covers provisional T0605

- [x] **T0604** [US1] Pass provider compaction summaries to the fold as low-trust seeds
  - Why: spec §7.2 says they are seeds; only the masked-row half existed, so a chunk after a
    compaction boundary saw nothing of what came before it and nothing framed the text as lossy
  - Depends on: T0602
  - Touches: `prompts/fold_user.md` (version 1 → 2), `src/pipeline/fold/prompt.rs`,
    `src/pipeline/fold/mod.rs`
  - RED/GREEN proof: `cargo test --all-features --lib prior_summaries` and
    `cargo test --all-features --lib the_low_trust_seed`
  - Acceptance: three most recent summaries, oldest first, 400 tokens each; an unreadable boundary is
    reported rather than dropped; the rendered prompt carries the "never as evidence" framing
  - Evidence: [`evidence/T0604.md`](evidence/T0604.md)
  - Covers provisional T0604

## Open

- [ ] **T0605** [US4] `prompts/baseline_codex_compact.md` — Codex's own compaction prompt, verbatim
  - Why: spec §8.6 and Appendix A require it as the `sctxx eval --baseline codex-compact` text
  - Depends on: the eval harness (`specs/016-m5-eval-harness-and-benchmark/`)
  - M5-gated. Adding an unused prompt file now would be speculative work (Ponytail rung 1).
  - Covers provisional T0606

## Out of scope for this block

- Provisional T0607 (resolve the vendoring pin) lives in
  `specs/000-wayfinding/issues/11-codex-vendoring-pin.md`, not here.
- Fork-parent interaction with `--since-compact` (see spec Non-goals).
