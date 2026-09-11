# Production OSS condensers — what is proven, and what is merely shipped

**Question.** Which context-compaction implementations in production open-source coding agents have evidence
behind them, and what should `sctxx` copy? Read from source.

**Sources.** `OpenHands/software-agent-sdk` @ `57f5cc9f` and `OpenHands/OpenHands` @ tag `0.62.0` (both MIT);
`openai/codex` @ `ce7fbb37` **and** at the sctxx pin `818f1cca` (Apache-2.0) — prompt and `compact.rs` are
identical at both; JetBrains Research, *The Complexity Trap* (arXiv:2508.21433v3), the only controlled ablation
of a compaction method on a coding benchmark I could find; OpenHands blog *Context Condensation* (Apr 2025) and
SDK tech report (arXiv:2511.03690v2 §4.6). Neither repo documents compaction for users. Everything below is
quoted code, a sourced number, or marked inference.

---

## 1. Landscape

| Project | File (upstream path) | Strategy | Model? | Licence | Evidence |
| --- | --- | --- | --- | --- | --- |
| OpenHands SDK (now) | `sdk/context/condenser/` (`llm_summarizing_condenser.py`, `pipeline_condenser.py`, `no_op_condenser.py`) | LLM summary replaces the first half; chain; identity control | yes | MIT | Blog 54% vs 53% on a *subset*; third-party 21/50 vs 20/50 |
| OpenHands 0.62 | `memory/condenser/impl/observation_masking_condenser.py` | Old observation bodies → `<MASKED>` | no | MIT | **best-evidenced here**, §5 |
| OpenHands 0.62 | `impl/recent_events_condenser.py`, `amortized_forgetting_condenser.py`, `browser_output_condenser.py`, `conversation_window_condenser.py` | Head + last N; head + tail, middle dropped; mask old browser outputs; keep system + first user + last half | no | MIT | none — the first *is* the recency baseline |
| OpenHands 0.62 | `impl/llm_summarizing_condenser.py` | LLM summary replaces forgotten span | yes | MIT | as above |
| OpenHands 0.62 | `impl/structured_summary_condenser.py`, `llm_attention_condenser.py` | Summary via JSON-schema tool call; LLM ranks event ids | yes | MIT | none |
| Codex CLI | `codex-rs/core/src/compact.rs` | Prompt-turn summary; verbatim user msgs + summary | yes | Apache-2.0 | none published |
| Codex CLI | `codex-rs/core/src/compact_token_budget.rs` | **No model**: drop transcript, re-inject initial context | no | Apache-2.0 | none published |
| SWE-agent | `sweagent/agent/history_processors.py` | `LastNObservations` elides old observations | no | MIT | the masking arm of *The Complexity Trap* |
| Aider | `aider/history.py` (+ `repomap.py`) | `ChatSummary` recursive head summary; deterministic repo map | split | Apache-2.0 | none published |
| Cline | `sdk/packages/core/src/extensions/context/basic-compaction.ts` | No-model fold: keep typed users, summarize tool activity | no | Apache-2.0 | none published |
| Roo Code | `src/core/condense/index.ts` | `summarizeConversation`, fresh start | yes | Apache-2.0 | none published |
| Goose | `crates/goose/src/context_mgmt/mod.rs` | `compact_messages` at 80% of window; middle-out dropping | yes | Apache-2.0 | none published |
| Continue | `extensions/cli/src/compaction.ts` (+ `core/llm/countTokens.ts`) | Compaction prompt + deterministic prune-to-fit | split | Apache-2.0 | none published |

## 2. OpenHands condensers, in detail

The agent was extracted into `OpenHands/software-agent-sdk`; the current tree keeps only
`LLMSummarizingCondenser`, `PipelineCondenser` and `NoOpCondenser`. Every other strategy the task named still
exists at tag `0.62.0`. All implement one interface (`memory/condenser/condenser.py`): *"Abstract condenser
interface. Condensers take a list of Event objects and reduce them into a potentially smaller list."* A
condenser returns a shorter `View` (masking) **or** a `Condensation` holding a `CondensationAction`
(forgetting) — a tombstone over an append-only log, where the summary is a new event and the forgotten span is
marked by `forgotten_events_start_id`/`_end_id`, so the log is never rewritten.

**No-model strategies.** `ObservationMaskingCondenser(attention_window=5)` replaces
`isinstance(event, Observation) and i < len(view) - attention_window` with
`AgentCondensationObservation('<MASKED>')`, keeping every reasoning step and action; its config makes the *live*
default `attention_window=100` while the class default is 5. `RecentEventsCondenser(keep_first=1,
max_events=10)` is exactly `head = view[:self.keep_first]`, `tail_length = max(0, self.max_events -
len(head))`, `return View(events=head + view[-tail_length:])` — **this is sctxx's recency window, verbatim.**
`AmortizedForgettingCondenser(max_size=100, keep_first=0)` keeps head + tail toward `max_size // 2` and emits
one range tombstone with **no summary**. `BrowserOutputCondenser` masks
`BrowserOutputObservation` only, and is the one place OpenHands states a rationale: those outputs are *"really
large and consume a lot of tokens without any benefits in performance."*

**Model-driven.** `LLMSummarizingCondenser(llm, max_size=100, keep_first=1, max_event_length=10_000)` keeps the
head and `max_size // 2 - len(head) - 1` tail events and summarizes the middle; each event is
`truncate_content(str(event), max_chars=max_event_length)` inside `<EVENT id=…>` and the prior summary is
injected as `<PREVIOUS SUMMARY>`. The prompt ends *"Now summarize the events using the rules above."* — today
`prompts/summarizing_prompt.j2`, with sections `USER_CONTEXT / TASK_TRACKING / COMPLETED / PENDING /
CURRENT_STATE` plus `CODE_STATE`, `TESTS`, `CHANGES`, `DEPS`, `VERSION_CONTROL_STATUS`.

The current SDK differs from 0.62: triggers are graded (`Reason.REQUEST`/`TOKENS` are `HARD`, `Reason.EVENTS`
is `SOFT` — token pressure is hard *"in benchmark runs that use a fixed local model context"*), an unsatisfiable
hard trigger retries with `max_event_str_length` scaled by `0.8`, and `minimum_progress=0.1` refuses a
condensation that would forget under 10% of the view.

## 3. Codex compaction, in detail

Two implementations share one prompt, byte-identical at the pin and at HEAD —
`codex-rs/prompts/templates/compact/prompt.md` in full:

```
You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for another LLM that will resume the task.

Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue

Be concise, structured, and focused on helping the next LLM seamlessly continue the work.
```

That is sctxx's thesis in four bullets — the artifact is a handoff for another model — with no "what failed"
or "do not retry this" bullet.

**Path A — `compact.rs`, model-driven.** The prompt is **appended as a user message to the existing history**
and the whole thing is sent. If the call hits `ContextWindowExceeded` it does not give up — `if turn_input_len >
1 { history.remove_first_item(); retries = 0; continue; }`, commented *"Trim from the beginning to preserve
cache … and keep recent messages intact."* The summary is the **last assistant message of that turn**, wrapped
as `format!("{SUMMARY_PREFIX}\n{summary_suffix}")`, and the replacement history comes from
`build_compacted_history(Vec::new(), &user_messages, &summary_text)`:

```rust
// COMPACT_USER_MESSAGE_MAX_TOKENS = 20_000
for message in user_messages.iter().rev() {          // newest first
    let tokens = approx_token_count(&message.message);
    if tokens <= remaining { selected_messages.push(message.clone()); remaining -= tokens; }
    else { /* middle-truncate to `remaining` tokens, then break */ }
}
selected_messages.reverse();                          // back to chronological
… history.push(CompactionSummary::new(summary_text))  // summary last
```

The new window is **the most recent ≤20k tokens of real user messages, verbatim (newest kept, the boundary
message middle-truncated), then one summary item last**. `is_summary_message()` stops a previous summary being
retained as a "user message", so summaries never accumulate. A warning then fires: *"Long threads and multiple
compactions can cause the model to be less accurate."*

**Path B — `compact_token_budget.rs`, no model.** Present at the pin, gated on `Feature::TokenBudget`, and
its body is pre-compact hooks, `sess.start_new_context_window(step_context, world_state)`, post-compact hooks —
which rebuilds history as *initial context only* and records `message: String::new()`, i.e. **no summary at
all**. Doc comment: *"Token-budget compaction skips model/server summarization and installs a fresh context
window instead."* A frontier lab ships "keep the world state, the transcript discarded" behind a flag. Triggers
are pre-turn, mid-turn and model-downshift; **evaluation: none** (§5).

## 4. Other agents

**None of these six attributes a benchmark number to its compaction method** — no ablation, no A/B, no score
delta; SWE-agent ties its config to the original paper but publishes no elide-vs-not figure.

- **SWE-agent** (`sweagent/agent/history_processors.py`, MIT) — `LastNObservations(n, polling=1)`: *"Elide all
  but the last n observations or remove tagged observations. This is our most classic history processor, used
  in the original paper to elide but the last 5 observations."* Elided text becomes *"Old environment output:
  (n lines omitted)"*; it never removes the first observation (the instance template) and honours
  `always_remove_output_for_tags = {"remove_output"}` / `{"keep_output"}` — **masking by semantic tag**.
- **Cline** (`sdk/packages/core/src/extensions/context/`, Apache-2.0) — `basic-compaction.ts` is
  **deterministic**: *"Fold the conversation history without a summarizer model. Typed user prompts always
  survive."* Constants `COMPACTION_TRIGGER_RATIO 0.9`, `DEFAULT_TARGET_RATIO 0.7`,
  `DEFAULT_PRESERVE_RECENT_TOKENS 20_000`; the verbatim suffix is snapped to an assistant message so tool pairs
  are not split, and prior output is frozen by metadata (`compaction: "preserved"`). `agentic-compaction.ts`
  adds a summarizer model, but overflow recovery is forced to the deterministic path because *"recovery must
  not depend on another successful LLM request"*.
- **Goose** (`crates/goose/src/context_mgmt/mod.rs`, Apache-2.0) — `compact_messages`,
  `DEFAULT_COMPACTION_THRESHOLD: f64 = 0.8`, plus a fallback that *"Drops tool responses from the middle
  outwards"* (`REMOVAL_PERCENTAGES [0,10,20,50,100]`).
- **Continue** (`core/util/conversationCompaction.ts`, `core/llm/countTokens.ts`, Apache-2.0) — split.
  `compactConversation` re-injects the prior summary as *"Previous conversation summary:"* and sends only
  history after it; `compileChatMessages` prunes deterministically and *"Always preserve[s] the last user/tool
  message sequence"*, removing older messages first.
- **Roo Code** (`src/core/condense/index.ts`, Apache-2.0) — `summarizeConversation` with *"CRITICAL: This is a
  summarization-only request. DO NOT call any tools or functions."* and a fresh-start model (*"return only
  messages from the summary onwards"*); thresholds `MIN_CONDENSE_THRESHOLD = 5`, `MAX_CONDENSE_THRESHOLD = 100`.
- **Aider** (`aider/history.py`, Apache-2.0) — `ChatSummary(max_tokens=1024)` keeps a tail verbatim and
  summarizes the head, recursing to depth 3; its repo map is `SELECT` at code-symbol granularity per *The
  Complexity Trap* Table 1 — code selection, not compaction.

## 5. Proven vs merely shipped

**Proven — one strategy only: observation masking.** *The Complexity Trap* is the only controlled ablation I
found. Abstract, verbatim: *"We find that a simple environment observation masking strategy halves cost
relative to the raw agent while matching, and sometimes slightly exceeding, the solve rate of LLM
summarization."* Table 3 (SWE-agent, SWE-bench Verified, 95% bootstrap CIs): Qwen3-Coder 480B Raw 53.4% /
$1.29, Observation Masking 54.8% / $0.61, LLM-Summary 53.8% / $0.64 — cheaper and higher, intervals
overlapping. §5.1 repeats the probe on **OpenHands v0.43.0** (Gemini 2.5 Flash, 50-instance slice, turn limit
250, masking M=10 and M=58) and reports it generalises *"after tuning"*, warning the window *"is an
agent-specific hyperparameter that requires tuning"* — OpenHands needed M=58 where SWE-agent needed M=10,
*"because OpenHands retains such retry turns."* §5.2 adds that summary generation is 2.86–7.2% of instance cost
and that subtracting it makes *"the efficiency difference … largely disappear."*

**Shipped but unproven: every other OpenHands condenser** — `AmortizedForgetting`, `RecentEvents`,
`BrowserOutput`, `ConversationWindow`, `StructuredSummary`, `LLMAttention`, `Pipeline` — no published number
exists for any of them. The v0 harness picks a condenser by name via `EVAL_CONDENSER` and documents the
noop default, so the A/B is *runnable* in-repo and was never published; the current `OpenHands/benchmarks`
harness is the opposite, hard-coding `enable_condenser: True, condenser_max_size: 240, condenser_keep_first: 2`
(`benchmarks/swebench/config.py:10–14`) while exposing `--disable-condenser` (`args_parser.py:113–138`) — the
benchmark everyone quotes runs **with** an LLM summarizing condenser and no published run reports the flag off.
The only OpenHands-authored number is the blog's: *"On the subset tested, the context condensation strategy
solves an average of **54%** of instances, while the baseline agent only solves an average of **53%**"* — no
instance count, model, config or raw data. The SDK tech report merely cites that blog. "OpenHands proved its
condenser helps" is **not supportable**; the only controlled public measurement is third-party, n=50.

**Codex: shipped, never evaluated.** No benchmark or ablation of Codex compaction exists in the repo or in
OpenAI's docs: `codex-rs/core/tests/suite/compact.rs` (5405 lines) asserts request shape, hooks and token
accounting against `wiremock`, with zero score/resolve-rate strings. The one third-party number mentioning it (a
GPT-5.1-Codex-Max table "with compaction") is a run *with* compaction enabled, not a delta attributed to it, and
its primary source returned HTTP 403.

**Better than a recency window: nothing proven.** No published number shows any strategy here beating a *tuned*
recency window; masking and a recency window both beat no management on cost, and masking matches LLM summarization at half the cost.

## 6. What sctxx should copy, ranked

1. **Observation masking as the shape of S2, not a fallback.** sctxx already substitutes placeholders for
   successful tool results (spec §7.3: `[read src/auth.ts: 340 lines]`) — that *is* masking, and it is the only
   strategy here with an ablation behind it. The refinement is a **graded, tag-aware** window: SWE-agent masks
   by observation tag as well as position, and OpenHands needed M=58 where SWE-agent needed M=10. A per-tier cut
   in `src/pipeline/segment.rs::plan` reuses `src/vendor/codex/tiered_input.rs`.
2. **Codex's user-message retention rule → S4 tail.** `build_compacted_history`'s budget is *N tokens of
   verbatim real user messages, newest kept, the oldest boundary message truncated, summary last*. sctxx's tail
   is a recency window over rows; the Codex rule says the tail should be **user messages first, budgeted
   backwards from the newest** — a change in `src/pipeline/segment.rs` plus one budget constant. Cline's
   `DEFAULT_PRESERVE_RECENT_TOKENS = 20_000` and Codex's 20k are independent arrivals at the same magnitude.
3. **Cline's deterministic-first fallback → S5 verify and ADR 0007.** Cline forces its no-model fold for
   overflow because *"recovery must not depend on another successful LLM request"* — sctxx's constitution stated
   by another project, and the strongest argument for keeping `--llm none` complete rather than degraded.
4. **`SUMMARY_PREFIX` / item-level provenance → S3 fold and render.** Codex prepends a fixed sentence telling
   the *reader* the text came from another model and that tool state is real, and marks the summary with
   `ContentItemKind("compaction.summary")`. sctxx has `prompts/handoff_preamble.md` and a low-trust
   `prior_summaries` slot, but its own fold output carries no self-description.
5. **`minimum_progress` → S5 verify.** Offline, the analogue of refusing a condensation that forgets under 10%
   of the view is a precondition the verifier states in the artifact: if the chosen budget folds less than X% of
   the transcript, say so rather than silently emitting a low-value fold.
6. **Not worth copying: `LLMAttentionCondenser`** (a full call to rank ids, discarded and back-filled from the
   newest events when too few come back) **or Goose's middle-out dropping** — the latter is the only drop policy
   here that is neither recency nor summarization, but no measurement supports it.

## 7. What does not apply to an offline one-shot compactor

- **Tombstones and `View` replay.** `Condensation` is a tombstone over a live append-only log, replayed by
  `View`. sctxx has a finished log and an active-branch index: filter, never replay a protocol. The
  transferable part is that the *decision* — which span was dropped — must be explicit, which sctxx does with
  gap markers carrying the omitted range; and since no live scaffold exists at handoff time, a hard-coded window
  is a guess, so keep the pointers that make a wrong guess recoverable.
- **Trigger policy.** `HARD`/`SOFT`, hard resets, `CompHashChanged`, `ModelDownshift`, Goose's 0.8 and Roo's
  percentage thresholds, Cline's `COMPACTION_TRIGGER_RATIO 0.9` and Continue's buffer answer "what does the live
  loop do when it is about to fail".
- **Prompt-cache economics and recursive summarization.** The OpenHands README's reason to condense *regularly*
  ("condensation destroys the prompt cache") and SWE-agent's `polling` are live-loop arguments with no offline
  analogue; and OpenHands, Codex, Aider, Roo and Continue all summarize summaries — Codex's own warning that
  *"multiple compactions can cause the model to be less accurate"* argues for deriving the artifact once.
