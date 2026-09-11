# Decide whether the fold should precompress hierarchically, and benchmark it

Type: research
Status: open

## Question

A survey of DeepSeek Harness's compaction
([`docs/DeepSeek-Harness-…-summarization-algorithm.md`](../../docs/DeepSeek-Harness-rolling-token-pressure-triggered-one-shot-structured-checkpoint%20-summarization-algorithm.md))
raised one idea sctxx does not have, and one measurement it should make. **Should the fold summarise
*episodes* into a few large semantic units before folding chunks — and can we show it beats the cheap
alternative?**

## What the survey established

Most of what the document describes, sctxx either already does or deliberately does not:

- **Giant tool outputs are pruned deterministically before any model call.** sctxx caps every tool row at
  2,000 tokens (`mask.rs`, taken from Codex's `TOOL_OUTPUT_TOKENS`) — the same policy DeepSeek Harness
  reaches with its `dsh-compaction-tool-result-pruner` (8,192 chars → first 4,096 + last 1,024). **No new
  work.**
- **A structured checkpoint shape.** Harness asks for Primary Request / Key Technical Concepts / Files and
  Code / Errors and Fixes / Pending Jobs / Current Work / Next Step / Critical Context. sctxx asks for
  nine *typed items* with provenance and a supersession log, which is a superset for the parts that
  matter to a handoff. Two of Harness's headings are not in our L0 and probably should be: **Key Technical
  Concepts** and **Critical Context**.
- **Rolling merge of the previous summary.** sctxx passes `prior_summaries` to the fold as low-trust
  seeds, which is the same idea with a trust label.
- **Live triggers, KV-cache reuse, and replacing history in place.** These exist because Harness compacts
  a *running* session. sctxx produces a handoff for a *different* agent after the fact, so the trigger,
  the cache trick, and the replacement all address a problem sctxx does not have.

## The idea worth taking

**Hierarchy.** Harness produces *one* summary of an old region. sctxx produces 40 fold calls over 40
chunks for a 103k-event session — 813,000 masked tokens and ~81 model calls. The survey's proposal:

```text
provider compaction checkpoints + episode segmentation
        ↓
hierarchical structured checkpoint summaries   (5–10 semantic units, not 40 chunks)
        ↓
FINAL RECONCILER ← deterministic ledgers, provenance, repository
        ↓
handoff.md
```

sctxx already computes **159 episodes** for that session before chunking. Summarising per *episode* and
folding the summaries would cut the call count by roughly an order of magnitude, and the deterministic
reconciler (`pipeline::finalize`) now exists to correct the result against the evidence — which is
exactly the "use their shape as precompression, then correct it with our truth" split the survey argues
for.

## The measurement worth making

The survey's own ranking puts **sctxx's deterministic handoff at ~8/10 and its standard anchored fold at
~6/10 for this problem**, and says the architecture ceiling is higher — *"it just needs to prove that its
expensive semantic fold beats the much simpler checkpoint approach."*

That is a fair challenge and the project already has the instrument planned: `sctxx eval` (block 015/016,
roadmap M5) with `--baseline codex-compact`. This ticket adds a second baseline —
**`baseline:deepseek-harness-compact`**, the eight-heading checkpoint prompt — so the comparison is
three-way: deterministic, cheap-checkpoint, anchored fold.

## Decide

1. **Build the eval harness first (M5), then measure.** Choosing a fold architecture without a number is
   the mistake ADR 0007 was written to stop.
2. Then either adopt hierarchical precompression, or record why the anchored fold earns its cost as it
   stands.

## Evidence to produce

For one large real session: L0 token count, items produced, human-read quality, and model calls, for each
of the three baselines. The decision is which one ships as `--mode standard`.
