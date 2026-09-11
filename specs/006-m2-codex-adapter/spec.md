# Feature Specification: M2 - Codex CLI adapter

**Feature Branch**: `006-m2-codex-adapter`
**Created**: 2026-09-10
**Status**: Active
**Input**: docs/SCTXX-ROADMAP.md M2; docs/SCTXX-SPEC.md §6.1, §6.2, Appendix A

## Scope

Read a Codex CLI rollout into the canonical IR with correct active-branch semantics, and make the
provider's own compaction history legible to the rest of the pipeline. Rollout line decoding,
`.jsonl.zst`, the `sessions/` and `archived_sessions/` roots, `request_user_input` pairing, vendored
`ThreadRolledBack` replay, and `apply_patch` header parsing already exist from 0.1.0 and keep their
existing coverage. The slice specified here is **compaction**.

**Unblocked by**: [Determine whether Codex `compacted` lines carry readable summaries](../000-wayfinding/issues/05-codex-compacted-readability.md)
— resolved 2026-09-11; decision recorded in `docs/adr/0002-codex-compaction-algorithm-reuse.md`.

## Actors and goals

- **A receiving agent** runs `sctxx extract` on a Codex session and needs to know whether the history it
  is handed is complete, and where the provider itself considered the conversation to have restarted.
- **An operator** extracting a very long Codex session wants to start at the provider's own boundary
  rather than fold a session that upstream already summarised.
- **A maintainer** needs the adapter to survive provider format drift without silently dropping events.

## User scenarios

1. **A transcript that was never truncated.** A session whose `compacted` items carry a `window_number`
   is a *window re-anchor*: the provider replaced its context window but kept the rollout. Extraction
   must keep the whole history and mark the boundary as a re-anchor, so `--since-compact` never discards
   history that was not lost.
2. **A transcript that was genuinely reset.** A `compacted` item with a `replacement_history` is a real
   history reset. `--since-compact` starts there and keeps that item's summary as a low-trust seed.
3. **Both, or neither.** Both kinds present → the newest reset wins. No compaction at all → the flag is
   a notice on stderr and the whole session is extracted; the exit code stays 0.
4. **An encrypted or empty summary.** Remote compaction returns an opaque item, and token-budget
   compaction writes an empty `message`. Neither may be reported as a readable summary.

## Functional requirements

- **FR-001** The IR records, for each provider compaction boundary, whether it was a window re-anchor or
  a history reset.
- **FR-002** The Codex adapter sets that flag from `compacted.payload.window_number`; every other
  adapter's boundaries default to a reset.
- **FR-003** An empty `message` never becomes summary text; such a boundary is a `System` event with
  subtype `native_compaction`, not a `NativeCompactionSummary`.
- **FR-004** `sctxx extract --since-compact` retains only active events at or after the chosen boundary,
  keeps the boundary event itself as the low-trust seed, and leaves `events` untouched so every
  `[evt a-b]` pointer still resolves through `sctxx expand`.
- **FR-005** Boundary choice: the newest reset if there is one; otherwise the earliest re-anchor;
  otherwise none.
- **FR-006** The run reports the chosen boundary, or its absence, on stderr. stdout stays payload-only.
- **FR-007** A compaction boundary never changes the default active branch: sctxx keeps the full history
  unless `--since-compact` is given.

## Edge cases

- `window_number: null` as a present key → a reset: the value decides, not the key's presence.
- A `replacement_history` whose text is nested two levels deep (message envelope → content blocks) must
  still render; nesting is bounded so a corrupt file cannot exhaust the stack.
- Many boundaries in one session, in any order.
- A session with no `compacted` line at all.
- `.jsonl.zst` rollouts follow the same rules once decompressed.

## Success criteria

- A fixture pins both compaction kinds, and an `insta` snapshot of its IR shows the flag for each.
- A unit test for boundary selection covers reset-preferred, re-anchor-only, and no-compaction cases.
- A CLI test proves `--since-compact` drops pre-boundary history from the artifact, keeps post-boundary
  history, names the boundary on stderr, and exits 0 when nothing compacted.
- The full gate (`fmt`, `clippy -D warnings`, `test --all-features`, `test --no-default-features`) passes.

## Non-goals

- Replaying Codex's live compaction loop (window accounting, prefill baselines, trigger thresholds).
  That is in-loop behaviour; sctxx is post-hoc (ADR 0002).
- Reconstructing the provider's post-compaction prompt (world state, permissions, retained developer
  messages). sctxx's ledgers and reconciliation serve that purpose more portably.
- Making `--include-fork-parent` interact with `--since-compact` across forks; `--include-fork-parent`
  is not built yet.

## Dependencies

- ADR 0002 for the reset-vs-re-anchor rule.
- `docs/SCTXX-SPEC.md` §3.4 (`--since-compact`), §6.2 (`compacted` mapping), §19 item 2 (resolved).
- The reference clone's exact revision is disputed: `specs/000-wayfinding/issues/11-codex-vendoring-pin.md`.
  This block depends only on the *shape* of `CompactedItem`, which the fixture pins independently.
