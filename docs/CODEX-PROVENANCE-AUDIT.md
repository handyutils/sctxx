# Codex provenance audit

**Date**: 2026-09-11 · **Pin**: `818f1cca8ccf8899f0f4d59336baebaccf358eed`
**Prompted by**: [`REVIEW_FEEDBACK_1.md`](REVIEW_FEEDBACK_1.md), which asked for a call-graph audit
rather than an argument from documentation: *"Codex source → sctxx port → modified how → called from →
actually exercised by your handoff"*.

This is that audit, done by grepping the actual call sites in `src/` for every public item of
`src/vendor/codex/`, excluding the vendored files themselves.

## Verdict in one line

**The provenance is real, the attribution is correct, and the claim "we extracted Codex's compaction
algorithm" is wrong.** What sctxx took is Codex's *context-management and memory-extraction
primitives* plus the *retention shape* of its compaction; what it built on top is its own architecture.
And the audit found one thing the review assumed and could not check: **the tiered-selection algorithm
is vendored but never called.**

## The audit

| Upstream (pinned commit) | sctxx file | What was kept | Call sites outside `vendor/` | Exercised? |
|---|---|---|---|---|
| `codex-rs/utils/string/src/truncate.rs` | `truncate.rs` | UTF-8-safe middle truncation, the 4-bytes-per-token estimate; added head/tail helpers | `approx_token_count` 10 · `truncate_middle_tokens` 18 · `truncate_middle_bytes` 8 · `truncate_head_bytes` 3 · `truncate_tail_bytes` 2 | **Yes** — masking, chunk packing, ledger rendering and every artifact layer budget through it |
| `codex-rs/secrets/src/sanitizer.rs` | `secrets.rs` | the four upstream patterns and `[REDACTED_SECRET]`; added the §10.2 classes and `RedactMode::Strict`; non-panicking table | `redact` 51 · `RedactMode` 14 · `secret_classes` 8 | **Yes** — every masked row before a backend, every model response, every rendered artifact |
| `codex-rs/core/src/session/rollout_reconstruction.rs` | `reconstruction.rs` | rollback semantics only: drop the newest `num_turns` user-turn segments | `ReplayEvent` 19 · `surviving_indices` 2 | **Yes** — the Codex adapter's active-branch reconstruction (`src/adapters/codex.rs`) |
| `codex-rs/apply-patch/src/parser.rs` | `apply_patch_paths.rs` | the hunk-header grammar, as a file-operation extractor (sctxx never applies a patch) | `PatchOp` 7 · `parse_ops` 3 | **Yes** — the file ledger in `pipeline::ledgers` |
| `codex-rs/memories/write/src/rollout_input.rs` (`serialize_tiered_input`) | `tiered_input.rs` | the `Tier` ordering and `TieredRow`; added `PriorSummary` and `ToolResultError` tiers | `Tier`/`TieredRow` used by `mask.rs` and `segment.rs` · **`select` 0 · `Selection` 0** | **Partly — see below** |
| `codex-rs/memories/write/templates/memories/stage_one_system.md` | `prompts/fold_system.md` | rewritten from memory prose into typed handoff operations, keeping the evidence/hygiene rules | `FOLD_SYSTEM` used by the fold | **Yes**, when a model is asked for |

Two smaller items are unused outside `vendor/`: `approx_bytes_for_tokens` and `contains_secret`.

## The finding the review could not check

**`tiered_input::select` has no call sites.** The tier *taxonomy* is used — `Tier` classifies rows, and
`mask::Row::tiered()` builds a `TieredRow` — but the algorithm that fills a budget newest-first within a
tier is never invoked. What selects the recency tail is sctxx's own `segment::plan`, which cuts at an
**episode boundary** that keeps the tail within budget.

So the review's one hedge —

> "my confidence is high that the Codex-derived evidence-selection machinery is real and operational"

— is **half right, and the half that is wrong matters**: the machinery is real and vendored, and it is
not operational. The taxonomy earns its place; the selector does not, today.

This is now a decision to make rather than a claim to soften: wire it where the spec says tier budgeting
belongs, or delete it and keep only the ordering. Filed as
[`specs/000-wayfinding/issues/15-tiered-selection-is-unreachable.md`](../specs/000-wayfinding/issues/15-tiered-selection-is-unreachable.md).

## The three things called "compaction"

The review's central correction is right and worth keeping in the repository's own words:

| | Codex | sctxx |
|---|---|---|
| **Context compaction** — shrink/replace context so the agent can continue | a dispatcher over four strategies: token-budget, remote v2 (server-side), remote v1, local summarization. The local prompt is ~9 lines | **not reproduced.** sctxx's artifact is a handoff for a *different* agent, not a replacement context for the same one |
| **Memory Phase-1 extraction** — get structured knowledge out of an old rollout | eligible rollouts → memory-relevant items → tiered evidence budget → structured extraction → redaction → durable memory | **adapted.** This is the closest thing to what sctxx does, and it is what the prompt, the tier ordering and the redaction are from |
| **Evidence budgeting / transcript processing** | `serialize_tiered_input`, truncation, rollback reconstruction, patch parsing | **ported** (with the caveat above) |
| **The anchored fold** — typed ops, supersession, provenance, validation, recency reconciliation | does not exist in Codex | **sctxx's own design**, and the interesting part |

The review is also right that there is no single OSS "Codex compaction algorithm" to extract: the local
path is a prompt plus a model call, and one of the four dispatcher strategies runs on OpenAI's servers.
**A claim to have extracted it cannot be true, because the thing is not all there to extract.**

## What the repository said, and what it should say

Nineteen places were checked; the inaccurate framing was concentrated in a few of them.

- A DEVELOPMENT-LOG headline read *"Codex compaction algorithm extracted"*. That is the overstatement
  the review identified. Corrected to name what was actually ported.
- `src/vendor/codex/README.md` described `tiered_input.rs` as if the selection were in use. Corrected,
  with the audit result.
- The marketing description is now the review's own sentence, which is stronger because it is
  defensible:

> **sctxx ports selected Rust components and extraction techniques from OpenAI Codex's open-source
> memory and context-management pipeline — tiered evidence budgeting, rollback reconstruction,
> truncation, redaction, and extraction-prompt rules — and combines them with an original
> provenance-preserving anchored handoff fold. It does not reproduce OpenAI's server-side Codex
> compaction algorithm.**

## What the review got right that is not yet done

Its engineering asks are separate from provenance and mostly still open. Recorded in
`specs/024-m8-interactive-tui/tasks.md` as T2422–T2425:

1. **End-state reconciliation is authoritative** — `CurrentStep`/`NextAction` only after consuming the
   recency tail and the last-known command and git state; an action followed by success resolves
   automatically.
2. **A semantic-output health gate** — *done as of `2026-09-11`*: a fold that produced nothing now
   reports `semantic: degraded|unavailable` in the header and in L0 rather than rendering as an
   ordinary handoff. The run that prompted this review had **81 failed calls and zero items**, which is
   exactly the case the gate now catches.
3. **Cross-check L0 against L2 mechanically** — does the tail contain evidence later than a proposed
   next action that indicates success, replacement, or a changed direction? Needs no model call.
4. **Separate the historical ledger from the active workset** in L1 — the review's most concrete
   product suggestion.
5. **Evolve the goal** rather than equating it with the first prompt: `Original goal` vs the current
   active goal from folded state.
6. **Treat a large `stale:` count as a first-class warning** rather than header metadata.
7. **Exploit provider compactions, keeping their low-trust status** — the highest-value semantic object
   in a long session is often the provider's own summary, and it is currently buried in L2.
