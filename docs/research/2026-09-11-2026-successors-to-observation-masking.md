# 2026 Successors to "The Complexity Trap" (Observation Masking)

**Question:** does anything published in 2026 challenge, refine, or supersede the 2025 result below, and is
any of it proven **and open source**? Also: does any published result show LLM summarization beats
selection + addressable retrieval when compacting a *finished* transcript for a *different* agent?

**Baseline checked.** *The Complexity Trap: Simple Observation Masking Is as Efficient as LLM Summarization
for Agent Context Management* — Lindenbauer, Slinko, Felder, Bogomolov, Zharov (JetBrains),
[arXiv:2508.21433](https://arxiv.org/abs/2508.21433). SWE-bench Verified, SWE-agent, 5 model configs; masking
(M=10) cheapest in 4/5 and matches/beats summarization on solve rate (Qwen3-Coder 480B: 54.8% @ $0.61 vs
53.8% @ $0.64 vs raw 53.4% @ $1.29). Reported limitations: one scaffold, one domain, non-adaptive triggers.

**Correction to the stated baseline.** The brief describes v1. Current is **v3 (27 Oct 2025)**, which already
added an **OpenHands probe** and a **hybrid strategy** ("reduces costs by 7% and 11% vs masking or
summarization") — so "one scaffold" and "no hybrid" were partly answered in 2025. Numbers are quoted from source.

## 1. Candidate findings (2026)

### 1.1 AttnCompress — the only direct masking-vs-summarization test on SWE-bench Verified in 2026
*AttnCompress: Dynamic Attention-Guided Trajectory Compression for Software Engineering Agents* — Zeng, Li,
Xie, Ye, Zhang (Peking Univ.), **ISSTA 2026**, [arXiv:2609.08318](https://arxiv.org/abs/2609.08318). Code
`github.com/ZZR0/AttnCompress` (Trae-Agent artifact + raw trajectories). **No LICENSE file → publicly
released but not open source** (paper is CC BY). Its Table 1, per-instance mean over **SWE-bench Verified,
3 agent LLMs** (Gemini-3-Flash, Qwen3-235B, Qwen3-Coder-30B), cites Lindenbauer et al. 2025 as **ObsMask**:

| Method | Pass (%) | Input (k) | Total cost ($) | Steps |
|---|---|---|---|---|
| Original (full context) | **55.17** | 1118.16 | 0.1189 | 45.93 |
| **LLMSummary** | **50.83** | 810.52 | 0.1126 | 52.54 |
| **ObsMask** (the baseline) | **47.17** | 628.93 | **0.0443** | **63.47** |
| AgentDiet | 51.17 | 822.44 | 0.1429 | 48.12 |
| AttnCompress (dynamic window + recall) | **53.17** | 644.66 | 0.0949 | 52.43 |

This **contradicts the baseline's headline on solve rate**: ObsMask is still cheapest ($0.0443 vs $0.1126)
but **3.66 points *worse* than LLMSummary**, and it **inverts the trajectory effect** — masking runs longest
(63.47 steps), summarization shorter (52.54). Not verified: whether this ObsMask used M=10 — the paper says
only "removes observations older than a fixed window".

### 1.2 SWE-Pruner — summarize vs embedding-RAG on SWE-bench (MIT)
*SWE-Pruner: Self-Adaptive Context Pruning for Coding Agents* — Wang, Shi, Yang, Zhang, He, Lian, Chen, Ye,
Cai, Gu, [arXiv:2601.16746](https://arxiv.org/abs/2601.16746). Code `github.com/Ayanami1314/swe-pruner`,
**MIT**. Trained 0.6B "neural skimmer" — a *learned* trigger, the baseline's stated future work. On SWE-bench
Verified, Mini SWE Agent + Claude Sonnet 4.5: baseline **70.6% (353/500)** @ 0.911M tokens; + SWE-Pruner
**72.0% (360/500)** @ 0.701M (−23.1% tokens, −26.8% cost). Its Table 3: full context 62.0% / 0.972M;
**LLM Summarize 56.0% / 0.794M; RAG 50.0% / 0.771M**; LLMLingua2 54.0%; LongCodeZip 54.0%; SWE-Pruner
64.0% / 0.670M — here **summarization beats embedding retrieval by 6 points**. Authors' stated scope limit:
*"We did not compare with agent history compression methods [71,25]… our method compresses agents'
observations like repository content"* — i.e. **not** trajectory/handoff compaction.

### 1.3 SelfCompact — adaptive, model-triggered compaction (MIT), the learned-trigger answer
*Self-Compacting Language Model Agents* — Li, Zhang, Jurayj, Wang, Jin, Farajtabar, Nalisnick, Khashabi
(JHU/Apple), [arXiv:2606.23525](https://arxiv.org/abs/2606.23525). Code `github.com/tianjianl/selfcompact`,
**MIT**. Six benchmarks (IMO-Answerbench, HMMT Nov 25/Feb 26; BrowseComp, BrowseComp-Plus, DeepSearch QA),
7 models. "Matches or exceeds fixed-interval summarization at a fraction of the token cost… up to **18.1
points on math and 5–9 points on agentic search at 30–70% lower per-question cost**." **The delta baseline is
no-compaction, not masking**; under matched token budgets it beats fixed-interval summarization in 11 of 12
settings (exception Qwen3-30B-A3B on HMMT Feb, −1.1).

### 1.4 CompactionRL — RL-trained compaction, numbers on SWE-bench Verified
*CompactionRL: Reinforcement Learning with Context Compaction for Long-Horizon Agents* — Li, Hou, Jing, Tang,
Dong, [arXiv:2607.05378](https://arxiv.org/abs/2607.05378). **No dedicated code repo found**; cites *slime*
and a Zenodo artifact "Harbor…" (**Apache-2.0**). GLM-4.5-Air **66.8% Pass@1 on SWE-bench Verified (+7.0)**;
GLM-4.7-Flash **56.0% (+5.5)**. Baselines are the untrained models, not masking. I **could not verify** that
training code or licenses shipped.

### 1.5 Masking regime map (search agents) — strongest *challenge*, different domain
*Masking Stale Observations Helps Search Agents — Until It Doesn't: A Regime Map and Its Mechanism* — Zhang,
Xu, Li, Zhang, Jiang, Zhang, McAuley (UCSD/Berkeley/TAMU/UIUC),
[arXiv:2606.00408](https://arxiv.org/abs/2606.00408). Code `github.com/i-DeepSearch/observation-masking` —
**no LICENSE file → not open source**; HF eval logs released. 4B–284B backbones, 3 retrievers,
BrowseComp-Plus + GAIA / xBench-DeepSearch / BrowseComp-ZH. Gain is an inverted-U: **+6.2 to +6.6** under
BM25, peak **+11.7** (Qwen3.5-35B-A3B + AgentIR, recall 0.88, No-CM 62.9%), collapse above ~70% No-CM —
Tongyi-DeepResearch-30B-A3B **−1.1** (80.7% No-CM, 0.93 recall); GPT-OSS-120B **+0.1** offline but **−4.8 on
GAIA**. Tool calls surge: **+68.7/query** (GPT-OSS-120B), **+57.7** (DS-V4-Flash-Max). Attention: **53.7%**
to self-generated reasoning vs **25.6%** to tool observations. **Its comparison baseline is
no-context-management, not summarization**, so it does not directly test the Complexity Trap — but it
falsifies "masking is safe" as a general law.

### 1.6 Negative/limiting results worth recording (no code found for any)
- *What Does Context Compression Cost an Agent?* — Shuyu Liu,
  [arXiv:2608.16370](https://arxiv.org/abs/2608.16370). Deterministic planning env (24-turn horizon) +
  ALFWorld, 3 models. Completion statistically unchanged while **retrieval calls rise in all six
  model–regime cells** (5/6 survive Holm): GPT-5.5 completion **80%→85% (p=1.0)** while retrieval goes
  **21.0→63.9 calls (p=.002)**; "random selection is comparable to an offline hindsight oracle."
- *Measure Before You Manage* — Le Chen et al., [arXiv:2608.31057](https://arxiv.org/abs/2608.31057). 55
  archived trajectories, claude-opus-4.8, four rungs (raw/compressed/**summary**/**pointer** — pointer is
  `recall_object`-recoverable, i.e. addressable retrieval). Object-aware compression beats FIFO on
  calibration (−1.633 calls, Holm p=0.0146) but **not on held-out (−0.500, Holm p=0.5000)**; the retrieval
  policy shows **no** significant contrast on any baseline (all Holm p ≥ 0.875).
- *VISTA / LLM Agents Are Latent Context Managers* — Xu, Li, Zhang,
  [arXiv:2606.30005](https://arxiv.org/abs/2606.30005). Typed addressable blocks + dashboard + recoverable
  archive; LOCA-Bench Gemini-3-Flash **22.7→50.7%**, BrowseComp-Plus **58.0%**. **I could not verify a code
  release** — its only release language refers to *other* systems not releasing code. Also present, not
  fully verified: CoACT (MIT; −33.0% tokens on SWE-bench Verified), Agent-Omit.

## 2. Does anything supersede it?

**No — not cleanly, and not by an open-source result.**

1. **No 2026 open-source work re-runs the baseline's own experiment.** The only paper re-testing masking
   against LLM summarization on SWE-bench Verified is AttnCompress, and it **disagrees** (47.17% vs 50.83%).
   But its code is **unlicensed**, its scaffold is Trae Agent (not SWE-agent), the masking config is
   unspecified, and the authors ship a competing method. By the "proven *and* open source" bar it does not
   supersede; by the "challenges" bar it is the **single strongest 2026 challenge**.
2. **The limitations 2026 moved are the baseline's own.** Non-adaptive triggers → SelfCompact (MIT) and
   SWE-Pruner (MIT) supply adaptive/learned triggering. One domain → the regime map shows the effect is
   *regime-dependent*, but on deep search with a no-CM baseline.
3. **Nothing refutes the cost claim, and open-source successors improve the framing rather than overturn
   it.** Masking is cheapest in every 2026 table ($0.0443 vs $0.1126; baseline $0.61 vs $0.64); only the
   **solve-rate parity** is contested, and only by an unlicensed artifact. SWE-Pruner declines to compare
   against agent-history compression and SelfCompact's baseline is no-compaction — neither measures itself
   against M=10 masking.

Verdict: **refined and narrowed, not superseded** — treat "masking ≈ summarization on solve rate" as scaffold-, model-, and domain-specific; treat "masking is cheapest" as durable.

## 3. The handoff question

**Is there any published result showing compaction quality for a *finished* transcript handed to a *different*
agent is improved by an LLM summary over selection plus addressable retrieval?**

**No. I could not find any such result, and the absence is the finding.** The closest work:

- **Handoff Debt (arXiv:2606.02875, MIT, SWE-bench Verified, OpenHands-style; 75 source tasks → 181
  handoff-point tasks, 724 takeover runs per successor, 3 successors).** Four views: repository only, raw
  trace, **summary notes**, **structured notes** — **no addressable-retrieval arm at all**. Qwen→Qwen:
  repo-only 46.4% / 99 events / 1.63M; raw **52.5% (+6.1pp) / 41 (−59%) / 811k (−50%)**; summary
  **51.4% (+5.0pp) / 53 (−46%) / 602k (−63%)**; structured **50.8% (+4.4pp) / 55 (−44%) / 660k (−60%)**.
  Qwen→Devstral: repo-only 34.3% / 175 / 3.94M; raw **49.2% (+14.9pp) / 73**; summary **43.6% (+9.4pp) / 123**;
  structured **44.8% (+10.5pp) / 125**. Handoffs cut median events 20–59% and tokens 42–63%, but
  **solved-rate gains are "smaller and model-dependent"** (note-based gains for Qwen and Gemma successors
  are **not significant at α=0.05**), and *"Raw traces, summaries, and structured notes do not produce one
  universal ranking."* It **cannot** order summary vs structured selection and never tests retrieval-backed
  recovery.
- **SWE-Pruner Table 3** *does* put LLM Summarize (56.0%) ahead of RAG (50.0%) on SWE-bench — but that is
  **observation-content** compression with embedding chunk retrieval, explicitly *not* trajectory or handoff
  compaction; not evidence about handing a finished transcript to a new agent.
- **Measure Before You Manage** *does* compare a compression ladder ending in `summary` against a
  pointer/`recall_object` retrieval policy — inside a live loop, not a handoff — and finds **neither**
  survives held-out correction.

Honest design conclusion: **no published evidence supports preferring an LLM summary over selection +
addressable retrieval for cross-agent handoff**; the open-source cross-agent study found no stable ordering
between summary and structured selection and shipped no retrieval arm.

## 4. What a Rust CLI should take from this

1. **Keep masking/selection as the default and the floor, and claim cost — not accuracy.** Cheapest arm in
   every 2026 table; no open-source result overturns that, while solve-rate parity is at best split. Do not
   adopt summarization on AttnCompress alone — unlicensed, contestable.
2. **Bind artifact fidelity; make recovery addressable and cheap.** Handoff Debt's mechanism is not
   "summarize better" — dropped state must be cheaply re-obtainable (masking pays in re-search;
   arXiv:2608.16370 measures retrieval calls 21.0→63.9). sctxx's `[evt a–b]` citations and `sctxx expand`
   are that primitive; MBM's `pointer` rung independently confirms it.
3. **Make triggering adaptive, not fixed-interval.** SelfCompact (MIT) and SWE-Pruner (MIT) both beat fixed
   thresholds ("typically fires too late"). Ship deterministic triggers now, leave a seam for a learned one.
4. **Report interaction cost, not just tokens.** Compression shifts work into extra tool turns that completion metrics hide (masking 63.47 vs 45.93 steps); track steps and recall calls too.

**Not verified:** the ObsMask window size in AttnCompress; whether CompactionRL released training code; any
code release for VISTA (2606.30005), MBM (2608.31057) or 2608.16370; Agent-Omit's license; whether any 2026
work evaluates OpenHands condensers specifically against M=10 masking.
