# sctxx Development Log

Important product, architecture, workflow, and operational evolutions, newest first. Each entry is added in
a separate documentation commit after the implementation commit and cites full canonical commit SHAs.
Update logged hashes after any history rewrite or squash.

<!-- Entry format:
## YYYY-MM-DD - <short title>

Commits: `<full sha>`, `<full sha>`

<What changed, why, and what later work must know. Link the ledger block: specs/NNN-slug/.>
-->

## 2026-09-11 - Compaction kind in the IR, and `--since-compact` implemented

Commits: `0b81048d81808574c86437e4290f89c629cde40e`

The research entry below identified two dead wires: `NativeCompaction` could not say whether a provider
boundary was a window re-anchor or a real history reset, and `--since-compact` (spec §3.4) had no flag.
Both are now closed, and the Codex `compacted` path is exercised by a fixture.

- **IR**: `NativeCompaction` gains `windowed: bool` (serde-defaulted, so an older `ir.v1` document still
  deserializes). Codex sets it from `compacted.payload.window_number`; every other adapter defaults to a
  reset, which is correct for Claude Code's `isCompactSummary` and Pi's `compactionSummary`.
- **CLI**: `extract --since-compact` starts at the newest *reset* when the session has one, otherwise at
  the earliest *re-anchor*, keeps the boundary event as the low-trust seed, and leaves `events`
  untouched so `expand` still resolves every `[evt a-b]` pointer. A session that never compacted is a
  stderr notice with exit 0, never a failure.
- **Bug fixed**: a `compacted.replacement_history` message envelope nests its text under `content`, so a
  local compaction summary was previously read as an empty string. Nesting is now walked, bounded to
  eight levels so a corrupt session file cannot exhaust the stack.
- **Snapshot churn**: exactly one added line (`"windowed": false`) in each of the three existing
  compaction snapshots, reviewed individually, plus one new snapshot for the new fixture.
- **Still open in this block**: T0604 (pass provider summaries to the fold as low-trust seeds — a
  versioned-prompt change, deferred to its own slice) and T0605 (`prompts/baseline_codex_compact.md`,
  M5-gated on the eval harness). Both are recorded unchecked in
  `specs/006-m2-codex-adapter/tasks.md`.
- **Process debt recorded**: RED was not captured as a separate failing run for T0601–T0603; the tests
  were written alongside the implementation and the pre-change observable is stated in
  `specs/006-m2-codex-adapter/evidence/T0601-T0603.md`. Do not let this become the habit — the
  methodology's RED step is what proves a test can fail.

Ledger: `specs/006-m2-codex-adapter/` (spec, tasks, evidence).

## 2026-09-11 - Codex compaction algorithm extracted; the reuse boundary drawn

Commits: `7e814ef08d518aad2f645a702afa441d533bf039`

The specs existed but the Codex side of the design was an assessment, not a reading. This change reads
the reference clone's compaction path end to end and records what sctxx actually takes from it.

Findings that later work must honor:

- **`compacted` lines have two meanings.** `window_number == null` (with a `replacement_history`) is a
  legacy history reset; `window_number != null` is a window re-anchor that leaves the full transcript
  intact. sctxx's adapter emits the right event in every case — including the empty-`message` case,
  because `str_field` filters empty strings — but `NativeCompaction` does not record which kind it saw,
  so `--since-compact` is currently undefinable. Ticket
  `specs/000-wayfinding/issues/05-codex-compacted-readability.md` is resolved; the IR change is scoped
  as candidate tasks T0601–T0605 in `specs/006-m2-codex-adapter/research.md`.
- **Codex now ships a summarization-free compaction path.** Token-budget compaction replaces the window
  with canonical context plus retained evidence and writes `message: ""`. That is the upstream
  equivalent of sctxx's `--llm none` artifact, and it needs no new mechanism here.
- **Two dead wires were found in the shipped code.** `Session.native_compactions` is populated and read
  by nothing, so spec §7.2's "low-trust seeds" reach no consumer; and `--since-compact` (spec §3.4) has
  no flag. Both are in the candidate task list (T0604, T0605).
- **An anti-pattern to avoid:** Codex's tiered budgeting returns an *empty* evidence string when the
  render still exceeds the budget. sctxx budgets must fail soft to the deterministic artifact.
- **A provenance problem:** the `codex/` reference clone is a `0.0.0-dev`, ≥0.120-line build with no
  `.git`, so the pinned commit asserted in `AGENTS.md`, spec §2.3/Appendix A, the vendor README, and
  every vendored header cannot be verified. Filed as
  `specs/000-wayfinding/issues/11-codex-vendoring-pin.md`; must be resolved before the M4 publish.

Decision record: `docs/adr/0002-codex-compaction-algorithm-reuse.md`. Evidence:
`specs/006-m2-codex-adapter/research.md`. Spec §19 item 2 is annotated as resolved.
