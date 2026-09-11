# Who actually has the best proven results, and what should sctxx take

Status: answer to "research who has the best results so far, proven, and OSS, and let's take that best
algorithm". Reading of the evidence only — the implementation decisions are in ADR 0008 and the
CHANGELOG.

---

## 1. The short answer

**There is no best-proven algorithm for what sctxx does, because nobody has benchmarked it.**

For context management *inside a live agent loop* there is exactly one strategy with an ablation
behind it — **observation masking** — and even that has been narrowed by a 2026 re-test. For
**a finished transcript handed to a different agent**, which is sctxx's problem, the closest
published work has no arm that tests what sctxx does, and no result anywhere shows an LLM summary
beating selection plus addressable retrieval.

So the honest plan is not "adopt the winner". It is: **take the one proven mechanism, take the one
concrete production rule that is portable, and then run the benchmark nobody has run** — because
sctxx is unusually well placed to run it, and its constitution already commits it to the answer that
the evidence weakly favours.

---

## 2. What is proven, in a live loop

### Observation masking — the only strategy with an ablation

*The Complexity Trap: Simple Observation Masking Is as Efficient as LLM Summarization for Agent
Context Management* — Lindenbauer, Slinko et al., JetBrains Research,
[arXiv:2508.21433](https://arxiv.org/abs/2508.21433) (v3). SWE-bench Verified, SWE-agent scaffold,
five model configurations, 95 % bootstrap intervals.

| model | strategy | solve rate | cost/instance |
| --- | --- | --- | --- |
| Qwen3-Coder 480B | raw | 53.4 ±4.3 | $1.29 ±0.26 |
| | **observation masking** | **54.8 ±4.4** | **$0.61 ±0.06** |
| | LLM summary | 53.8 ±4.2 | $0.64 ±0.06 |
| Gemini 2.5 Flash (thinking) | raw | 40.4 ±4.3 | $0.56 ±0.10 |
| | **observation masking** | 36.4 ±4.2 | **$0.24 ±0.04** |
| | LLM summary | 31.4 ±4.0 | $0.25 ±0.05 |

- Masking was cheapest in **4 of 5** configurations and matched or beat summarisation on solve rate
  in 4 of 5.
- Environment observations are **~84 %** of an average SWE-agent turn, which is why targeting them
  alone is so effective.
- Summarisation generation is **2.86–7.2 %** of instance cost, and those calls are cache-hostile.
- The **trajectory-elongation effect**: summarisation lengthens trajectories ~15 %, apparently by
  smoothing over the failure signals that would have ended a bad attempt early.
- **Critic-enhanced summarisation is no better and ~25 % more expensive**, with ~13 % longer
  trajectories.
- The window is not a constant: SWE-agent's optimum was **M = 10 turns**, OpenHands' probe in the
  same paper needed **M = 58**. It is "an agent-specific hyperparameter that requires tuning".
- Limitations the authors state: one scaffold, one domain, and only non-adaptive triggers.

### The 2026 re-test narrows the parity claim

*AttnCompress* (ISSTA 2026, [arXiv:2609.08318](https://arxiv.org/abs/2609.08318)) is the only 2026
work re-testing masking against LLM summarisation on SWE-bench Verified. Mean over three agent LLMs:

| | solve rate | cost |
| --- | --- | --- |
| full context | 55.17 % | — |
| AttnCompress | 53.17 % | $0.0949 |
| LLM summary | 50.83 % | $0.1126 |
| observation masking | 47.17 % | **$0.0443** |

**Masking is still cheapest by 2.5×, but it is 3.66 points worse than summarisation** — which
contradicts the 2025 parity claim — and it now runs *longest* (63.47 vs 52.54 steps), inverting the
trajectory-elongation finding. Caveats that matter: the repository has **no LICENSE file**, the
scaffold is Trae Agent rather than SWE-agent, the masking window is unspecified, and the authors ship
a competing method. Treat it as a warning, not a verdict.

A second 2026 paper maps the regime: masking has an inverted-U, peaking at **+11.7 points** for
search agents and collapsing to **−4.8 on GAIA**. The trigger matters more than the mechanism.

### What is *not* proven, and this is the more useful list

- **No OpenHands-authored ablation exists for any condenser.** The only number is an April-2025 blog
  post: 54 % vs 53 % on "a subset" of SWE-bench Verified, with no instance count, model, config, or
  raw data. The SDK technical report cites it and contains the word "ablation" zero times.
- **OpenHands' current SWE-bench harness hard-codes the condenser on**
  (`benchmarks/swebench/config.py`, `enable_condenser: True`, `condenser_max_size: 240`,
  `condenser_keep_first: 2`) and exposes `--disable-condenser`; no published run reports the flag off.
  The advertised score is a with-condenser score and nobody knows the without-condenser score.
- **Codex's compaction has no evaluation at all.** Its `compact.rs` tests are wiremock request-shape
  and insta snapshot tests; there are zero score strings.
- **SWE-agent, Aider, Cline, Roo Code, Goose and Continue attribute no benchmark number to their
  compaction method.**
- **CompactionRL** ([arXiv:2607.05378](https://arxiv.org/abs/2607.05378)) reports the best numbers in
  the field — +7.0 points on SWE-bench Verified for GLM-4.5-Air, 59.8 → 66.8 — and is **unusable**:
  no checkpoint is published anywhere (GitHub releases: 0; Hugging Face: 0), the reference repo is an
  8-GPU slime/Megatron/Ray/SGLang training harness whose own reproduction script targets a different
  task, and the released shards are 0-byte placeholders. It is evidence about how to *train* a model,
  not an algorithm a binary can run.

**Nothing is proven better than a tuned recency window.**

---

## 3. What is proven for sctxx's actual problem — a handoff

Almost nothing, and the near-miss is instructive.

*Handoff Debt* ([arXiv:2606.02875](https://arxiv.org/abs/2606.02875), MIT) is the closest published
work: SWE-bench Verified, **181 handoff tasks, 724 runs across 3 successor agents**, comparing
repo-only, raw trace, summary notes, and structured notes. Its finding is that "raw traces,
summaries, and structured notes do not produce one universal ranking" — the solved-rate gains for
notes were not significant at α = 0.05 for 2 of the 3 successors.

**It has no addressable-retrieval arm.** No condition where the successor can ask for the part of the
transcript it needs. That is precisely the arm sctxx implements.

Two adjacent results, neither of which settles it:

- **SWE-Pruner** (MIT, [arXiv:2601.16746](https://arxiv.org/abs/2601.16746)): LLM Summarize 56.0 % vs
  RAG 50.0 % on SWE-bench — but this is *observation-content* compression inside a loop, explicitly
  not a handoff, and the retrieval arm there has no pointers into the original trajectory.
- **Measure Before You Manage** ([arXiv:2608.31057](https://arxiv.org/abs/2608.31057)) compares a
  summary rung against a `recall_object` pointer policy inside a live loop and finds **neither**
  survives held-out correction.

### The absence, stated plainly

**No published result shows an LLM summary beating selection plus addressable retrieval for a
finished transcript handed to a different agent.** That absence is the finding. It does not make
sctxx right; it means sctxx's design is untested rather than contradicted, and that the tool is
making a claim no one has measured — including sctxx itself.

---

## 4. What to take, ranked

1. **Keep selection as the default, and claim cost rather than accuracy.** The 2025 parity claim is
   narrowed by the 2026 re-test. The defensible claim is "about 2.5× cheaper, with everything still
   reachable", not "as good".
2. **Port Codex's window rule.** `build_compacted_history` in `codex-rs/core/src/compact.rs` keeps the
   most recent **≤ 20,000 tokens of real user messages** verbatim — newest first, and **the message
   that does not fit is middle-truncated rather than dropped** — then appends one summary last, with
   `is_summary_message()` preventing summaries from accumulating. This is production code, it is
   Apache-2.0, sctxx already vendors from the same pin, and it matches sctxx's rule that user turns
   are the evidence. *Implemented: `segment::digest` now truncates its boundary row.*
3. **Make recovery addressable and cheap.** Done: `sctxx expand` pages in exact non-overlapping
   chunks, and L3 indexes every episode the artifact did not carry. This is sctxx's only defensible
   answer to a masked region, and it is the arm Handoff Debt lacks.
4. **Trigger adaptively, not on a fixed interval.** Both the baseline's own future work and the 2026
   regime map point the same way; **SelfCompact** (MIT,
   [arXiv:2606.23525](https://arxiv.org/abs/2606.23525)) reports +18.1 points on math and 5–9 on
   search versus no compaction at 30–70 % lower cost from a learned trigger. sctxx currently folds
   unconditionally.
5. **Adopt Cline's stated principle, because it is sctxx's too.** `basic-compaction.ts` forces
   deterministic compaction on overflow because *"recovery must not depend on another successful LLM
   request"* — the same rule as ADR 0007, arrived at independently, and worth citing in the README.
   Cline also always preserves typed user prompts, with `DEFAULT_PRESERVE_RECENT_TOKENS = 20_000` and
   `COMPACTION_TRIGGER_RATIO = 0.9`.
6. **Tag-based masking beats position-based masking.** SWE-agent's `LastNObservations` supports
   `always_remove_output_for_tags` / `always_keep_output_for_tags`. sctxx masks by row kind and tier,
   which is the same idea; making the tag explicit would let a reader say "never mask test output".
7. **Note what Codex ships behind a flag.** `compact_token_budget.rs` drops the transcript and
   reinstalls context with `message: String::new()` — *"Token-budget compaction skips model/server
   summarization and installs a fresh context window instead."* A frontier lab ships "keep the world
   state, discard the transcript, no summary". It has no evaluation either, but it is the strongest
   available signal that the deterministic-first posture is not a compromise.

## 5. What not to take

- **CompactionRL** — no checkpoint, 8 GPUs, and no head-to-head against any competing compaction
  algorithm, so "best proven" is not establishable from it even in principle.
- **AttnCompress** — no licence, unspecified window, and it is a model-side attention intervention
  that a Rust binary cannot host.
- **OpenHands condensers** — LLM-driven, and shipped without an ablation for any of them.
- **The specific numbers from any live-loop paper** as if they transferred to a handoff. They do not;
  the consumer's ability to re-obtain a masked observation is the whole mechanism, and a handoff
  reader has no tools.

## 6. The thing sctxx should do that nobody has

Run the handoff benchmark. `Handoff Debt` supplies the method — a fixed task set, a real repository,
several successor agents, and a significance test — and has no addressable-retrieval arm. sctxx can
supply that arm and can run it on its own artifact, with and without the fold, which also produces
the first honest number for whether the fold is worth its cost in a handoff at all.

Until that exists, the fold is an expensive unmeasured hypothesis and the deterministic artifact is
the product.
