# Candidate: CompactionRL (context compaction via RL) — sctxx applicability research

- Date: 2026-09-11
- Candidate: **CompactionRL: Reinforcement Learning with Context Compaction for Long-Horizon Agents**
- Paper: [HF papers/2607.05378](https://huggingface.co/papers/2607.05378) · [arXiv abs](https://arxiv.org/abs/2607.05378) · [arXiv HTML v1](https://arxiv.org/html/2607.05378v1) (arXiv:2607.05378v1 [cs.LG], 06 Jul 2026; Li, Hou, Jing, Tang, Dong — Tsinghua / Z.AI)
- Claimed reference implementation: [ukpkhkk/SAO_and_CompactionRL](https://github.com/ukpkhkk/SAO_and_CompactionRL) (shallow clone `/tmp/crl`, commit `ebc3393`, message `remove git`)
- Consumer assumed here: `sctxx` — offline, one-shot, laptop-side, no GPU, no live agent loop, no environment to re-query.

## 1. What the algorithm actually is

CompactionRL is **not a compactor**; it is a PPO training recipe whose purpose is to make one LM good at
*being compacted*. It is defined over an **interactive rollout**, not over a finished transcript.

- **What is compressed.** `h_t = (s, u, z_1…z_t)` with `z_i = (a_i, o_i)` an assistant action plus its
  environment observation, treated as **atomic** so a tool call is never split from its feedback (Eq. 6).
- **When.** When remaining budget falls under a threshold: `C − |h_t| < T_comp` (Eq. 7). **Discrepancy:** the
  released code triggers on *absolute* context length — `generate_with_retool.py:752-757` computes
  `normal_context_len_after_obs >= compaction_trigger_len`.
- **By what.** The summarizer is **the same trainable policy**: `S_t ~ π_θ(· | h_t ⊕ q_sum)` (Eq. 8). The
  only fixed parts are the trigger threshold, the summary instruction `q_sum`, the resume template, and `k`.
- **Resume.** `h̄_t = (s) ⊕ u_resume(S_t) ⊕ (z_{t−k+1}…z_t)`, `k = 2` by default (Eq. 9) — recent
  environment state is kept verbatim, not summarized.
- **What the model is trained to produce.** Both execution tokens and summary tokens under **one shared task
  reward** `R(τ)`. No separate summary-quality reward: "hand-designed summary metrics may not reflect which
  details are useful for solving the task."
- **Objectives.** Token-level loss normalization over all optimized assistant tokens (Eq. 12) instead of
  segment-level averaging, and **cross-trajectory GAE** `Â_{s,i} = (γλ)^{N_{>s}} · A^loc_{s,i}` (Eq. 14),
  discounting earlier segments by the trainable tokens generated after them. PPO + critic, not GRPO, because
  compaction makes the segment count per rollout variable.

**Is the compaction policy learned?** Yes — the compaction behaviour *is* the trained policy's weights.
There is no separate learned compactor and no deterministic compaction algorithm in the paper. What is
deterministic is the harness: threshold, prompt text, resume template, keep-`k`.

## 2. Proven numbers

**Harbor** environment, **Terminus-KIRA** scaffold, **Pass@1 (%)**; SWE-bench Verified on a **random
200-instance subset** (public baselines are full-benchmark), Terminal-Bench 2.0 full set; temperature 1.0,
up to 250 turns, at most 3 compactions; **mean of 2 evaluation runs**.

| Model | Peak len | SWE-V Single (×1) | SWE-V Compacted (×4) | TB2 Single (×1) | TB2 Compacted (×4) |
| --- | --- | --- | --- | --- | --- |
| GLM-4.7-Flash (30B-A3B) | 64k | 47.5 | 50.5 | 14.6 | 13.4 |
| + RL (w/o compaction) | 64k | 50.0 | 48.0 | 16.9 | 12.4 |
| **+ CompactionRL** | 64k | 43.7 | **56.0** | 16.9 | **20.2** |
| GLM-4.5-Air (106B-A30B) | 80k | 57.8 | 59.8 | 17.9 | 21.4 |
| + RL (w/o compaction) | 80k | 58.3 | 62.5 | 20.2 | 23.6 |
| **+ CompactionRL** | 80k | 57.3 | **66.8** | 21.4 | **24.5** |

- The headline gain is against **the same base model with inference-time compaction** (self-summarization,
  no training), not against a long-context model: +5.5 SWE-V / +6.8 TB2 at 30B-A3B, +7.0 / +3.1 at
  106B-A30B. Single-window (×1) either drops or barely moves — the paper calls this a train–test mismatch.
- Ablation (Table 3), 106B: `+ CompactionRL (w/o summary training)` 64.5 compacted SWE-V vs **66.8** with
  summary training; 30B: 54.5 vs **56.0**. `+ RL (w/o compaction)-160k` reaches 64.5 SWE-V, so the 106B
  advantage over a *longer-context* no-compaction model is +2.3 points, not +7.0.
- **Component swap on a fixed execution model (Table 1)** — the only result isolating compaction from the
  trained execution policy. Execution agent fixed at GLM-4.7-Flash; only the summary agent changes:
  Qwen3.5-27B → 55.5, GLM-4.7-Flash → 50.5, Qwen3-30B-A3B → 49.0 SWE-V. **A 6.5-point spread from
  summarizer choice alone**, with no RL on the summarizer.
- **Every main result is "their model + their compaction."** No result applies a CompactionRL-trained
  summarizer to a third-party execution model. There is also **no head-to-head against any other compaction
  algorithm**: SUPO, ReSum, Context-Folding appear in Related Work only, with no shared benchmark numbers.
  "Best proven compaction algorithm" therefore cannot be established from this paper.
- Verification limits: 2 runs, 200-task SWE-V subset, self-reported; public baseline rows are borrowed from model cards with different scaffolds (the paper says so).

## 3. What the released code actually is

The repo is a **research fork of [slime](https://github.com/THUDM/slime)** (Megatron + SGLang + Ray) with an
in-tree `slime/` package (`setup.py` version `0.3.0`), squashed to one commit. It is a **training harness**,
and it does contain a real CompactionRL implementation — not just prose.

| Concern | Path |
| --- | --- |
| Rollout-side compaction loop | `examples/retool/generate_with_retool.py` |
| Summary prompt / resume template | `generate_with_retool.py:61-71` |
| Cross-trajectory GAE | `slime/utils/ppo_utils.py:707-760` ("Compute CompactionRL segment-local GAE with cross-segment correction") |
| Reward routing / segment modes | `slime/backends/megatron_utils/loss.py:859-884` |
| Segment → training rows, `n_trainable_after` | `slime/ray/rollout.py:718`, `:811-820` |
| CLI surface | `slime/utils/arguments.py:1139-1164` |
| Only CompactionRL entry point | `scripts/run-qwen3-4B-compactionrl.sh` |
| Standalone summary-turn replay | `scripts/debug_compaction_summary.py` + `scripts/run-debug-compaction-summary-sglang.sh` |

```python
# examples/retool/generate_with_retool.py:61 — the "compaction policy spec" that is not learned
DEFAULT_COMPACTION_SUMMARY_PROMPT = (
    "The context is too long. I need to compact the previous problem-solving history now.\n"
    "I should output only a concise state summary needed to continue solving the same problem.\n"
    ...
)
```

```python
# slime/utils/ppo_utils.py:744-755 — the paper's Eq. 14
n_after = int(compaction_meta.get("n_trainable_after", 0) or 0)
factor = (gamma * effective_lambd) ** n_after
corrected_adv = adv * factor
```

`scripts/run-qwen3-4B-compactionrl.sh` shows what "using it" means: `--enable-compaction-rl`,
`--compaction-max-context-len 16384`, `--compaction-trigger-len 12000`, `--compaction-max-count 3`,
`--compaction-recent-steps 2`, `--compaction-summary-max-new-tokens 2048`,
`--compaction-segment-reward-mode paper_each_segment`, `--num-critic-only-steps 50`,
`--calculate-per-token-loss`, on 8 GPUs (4 actor + 4 rollout, TP=2) with Ray, Megatron-LM, SGLang, Python
3.12, Linux + CUDA (`README.md:36-58`, `build_conda.sh`).

**The released experiment does not reproduce the paper.** The CompactionRL script targets **Qwen3-4B on math
TIR**: `dapo-math-17k.jsonl` train, `aime-2024.jsonl` eval, a stateless Python `code_interpreter` sandbox
and a `\boxed{}` answer reward (`run-qwen3-4B-compactionrl.sh:107,125`). The paper's results are
**GLM-4.7-Flash and GLM-4.5-Air-SFT on SWE-bench Verified / Terminal-Bench 2.0**. No script, config, or data
path in the repo targets the GLM models or those benchmarks; the SWE example that does exist
(`examples/coding_agent_rl/run_qwen36_35b_a3b_swe_8nodes.sh`) never passes `--enable-compaction-rl` (the
only two files containing that flag are `arguments.py` and the Qwen3-4B script).

**Checkpoint status: none published.** The GitHub releases API for the repo returns `[]` (0 releases), and
an HF model search for `CompactionRL` returns 0 results. `qwen3_4b_vanillappo_checkpoints/` has 33 tracked
files, but the shards are **0-byte placeholders** (`iter_0000379/__0_0.distcp` … `common.pt`) for a
*Qwen3-4B vanilla PPO math* run — not a CompactionRL checkpoint, and not loadable. The only substantive
committed artifacts are math rollout/eval JSONL and `case.json` / `summary_debug.json` for a geometry
problem ("sphere, `h,k,l,R`"), confirming the 4B math setting. Checkpoint size cannot be quoted because
nothing is distributed; the paper's models are 30B-A3B and 106B-A30B, and the repo tells the reader to
supply their own (`MODEL_DIR="/mnt/workspace/models/font-info/qwen3-4b-sft"`).

## 4. Is the compaction step separable from the RL training?

**The RL machinery: no.** Cross-trajectory GAE is defined over per-token critic values and per-segment loss
masks, with `n_trainable_after` computed during rollout collection (`rollout.py:811-820`) and consumed in
the Megatron loss (`loss.py:895-898`). It cannot be lifted into a Rust binary or any non-Megatron consumer.

**The compaction behaviour: yes, in principle — as a prompt + sampling call.** Two pieces of code show the
boundary is text, not framework:

1. `slime/utils/arguments.py:1162-1163` — `--compaction-summary-prompt-path` and
   `--compaction-resume-template-path` load the summary instruction and resume template from **plain text
   files**; the prompt carries no learned parameters.
2. `scripts/debug_compaction_summary.py:205` — `--backend choices=["prompt-only", "sglang",
   "transformers"]`. The script reconstructs the exact summary prompt from a recorded rollout trace
   (`extract_summary_prompt`), re-derives the training sampling params, then calls either an **SGLang
   `/generate` HTTP endpoint** (`call_sglang:118`) or plain **HF `transformers` `model.generate`**
   (`call_transformers:166`) — no Ray, Megatron, critic, or optimizer involved.

Four conditions break that in practice for `sctxx`: (i) what makes the summary good is a **checkpoint
trained by this pipeline, and none is published** — the available GLM weights are the *base* models, and the
paper never says Z.AI serves a CompactionRL variant; (ii) the prompt must be re-rendered in the training
tool/chat template and tokenized with that tokenizer; (iii) the models are 30B/106B, far off a laptop;
(iv) the prompt is written for *mid-rollout* resumption ("I should continue solve the same problem.
Remember each code_interpreter call is stateless"), not for a finished-session handoff.

Portable today with no checkpoint and no GPU: the summary content checklist (original goal, key facts/tool
results, failed attempts, unresolved errors, current state, next plan), keeping the last `k=2` steps
verbatim rather than summarized, atomic action+observation units, token-budget accounting, and the
empirical claim that summarizer choice can swing outcome accuracy ~6.5 points — an argument for spending
`sctxx`'s optional LLM pass on summary quality rather than on more deterministic passes.

## 5. Licence

- **Code:** Apache-2.0 (`LICENSE` is the Apache 2.0 text; GitHub API reports `spdx_id: apache-2.0`). Vendored upstream slime is also Apache-2.0, so no conflict with `sctxx` — but nothing worth vendoring.
- **Paper:** the HTML carries "License: arXiv.org perpetual non-exclusive license" — paper text only.
- **Checkpoint:** **none published, so no checkpoint licence exists.** No CompactionRL-specific weight terms could be verified; the GLM-4.5-Air / GLM-4.7-Flash weights are not distributed in this repository.

## 6. Cold verdict

**(b) Usable only as a design idea — not directly usable.**

CompactionRL is a reinforcement-learning *training* method for an interactive agent, not a compaction algorithm executable over a finished transcript: its compaction policy is the trained policy's weights, its trigger and resume logic presuppose a live environment and a budget being spent turn-by-turn, and its cross-trajectory GAE is inseparable from the Megatron/Ray/SGLang training loop.
For `sctxx` it is disqualified three times over — the algorithm needs an interactive rollout that does not exist offline; the released code is an 8-GPU training harness whose only CompactionRL script targets Qwen3-4B math TIR rather than the paper's GLM SWE-bench setting; and **no checkpoint is published anywhere** (0 GitHub releases; the only in-repo "checkpoint" is 0-byte placeholder shards from a different, vanilla-PPO run).
It cannot be "the best proven open-source compaction algorithm" for this use case because nothing runnable is released and the paper reports no head-to-head against any competing compaction method.
What `sctxx` should take is the design lesson: keep the most recent interaction steps verbatim, and treat the summary prompt's coverage list as the thing worth optimizing — the paper's own Table 1 shows summarizer choice alone moves SWE-bench Verified by 6.5 points.

## Not verified / open

- **No official link between the paper and the repository.** The arXiv HTML contains no code URL and no "code available" statement, and never mentions "SAO". The repo's self-description and its internal `paper_each_segment` mode / "CompactionRL" docstrings are the only evidence of authorship — treat it as *a* reference implementation, not confirmed as the authors'.
- Whether `scripts/run-qwen3-4B-compactionrl.sh` was ever run to completion: the committed artifacts are vanilla-PPO math logs, not CompactionRL training curves.
- The GLM-4.5-Air-SFT construction ("SFT on trajectories generated by GLM-4.7") is not reproducible from anything released.
- Terminal-Bench 2.0 / Harbor / Terminus-KIRA details were read from the paper only; no third-party reproduction of any CompactionRL number was found.
