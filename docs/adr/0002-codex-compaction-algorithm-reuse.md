# ADR 0002 — What sctxx adopts from Codex's compaction algorithm

- **Status**: accepted
- **Date**: 2026-09-11
- **Resolves**: `docs/SCTXX-SPEC.md` §19 item 2 (readability of Codex `compacted` lines)
- **Affects**: §3.4 (`--since-compact`), §3.5 (`show`), §6.2 (Codex adapter), §7.2 (native compactions
  ledger), §8.5–8.6 (fold seeds, baseline prompt)
- **Evidence**: [`specs/006-m2-codex-adapter/research.md`](../../specs/006-m2-codex-adapter/research.md),
  read from the `codex/` reference clone; ticket
  [Determine whether Codex `compacted` lines carry readable summaries](../../specs/000-wayfinding/issues/05-codex-compacted-readability.md)

## Context

sctxx and Codex solve different problems with the same word. Codex's *compaction* is an in-loop
context-window mechanism: when the active context approaches the model's window, it replaces history
with a short summary so the thread can continue. sctxx's *extraction* is post-hoc and offline: it reads
a finished transcript and produces a handoff artifact for a **different** agent.

The spec (§2.1) assessed Codex's compaction path as "thin" and planned to reuse only the Phase-1
memory pipeline, tiered budgeting, rollback replay, truncation, redaction, and the `apply_patch`
grammar, using the compaction prompts purely as an eval baseline. That assessment was made against an
older revision. The reference clone now contains three compaction implementations, a window bookkeeping
layer, and a summarization-free "token-budget" path — enough that the boundary between *reusable* and
*in-loop-only* had to be drawn deliberately rather than assumed.

The blocking question (§19 item 2) was factual: do Codex's `compacted` lines carry readable summaries?
The answer decides what the M2 adapter emits and whether `--since-compact` can be built at all.

## Decision

1. **Readability is not guaranteed, and the adapter's current behaviour is correct.** A `compacted`
   item carries readable text when a local summarization ran (`message`, plus a readable
   `replacement_history`). It carries none when the summary is a server-side encrypted item (remote
   compaction) or when token-budget compaction ran, which writes `message: ""` by design. The adapter
   emits `NativeCompactionSummary` when text exists and `System { subtype: "native_compaction" }`
   otherwise; `str_field` already filters empty strings, so the empty-`message` case is handled
   correctly (`src/adapters/mod.rs:272-277`, `src/adapters/codex.rs:90-103`).

2. **A `compacted` line has two meanings and the IR must say which.** `window_number == null` together
   with a `replacement_history` is a **legacy history reset**: everything before it is gone and the
   replacement history is the surviving conversation. `window_number != null` is a **windowed
   re-anchor**: the full transcript is intact and the item only marks a new context window and a reset
   token baseline. `NativeCompaction` gains a flag for this and the adapter populates it.

3. **Adopt Codex's retention *shape*, not its retention *policy*.** Newest-first user-message fill under
   a token budget with the summary kept last as the anchor (20,000 tokens upstream) becomes the shape
   of the bounded `--since-compact` seed. sctxx's artifact keeps rendering **all** user messages
   (spec §7.2): an artifact is not a context window, and `expand` can always recover detail.

4. **Do not port the in-loop loop.** Triggers, window numbers, prefill baselines, fallback buffers, and
   world-state accounting exist to decide *when* to compact. Offline, sctxx only needs to know *what a
   past compaction meant*. This keeps the roadmap's "no live in-loop compaction before M6" invariant.

5. **Token-budget compaction is treated as external validation, not as new scope.** Codex now ships a
   path that performs no summarization at all and rebuilds the window from canonical context plus
   retained evidence. That is the same design as sctxx's `--llm none` artifact, and it needs no new
   mechanism here.

6. **Codex's Phase-1 tiered budgeting stays vendored as-is**, with one explicit exception: its
   over-budget branch returns an empty evidence string, and sctxx must never degrade that way — budgets
   fail soft to the deterministic artifact.

## Alternatives considered

- **Port Codex's compaction loop.** Rejected: wrong product boundary. sctxx reads a finished file; the
  loop is only meaningful inside a live agent, and the roadmap defers in-loop work to M6.
- **Model Codex's `ResponseItem` enum to parse `compacted` faithfully.** Rejected again (spec D-2):
  `serde_json::Value` projections keep the adapter tolerant of format drift and keep the crate
  publishable without a `codex-*` dependency.
- **Treat every `compacted` line as a hard history boundary.** Rejected: on current Codex it would
  discard the entire pre-window transcript of a session that was never actually truncated — for a tool
  whose value is recovering exactly that history, the worst possible failure.
- **Replace sctxx's user-message rendering with Codex's 20,000-token retention.** Rejected: it optimises
  for the wrong budget (model context) and would silently drop user constraints the artifact exists to
  preserve.
- **Mirror `world_state` / `retained_context` / `guardian_history` in the IR.** Rejected (Ponytail):
  sctxx's repo reconciliation and ledgers serve the same purpose more portably, from data any provider
  writes.

## Consequences

- `NativeCompaction` changes shape (architectural class): `schemas/ir.v1.json`, adapter snapshots, and
  the IR round-trip tests move in the same change. `ir.v1` is unfrozen until M7, so no version bump is
  required yet; the change follows the normal block lifecycle in `specs/006-m2-codex-adapter/`
  (specify → plan → tasks → implement), not a direct edit.
- `--since-compact` becomes well defined: prefer the newest legacy **reset** boundary; otherwise the
  oldest **window** boundary; seed with that item's `message` as a low-trust summary and say so on
  stderr.
- `Session.native_compactions` must actually reach the fold as low-trust seeds (spec §7.2/§7.3).
  Today it is populated and never read.
- `docs/SCTXX-SPEC.md` §19 item 2 is resolved by this ADR and annotated in place; §6.2's `compacted`
  mapping stands unchanged.
- Licensing follow-up: the reference clone cannot be verified as the pinned commit
  (`818f1cca8ccf8899f0f4d59336baebaccf358eed`). The pin is asserted in `AGENTS.md`, spec §2.3 and
  Appendix A, `src/vendor/codex/README.md`, and every vendored header, so it must be reconciled before
  M4 publishes. Tracked separately; it is a governance item, not an adapter blocker.
- Revisit if a future Codex revision drops the legacy `replacement_history` path entirely: the
  windowed/legacy flag then collapses to a constant, and the adapter change becomes dead weight.
