Yes. I checked the current `deepseek-ai/deepseek-harness` compaction implementation and docs.

The short answer is:

**DeepSeek Harness uses a rolling, token-pressure-triggered, one-shot structured checkpoint summarization algorithm.**

It is **much more sophisticated operationally than a naive “summarize chat” prompt**, but semantically it is still basically **one LLM summary of an old history region + a verbatim recent tail**.

It is not doing the kind of multi-stage extraction that sctxx attempts.

## How DeepSeek Harness compaction actually works

The shipped implementation is `@deepseek-ai/dsh-compaction-basic`. It activates automatically when the current model context reaches roughly **80%** of the context window by default. It aims to retain about **16%** of the context window as recent conversation verbatim. Both values are configurable per model. ([GitHub][1])

Conceptually:

```text
CURRENT SESSION
│
├── system prompt
├── tools
├── old conversation ─────────────────┐
├── more old conversation             │
├── old tool results                  │  ← selected for compaction
├── previous compacted summary        │
│                                     │
├── recent conversation               │
├── current work                      │  ← KEEP VERBATIM
└── latest user message ──────────────┘

                ↓

      optional deterministic
       TOOL RESULT PRUNING

                ↓

       one LLM summarization call

                ↓

<compacted-summary>
    structured checkpoint
</compacted-summary>

+ recent conversation untouched
```

The important point is that it **replaces** the selected historical surface region rather than adding another copy of the summary. The original events remain in the append-only session log for replay/debugging, but the model-visible “surface” gets the replacement summary. ([GitHub][2])

---

# Stage 1 — measure real context pressure

DeepSeek Harness has a separate `ctx.tokenMeter`.

It measures the actual current request envelope, including things such as:

* system prompt
* tool schemas
* messages
* assistant completion
* tool results
* buffered context
* steering/context injections

So the trigger is based on the real effective model context rather than just counting chat messages. ([GitHub][1])

Default:

```text
threshold = context_window × 0.80
```

For a hypothetical 256K model:

```text
256K × 0.80 ≈ 204,800 tokens
```

At that point it considers compaction.

---

# Stage 2 — prune giant tool outputs first

This is actually a good design.

Before spending an LLM call on compaction, it optionally runs:

`dsh-compaction-tool-result-pruner`

Default policy:

```text
If tool result > 8192 Unicode code points:

keep:
    first 4096 chars
    +
    [... tool result middle pruned ...]
    +
    last 1024 chars
```

Then it measures context pressure again.

If pruning alone brought the context below the threshold:

**no summarization call is made.**

([GitHub][3])

That is quite sensible because coding sessions can contain giant `npm`, test, compiler or file outputs where the middle adds almost no useful continuation context.

There is a limitation, though: it is syntactic head/tail pruning, not semantic extraction. The project itself documents that limitation. ([GitHub][4])

---

# Stage 3 — select the historical range

This part is deterministic.

DeepSeek Harness walks backward from the newest context and preserves a recent token budget.

Default:

```text
retain = context_window × 0.16
```

So for 256K:

```text
~41K tokens recent history remain untouched
```

Everything sufficiently old before that becomes a candidate for summarization.

It also makes sure the cut does **not split a tool call from its tool result**. This is important: the selected region must have balanced tool-call/result boundaries. ([GitHub][2])

Interestingly, it does **not necessarily preserve whole turns**. It can compact closed earlier steps inside one extremely long turn. ([GitHub][1])

Recent source discussion confirms the range algorithm is fundamentally **positional/token-budget based**, not semantic or source-aware. It walks backward according to `retainTokens` and tool-pairing constraints. ([GitHub][5])

That fact becomes important when comparing it to sctxx.

---

# Stage 4 — one structured LLM summary

This is the semantic part.

Harness replays the conversation being summarized to an LLM and adds a final compaction instruction.

The required output structure is approximately:

```text
## Primary Request and Intent

## Key Technical Concepts

## Files and Code

## Errors and Fixes

## Pending Jobs

## Current Work

## Next Step

## Critical Context
```

The instructions explicitly tell the model to preserve:

* exact paths
* commands
* error strings
* identifiers
* numeric values
* function signatures
* user corrections
* user preferences
* decisions and rationale
* unresolved questions

And importantly:

> If there is already a previous `<compacted-summary>`, merge still-valid information, drop stale information, and consolidate it into the new checkpoint.

([GitHub][1])

That last part means Harness uses **recursive rolling summarization**.

Something like:

```text
Raw history A
      ↓
 Summary A

Summary A + history B
      ↓
 Summary AB

Summary AB + history C
      ↓
 Summary ABC
```

while keeping the most recent tail verbatim.

---

# Stage 5 — replace old history

The resulting checkpoint becomes a model-visible user message wrapped roughly as:

```xml
<compacted-summary>
...
</compacted-summary>
```

and replaces the old selected region.

The event-sourced log retains:

* the summary
* which event range it shadowed
* individual shadowed event seqs
* estimated shadowed tokens
* which provider/model produced it
* token usage
* the raw provider output where applicable

So operational provenance is quite good. ([GitHub][2])

---

# A clever part: KV-cache reuse

This is one of the strongest pieces of DeepSeek Harness's implementation.

Instead of creating an arbitrary completely new summarization prompt, it tries to replay the **same conversation prefix** the model already saw and appends the compaction instruction at the end.

Conceptually:

```text
normal model request:

SYSTEM
TOOLS
MESSAGE 1
MESSAGE 2
...
MESSAGE 800
```

Compaction request:

```text
SYSTEM             ← identical
TOOLS              ← identical
MESSAGE 1          ← identical
MESSAGE 2          ← identical
...
MESSAGE 800        ← identical

"Now act as compaction engine..."
```

Therefore providers supporting prefix/KV caching can potentially reuse the huge prefix and mainly pay for the appended instruction/output. ([GitHub][1])

That is clever **production inference engineering**.

There have also been recent bugs around exactly this cache behavior and model-route inheritance, so it isn't magically perfect. ([GitHub][6])

---

# So is DeepSeek Harness superior to sctxx?

It depends on **what problem you're solving**.

And I think this distinction is extremely important for your project.

|                                    | DeepSeek Harness               | sctxx                                           |
| ---------------------------------- | ------------------------------ | ----------------------------------------------- |
| Primary job                        | Keep **current session alive** | Transfer knowledge to **another session/agent** |
| Runs                               | During agent loop              | Offline/post-hoc                                |
| Trigger                            | Context pressure               | User requests extraction                        |
| Semantic algorithm                 | One-shot structured summary    | Chunked anchored fold                           |
| Old history                        | Summarized + replaced          | Remains external source                         |
| Recent tail                        | Kept verbatim                  | Kept as L2 tail                                 |
| Tool output pruning                | ✅                              | Mask/truncate                                   |
| Structured sections                | ✅ Forced by prompt             | Typed state                                     |
| Deterministic ledgers              | Limited compaction bookkeeping | ✅ extensive                                     |
| Event provenance per semantic fact | ❌                              | ✅ intended                                      |
| Repo verification                  | ❌                              | ✅                                               |
| Cross-agent                        | Not the primary purpose        | ✅                                               |
| Typed add/update/resolve state     | ❌                              | ✅                                               |
| Multi-chunk extraction             | ❌                              | ✅                                               |
| Cost                               | Usually 1 summarization call   | potentially dozens                              |
| Latency                            | Low                            | potentially high                                |
| KV-cache optimization              | ✅ excellent                    | not central                                     |
| In-loop continuation               | **Excellent fit**              | Wrong tool                                      |
| Huge finished-session handoff      | Limited                        | **sctxx's intended strength**                   |

So I would say:

### For **live context compaction**, DeepSeek Harness is superior.

Absolutely.

I would not replace DeepSeek Harness's in-loop algorithm with the current sctxx anchored fold.

Harness has excellent machinery for:

* exact token-pressure measurement,
* retained-tail calculation,
* tool-call pairing,
* deterministic pruning,
* event-sourced replacement,
* recovery,
* retries,
* cache reuse,
* model-specific policies,
* automatic overflow recovery.

That's a mature compaction architecture.

---

# But for your sctxx problem, no

Your problem is different:

> “I have a gigantic Claude/Codex/Pi session that ran for days. Give a different agent enough reliable state to resume.”

DeepSeek Harness does not solve that as robustly.

Its semantic knowledge extraction is ultimately:

```text
Huge chunk of conversation
            ↓
        ONE LLM
            ↓
structured Markdown summary
```

There is no equivalent of sctxx's:

```text
deterministic file ledger
deterministic command ledger
error ledger
git ledger
user constraints
        ↓
40 chunks
        ↓
candidate extraction
        ↓
stateful typed operations
        ↓
supersede / resolve / confirm
        ↓
repo reconciliation
        ↓
event provenance
```

So from a theoretical standpoint, sctxx has a considerably stronger architecture for **post-hoc forensic handoff**.

The problem is that your actual 103K-event test showed that your current implementation hasn't yet fully delivered that theoretical advantage.

---

# The interesting surprise

For **your exact 103,727-event test**, I suspect a DeepSeek-Harness-style summary may actually have produced a **better L0 than your current sctxx standard mode**.

Why?

Because Harness **forces** these sections:

```text
Pending Jobs
Current Work
Next Step
Critical Context
```

and refuses to let the summarizer omit them. ([GitHub][1])

Your sctxx artifact spent roughly 81 semantic calls processing ~813K masked tokens but then failed to clearly render:

* CurrentStep
* constraints
* decisions
* open threads

and produced stale NextActions.

That's a poor trade.

DeepSeek Harness could potentially make **one** strong model call against the latest consolidated checkpoint + relevant history and give something structurally much closer to:

```text
## Current Work
Root-slot recovery has just been implemented and committed.

## Pending Jobs
- pi-cordis adoption mechanism
- acryl-cli → createAcrylEngineHost T013–T017
- ACRYL_HOME cold-start test

## Next Step
Continue T013...
```

That might have beaten the current sctxx L0 for a fraction of the complexity and cost.

But that's a hypothesis—we'd need to run both on the same session to make a defensible quality claim.

---

# DeepSeek Harness has its own weaknesses

One is especially relevant to sctxx.

Its range selection is essentially **positional**.

A very recent issue points out that it can summarize material such as instruction/catalog context that will simply be re-injected later because the selection algorithm doesn't understand source semantics. ([GitHub][5])

And the deterministic tool-result pruner literally discards the middle:

```text
HEAD
...
[middle removed]
...
TAIL
```

The full original remains in the session log, but **the model cannot retrieve that omitted middle through the compacted surface** unless some other mechanism exposes it. ([GitHub][4])

Compare that to sctxx's idea:

```text
summary:
    "failed authentication migration [evt 51920–52781]"

agent needs detail
        ↓
sctxx expand 51920..52781
        ↓
original evidence
```

That retrieval property is genuinely powerful.

Harness compaction is primarily **lossy compression**.

sctxx is trying to be **lossy index + lossless retrieval path**.

That's a significant architectural advantage if implemented correctly.

---

# I would actually combine the two ideas

I think your next version of sctxx should steal a lesson from DeepSeek Harness—not its whole algorithm.

You currently have:

```text
                     sctxx
58M raw
  ↓
813K masked
  ↓
40 chunks
  ↓
40 premap calls
  ↓
~41 fold calls
  ↓
typed state
```

I would experiment with:

```text
                   sctxx vNext

RAW SESSION
      │
      ├──── deterministic Rust ledgers ─────────────┐
      │                                             │
      ↓                                             │
provider compaction checkpoints                     │
      +                                             │
episode segmentation                               │
      ↓                                             │
hierarchical structured checkpoint summaries       │
(DeepSeek-Harness-style schema)                    │
      ↓                                             │
5–10 semantic units instead of 40                  │
      │                                             │
      └──────────────┐                              │
                     ↓                              ↓
                FINAL RECONCILER ← deterministic truth
                     │
                     ↓
      Goal / Constraints / CurrentStep /
      Pending / Next / DeadEnds / Decisions
                     │
                     ↓
             repo verification
                     │
                     ↓
                 handoff.md
```

In other words:

**Use DeepSeek Harness's structured checkpoint format as a cheap hierarchical precompression stage, then use sctxx's deterministic ledgers/provenance/reconciliation to correct it.**

That could potentially get you the best of both systems:

* Harness's excellent **semantic checkpoint shape**
* Harness's hierarchical/rolling summarization intuition
* Codex's **evidence budgeting**
* sctxx's **deterministic truth**
* sctxx's **provenance**
* sctxx's **cross-agent retrieval**
* far fewer model calls

### My ranking

For **same-session continuous coding**:

**DeepSeek Harness: 9/10**
**Codex OSS local compaction: ~7/10 conceptually**
**sctxx: not intended for this task**

For **cross-agent handoff after a huge session**:

**DeepSeek Harness checkpoint alone: ~7/10**
**simple Codex compact summary: ~5–6/10**
**sctxx deterministic handoff: ~8/10 already**
**sctxx standard anchored fold as demonstrated by your 103K session: ~6/10 currently**

But the **architecture ceiling** of sctxx is higher for the latter problem. It just needs to prove that its expensive semantic fold beats the much simpler checkpoint approach.

And this suggests a very obvious benchmark for your OSS project: **add `baseline:deepseek-harness-compact` alongside `baseline:codex-compact` in `sctxx eval`.** That comparison would be extremely informative.

[1]: https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/compaction/compaction-basic/README.md?utm_source=chatgpt.com "deepseek-harness/packages/compaction/compaction-basic/README.md at master · deepseek-ai/deepseek-harness · GitHub"
[2]: https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/compaction.md?utm_source=chatgpt.com "deepseek-harness/docs/subsystems/compaction.md at master · deepseek-ai/deepseek-harness · GitHub"
[3]: https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/config-catalog.md?utm_source=chatgpt.com "deepseek-harness/docs/config-catalog.md at master · deepseek-ai/deepseek-harness · GitHub"
[4]: https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/compaction/compaction-tool-result-pruner/README.zh.md?utm_source=chatgpt.com "deepseek-harness/packages/compaction/compaction-tool-result-pruner/README.zh.md at master · deepseek-ai/deepseek-harness · GitHub"
[5]: https://github.com/deepseek-ai/deepseek-harness/discussions/5766?utm_source=chatgpt.com "[bug] Compaction summarises the skill catalog and AGENTS.md that the next step re-injects verbatim; a loaded skill body gets no such restore · deepseek-ai deepseek-harness · Discussion #5766 · GitHub"
[6]: https://github.com/deepseek-ai/deepseek-harness/discussions/3565?utm_source=chatgpt.com "[Bug] Manual /compact on a resumed session summarizes through the session's stale routed provider, with zero cache reuse · deepseek-ai deepseek-harness · Discussion #3565 · GitHub"
