# Decide whether the ported tiered-selection algorithm is wired in or removed

Type: decision
Status: open

## Question

`src/vendor/codex/tiered_input.rs` ports two things from Codex's `serialize_tiered_input`
(`codex-rs/memories/write/src/rollout_input.rs`, pinned commit `818f1cca…`):

1. the **`Tier` ordering** — human messages first, then final assistant text, then subagent, commentary,
   prior summary, failed tool results, tool calls, successful tool results; and
2. **`select(rows, token_limit)`** — the algorithm that fills a budget newest-first within those tiers,
   reporting what fitted and what was dropped.

A call-graph audit (`docs/CODEX-PROVENANCE-AUDIT.md`, 2026-09-11) established that **`select` has no
call sites**: `Tier` classifies rows in `mask.rs` and `segment.rs`, `mask::Row::tiered()` builds a
`TieredRow`, and nothing ever asks it to choose. What picks the recency tail is sctxx's own
`segment::plan`, which cuts at an episode boundary that keeps the tail within budget.

So the file's own doc comment ("Codex's insight: when a whole transcript must fit one budget, filling it
newest-first *within a priority tier* keeps the highest-signal evidence") describes a mechanism this
repository does not use.

## Why it matters

Three separate problems, and only one of them is tidiness:

- **A claim in the tree is not true.** The vendor README and the file header present the tiered
  budgeting as operational. `AGENTS.md` rule 1 requires the vendored manifest to be accurate, and the
  provenance audit exists precisely because a claim like this was made too strongly elsewhere.
- **The spec may disagree with the code.** §7.3 describes masked evidence being fitted into a bounded
  context. If that is meant to be tier-ordered, `select` is the intended mechanism and its absence is a
  gap rather than dead weight.
- **Unused vendored code is a licensing and maintenance cost with no benefit.** Apache-2.0 obliges the
  attribution we carry; carrying it for an unreachable function is paying without receiving.

## Decide

1. **Wire it.** Have the tail/budget selection consult `select` where §7.3 says tier ordering applies —
   most plausibly when trimming a chunk or the tail to a budget, rather than only at episode
   boundaries. This must be measured: episode-boundary selection is deterministic and cheap, and tier
   selection could reorder a tail in ways the fold and the artifact do not expect.
2. **Keep the taxonomy, delete the selector.** Remove `select`, `Selection` and the parts of
   `TieredRow` that exist only for it, keep `Tier` as the classification it really is, and correct the
   README row to say so.
3. **Keep it, documented as unused**, with a named plan to use it. The weakest option, and only
   defensible with a dated intention.

Whichever is chosen, `docs/CODEX-PROVENANCE-AUDIT.md` and `src/vendor/codex/README.md` change in the
same commit.

## Evidence to produce

A before/after on a real session: does tier-ordered selection change the recency tail, the fold's input,
or the artifact's size, and is the result better by any measure the project already uses (`eval`,
probe scores, or a human read)? If the answer is "no measurable difference", option 2 is the honest one.
