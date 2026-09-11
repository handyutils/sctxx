I checked this more narrowly against the **actual OpenAI Codex source**, the published `sctxx` Rust crate, and the documentation your coding agent produced.

The verdict is:

**The agent did implement real Codex-derived Rust code. It is not fabricated. But saying “we extracted the Codex CLI compaction algorithm” is technically too strong and somewhat misleading.**

What was actually reused is substantial, but it is mainly **Codex's memory/extraction and context-selection machinery**, not the complete Codex compaction algorithm.

### What is definitely real

The published `sctxx` crate itself—not just your development log—declares that it contains code derived from OpenAI Codex at commit `818f1cca…`, specifically:

> UTF-8 truncation, secret redaction, tiered evidence budgeting, rollback-aware replay, and `apply_patch` parsing.

That attribution exists in the published crate and its `NOTICE`; the crate package includes the vendored Rust source. ([Docs.rs][1])

Even better, rustdoc exposes the actual derived Rust types. For example, `sctxx::vendor::codex::tiered_input::Tier` contains:

`User → AssistantFinal → Subagent → Commentary → PriorSummary → ToolResultError → ToolCall → ToolResultOk`

and explicitly says this ordering is the **fill order used by `select`**. So there really is implemented tiered-evidence selection code, not merely a document claiming there is. ([Docs.rs][2])

Your development log's later verification is therefore credible: it says the pinned Codex commit was checked and that `serialize_tiered_input`, its 2,000/10,000 caps, the compact prompt, `window_number`, and the relevant compaction sources existed at that pin. 

There is also direct evidence that the **LLM extraction prompt was genuinely derived from Codex source**. The published `FOLD_SYSTEM` in sctxx literally identifies:

`codex-rs/memories/write/templates/memories/stage_one_system.md`

at the pinned Codex commit and says it was rewritten from memory prose into typed handoff operations while preserving the evidence/hygiene rules. ([Docs.rs][3])

So, no: **the coding agent did not simply invent the Codex provenance.**

---

## But here's the important distinction

There are really **three different things** being mixed together under the word "compaction."

| Thing                                          | OpenAI Codex                                              | What sctxx did                   |
| ---------------------------------------------- | --------------------------------------------------------- | -------------------------------- |
| **Context compaction**                         | Shrink/replace conversation context so Codex can continue | **Not reproduced wholesale**     |
| **Memory Phase-1 extraction**                  | Extract useful structured knowledge from old rollouts     | **Substantially reused/adapted** |
| **Evidence budgeting / transcript processing** | Select important information under token limits           | **Ported/generalized in Rust**   |
| **sctxx anchored fold**                        | Doesn't exist this way in Codex                           | **Original sctxx design**        |

And that distinction matters a lot.

### Actual Codex compaction is surprisingly thin locally

The current OSS Codex local compaction prompt is literally nine lines. It asks the model to produce a handoff containing progress/decisions, context/constraints, remaining steps and critical references. 

More importantly, Codex does **not have one single local Rust “compaction algorithm.”**

The current dispatcher chooses among different implementations:

* token-budget compaction,
* remote compaction v2,
* remote compaction v1,
* or local model summarization. 

For the local path, the Rust code essentially constructs the compact prompt and runs the model:

```text
compact prompt
        ↓
run_compact_task(...)
        ↓
model creates summary
        ↓
summary replaces/re-anchors context
```

For OpenAI-hosted remote compaction, an important part of the behavior is on OpenAI's server. You cannot extract that private server-side algorithm from the OSS Codex repository.

So if your coding agent told you:

> “I extracted OpenAI Codex's production compaction algorithm from the Rust source and implemented that algorithm in sctxx.”

**I would call that inaccurate.**

There isn't one complete OSS production compaction algorithm there to copy.

---

# What the agent actually found — and this is arguably more useful

The valuable code it discovered is under **Codex Memories**, particularly Phase 1.

OpenAI describes that pipeline as:

```text
old rollout/session
      ↓
select eligible rollout
      ↓
filter to memory-relevant response items
      ↓
budget/select evidence
      ↓
LLM structured extraction
      ↓
raw_memory
rollout_summary
rollout_slug
      ↓
secret redaction
      ↓
persistent memory
```

That is real production machinery. 

And your own original specification actually says this quite correctly:

> Codex's compaction path is thin.

Then it identifies the **Memory Phase-1 extraction pipeline** as “the closest existing thing to sctxx's extractor,” along with tiered evidence selection, rollback replay, truncation, redaction and patch parsing. 

That's the accurate description.

The unfortunate part is that someone later titled a development-log entry:

> **“Codex compaction algorithm extracted”**



That headline oversells what happened.

A much better title would have been:

> **“Codex context-management and memory-extraction primitives ported; compaction reuse boundary established.”**

---

# What sctxx added itself

This is also important because the most interesting parts of your handoff system are actually **not from Codex**.

sctxx takes the Codex-derived substrate and builds this:

```text
Claude / Codex / Pi JSONL
        ↓
active branch reconstruction
        ↓
deterministic Rust ledgers
        ↓
masking + evidence classification
        ↓
episode segmentation
        ↓
chunks
        ↓
parallel pre-map
        ↓
STATEFUL ANCHORED FOLD
        │
        ├─ Goal
        ├─ Constraint
        ├─ Decision
        ├─ DeadEnd
        ├─ OpenThread
        ├─ CurrentStep
        └─ NextAction
        ↓
typed ADD / UPDATE / SUPERSEDE /
RESOLVE / MERGE / DROP operations
        ↓
Rust validation
        ↓
recency final pass
        ↓
repo reconciliation
        ↓
handoff.md
```

The typed `FoldState`, typed operations, supersession semantics, event-range provenance, validation and final recency reconciliation are your sctxx design. Your spec describes that architecture explicitly. 

That is **not how Codex's own context compactor works**.

And frankly, that's good. sctxx would be much less interesting if it simply duplicated Codex's nine-line “summarize this conversation” compactor.

---

## Was the Codex-derived algorithm actually *used*, rather than merely copied into `vendor/`?

There is strong evidence that **parts of it are actually in the extraction pipeline**, rather than dead vendored code.

The public crate describes its pipeline as deterministic-first and says Rust owns **branch resolution, ledgers, masking, segmentation, budgets, validation, apply, reconciliation and rendering**. ([Docs.rs][4])

The public `Tier` implementation explicitly refers to its fill order being used by `select`. ([Docs.rs][2])

And your spec says the Codex-derived tier budgeting is used when fitting masked transcript evidence into a bounded context, while the standard mode builds upon it with chunking and the anchored fold. 

So my confidence is high that **the Codex-derived evidence-selection machinery is real and operational**.

There is one level of proof I have **not** completed: a literal source-level call graph like

```rust
pipeline::extract()
   -> ...
   -> vendor::codex::tiered_input::select()
```

for the exact version that produced your 0.2.0 handoff. The published source/rustdoc gives us strong evidence, but I haven't line-by-line audited the complete repository checkout in this environment. So I wouldn't claim that final 100% proof yet.

---

# My verdict on your coding agent

I'd grade the claims this way:

| Claim from agent                                                          | Verdict                                      |
| ------------------------------------------------------------------------- | -------------------------------------------- |
| “We examined OpenAI Codex's Rust source”                                  | ✅ Supported                                  |
| “We pinned the exact upstream Codex commit”                               | ✅ Supported                                  |
| “We ported real Codex Rust code”                                          | ✅ Supported                                  |
| “We reused Codex tiered evidence budgeting”                               | ✅ Supported                                  |
| “We reused Codex rollback-aware reconstruction”                           | ✅ Supported                                  |
| “We adapted Codex's memory-extraction prompt methodology”                 | ✅ Very clearly supported                     |
| “These ideas are used to build the sctxx extractor”                       | ✅ Supported                                  |
| “We extracted **the Codex compaction algorithm**”                         | ⚠️ Overstated                                |
| “handoff.md is generated by the same algorithm Codex uses for compaction” | ❌ No                                         |
| “We reproduced OpenAI's complete production compaction implementation”    | ❌ Definitely no                              |
| “sctxx's anchored fold is Codex's algorithm”                              | ❌ No — it is mostly sctxx's own architecture |

So I **wouldn't accuse the coding agent of bullshitting about implementation**.

I **would accuse the documentation/title of imprecise engineering terminology**.

The cleanest truthful description of sctxx is:

> **sctxx ports selected Rust components and extraction techniques from OpenAI Codex's open-source memory/context-management pipeline—especially tiered evidence budgeting, rollback reconstruction, truncation, redaction and extraction-prompt rules—and combines them with an original provenance-preserving anchored handoff fold. It does not reproduce OpenAI's server-side Codex compaction algorithm.**

That's a stronger description anyway, because it's technically defensible.

The next thing I would do is **audit the actual `sctxx` call graph against the pinned Codex files function-by-function** and produce a table like `Codex source → sctxx port → modified how → called from → actually exercised by your handoff`. That would settle the “is it really used?” question at code level rather than documentation level.

I can also create a one-page diagram showing exactly which parts come from Codex and which are original sctxx.

[1]: https://docs.rs/crate/sctxx/0.1.3?utm_source=chatgpt.com "sctxx 0.1.3 - Docs.rs"
[2]: https://docs.rs/sctxx/latest/sctxx/vendor/codex/tiered_input/enum.Tier.html?utm_source=chatgpt.com "Tier in sctxx::vendor::codex::tiered_input - Rust"
[3]: https://docs.rs/sctxx/latest/sctxx/pipeline/fold/prompt/constant.FOLD_SYSTEM.html?utm_source=chatgpt.com "FOLD_SYSTEM in sctxx::pipeline::fold::prompt - Rust"
[4]: https://docs.rs/sctxx/latest/sctxx/?utm_source=chatgpt.com "sctxx - Rust"
