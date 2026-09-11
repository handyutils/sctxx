# Production OSS condensers — what is proven, and what is merely shipped

**Question.** Which context-compaction implementations in production open-source coding agents have
evidence behind them, and what should `sctxx` copy? Read from source, not from blog summaries.

**Sources read.** `OpenHands/software-agent-sdk` @ `57f5cc9f` (MIT) and `OpenHands/OpenHands` @ tag
`0.62.0` (pre-split tree, MIT, `git fetch --depth 1 --filter=blob:none`). `openai/codex` @ `ce7fbb37`
**and** at the sctxx pin `818f1cca` (committer date `2026-09-10T16:55:42Z`, Apache-2.0); the compaction
prompt and `compact.rs` are identical at both. JetBrains Research, *The Complexity Trap*
(arXiv:2508.21433v3, HTML) — the only controlled ablation of a compaction method on a coding benchmark I
could find. OpenHands blog *Context Condensation* (Apr 2025); SDK tech report (arXiv:2511.03690v2 §4.6).
Neither repo documents compaction for users: `docs/` in Codex has zero matches for `compact`. Everything
below is quoted code, a quoted number with its source, or marked as inference — no benchmark number is
invented or extrapolated.

---

## 1. Landscape

| Project | File (upstream path) | Strategy | Model? | Licence | Evidence |
| --- | --- | --- | --- | --- | --- |
| OpenHands SDK (now) | `sdk/context/condenser/llm_summarizing_condenser.py` | LLM summary replaces the first half | yes | MIT | Blog 54% vs 53% on a *subset*; 3rd-party 21/50 vs 20/50 |
| OpenHands SDK (now) | `sdk/context/condenser/pipeline_condenser.py` | Chain; first `Condensation` wins | no | MIT | none |
| OpenHands SDK (now) | `sdk/context/condenser/no_op_condenser.py` | Identity (ablation control) | no | MIT | none |
| OpenHands 0.62 | `memory/condenser/impl/observation_masking_condenser.py` | Old observation bodies → `<MASKED>` | no | MIT | **best-evidenced here**, §5 |
| OpenHands 0.62 | `impl/recent_events_condenser.py` | Head + last N events | no | MIT | none — this *is* the recency baseline |
| OpenHands 0.62 | `impl/amortized_forgetting_condenser.py` | Head + tail, middle dropped, no summary | no | MIT | none |
| OpenHands 0.62 | `impl/browser_output_condenser.py` | Mask all but newest N browser observations | no | MIT | none |
| OpenHands 0.62 | `impl/conversation_window_condenser.py` | System + first user msg + recall + last half | no | MIT | none |
| OpenHands 0.62 | `impl/llm_summarizing_condenser.py` | LLM summary replaces forgotten span | yes | MIT | as above |
| OpenHands 0.62 | `impl/structured_summary_condenser.py` | Summary forced through a JSON-schema tool call | yes | MIT | none |
| OpenHands 0.62 | `impl/llm_attention_condenser.py` | LLM ranks event ids; keep top-k | yes | MIT | none |
| OpenHands 0.62 | `impl/pipeline.py` | Chain of condensers | no | MIT | none |
| Codex CLI | `codex-rs/core/src/compact.rs` | Prompt-turn summary; verbatim user msgs + summary | yes | Apache-2.0 | none published |
| Codex CLI | `codex-rs/core/src/compact_token_budget.rs` | **No model**: drop transcript, re-inject initial context | no | Apache-2.0 | none published |
| Codex CLI | `codex-rs/core/src/compact_remote_v2.rs` | Server-side; retains developer messages | server | Apache-2.0 | none published |
| SWE-agent | `sweagent/agent/history_processors.py` | `LastNObservations` elides old observations | no | MIT | the masking arm of *The Complexity Trap* |
| Aider | `aider/history.py` | `ChatSummary` recursive head summary | yes | Apache-2.0 | none published |
| Cline | `sdk/packages/core/src/extensions/context/basic-compaction.ts` | No-model keep-users + tool-activity summary | no | Apache-2.0 | none published |
| Roo Code | `src/core/condense/index.ts` | `summarizeConversation` + summary prompt | yes | Apache-2.0 | none published |
| Goose | `crates/goose/src/context_mgmt/mod.rs` | `compact_messages` at 80% of window | yes | Apache-2.0 | none published |
| Continue | `extensions/cli/src/compaction.ts` | Compaction prompt + prune-to-fit | yes | Apache-2.0 | none published |

## 2. OpenHands condensers, in detail

The agent was extracted into `OpenHands/software-agent-sdk`; the current tree keeps only
`LLMSummarizingCondenser`, `PipelineCondenser`, `NoOpCondenser` (`condenser/__init__.py`). Every other
strategy the task named still exists at tag `0.62.0`, so that is the set worth reading. All implement one
interface (`memory/condenser/condenser.py`): *"Abstract condenser interface. Condensers take a list of Event
objects and reduce them into a potentially smaller list."* A condenser returns a shorter `View` (masking
style) **or** a `Condensation` holding a `CondensationAction` (forgetting style). The latter is a tombstone
over an append-only log: the summary is a new event and the forgotten span is marked by
`forgotten_events_start_id`/`_end_id`, so the log is never rewritten. `RollingCondenser` adds the
`should_condense()` / `get_condensation()` split every strategy uses.

**No-model strategies.** `ObservationMaskingCondenser(attention_window=5)` replaces
`isinstance(event, Observation) and i < len(view) - attention_window` with
`AgentCondensationObservation('<MASKED>')`, keeping every reasoning step and action; through
`ObservationMaskingCondenserConfig` the *live* default is `attention_window=100`, while the class default is
5. `BrowserOutputCondenser(attention_window=1)` is the same trick restricted to `BrowserOutputObservation`,
walking in reverse and counting, replacing with `f'Visited URL {event.url}\nContent omitted'`; its docstring
is the only rationale stated anywhere in OpenHands — *"These are really large and consume a lot of tokens
without any benefits in performance."* `RecentEventsCondenser(keep_first=1, max_events=10)` is
`head = view[:self.keep_first]`, `tail_length = max(0, self.max_events - len(head))`,
`return View(events=head + view[-tail_length:])` (config default `max_events=100`) — **this is sctxx's
recency window, verbatim.** `AmortizedForgettingCondenser(max_size=100, keep_first=0)` keeps head + tail
toward `max_size // 2` and emits one range tombstone with **no summary**, triggered by
`len(view) > max_size`. `ConversationWindowCondenser()` finds the system message, the first user message,
the `RecallAction` whose `query == first_user_msg.content` and its observation, keeps ~half the
non-essential events, and drops leading dangling observations from the kept slice; it fires only on an
explicit request. `CondenserPipeline(*condensers)` / `PipelineCondenser(condensers=[...])` applies each in
order and stops at the first that returns a `Condensation` instead of a `View`.

**Model-driven strategies.** `LLMSummarizingCondenser(llm, max_size=100, keep_first=1,
max_event_length=10_000)` keeps the head, keeps `max_size // 2 - len(head) - 1` tail events, and summarizes
the middle. Each event is `truncate_content(str(event), max_chars=max_event_length)` inside
`<EVENT id=…>`; the prior summary is injected as `<PREVIOUS SUMMARY>`. The fixed prompt ends *"Now summarize
the events using the rules above."*; today it is `prompts/summarizing_prompt.j2` with `USER_CONTEXT /
TASK_TRACKING / COMPLETED / PENDING / CURRENT_STATE` plus `CODE_STATE`, `TESTS`, `CHANGES`, `DEPS`,
`VERSION_CONTROL_STATUS` for code tasks. Validator: `keep_first < max_size // 2`.

**The current SDK differs materially** from 0.62. (i) Triggers are graded: `Reason.REQUEST` and
`Reason.TOKENS` are `HARD`, `Reason.EVENTS` is `SOFT` — a comment explains that token pressure is hard *"in
benchmark runs that use a fixed local model context"*. (ii) A hard trigger that cannot be satisfied retries:
`hard_context_reset()` re-summarizes the whole view with `max_event_str_length` scaled by `0.8`, up to
`hard_context_reset_max_retries=5`. (iii) `minimum_progress=0.1` — forgetting under 10% of the view raises
`NoCondensationAvailableException` instead of applying. Class default `max_size=240`, but the
`default_condenser()` helper uses `max_size=80, keep_first=4`. `StructuredSummaryCondenser` forces the
summary through function calling (`tool_choice` pinned to `create_state_summary`) into a 19-field
`StateSummary` model; on a parse failure it logs a warning and substitutes an **empty** `StateSummary()`,
and the call is still made and paid for. It disables prompt caching: *"This condenser cannot take advantage
of prompt caching."* `LLMAttentionCondenser` spends one call asking the model to sort event ids by
importance for the next step (`response_format={'type': 'json_schema', …}`), keeps the top `target_size -
keep_first`, filters head ids, and back-fills from the newest events if too few ids came back; it refuses to
construct unless `litellm.supports_response_schema(...)`. `NoOpCondenser` is identity, documented in the
0.62 evaluation README as the default for evaluation.

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

That is sctxx's thesis in four bullets: the artifact is a handoff and the summary is *for another model*.
It has no "what failed" or "do not retry this" bullet.

**Path A — `compact.rs`, model-driven.** From `run_compact_task_inner_impl`: (1)
`run_inline_auto_compact_task` builds the input from `config.compact_prompt.unwrap_or(SUMMARIZATION_PROMPT)`;
manual `/compact` reaches the same inner function. (2) The prompt is **appended as a user message to the
existing history** and the whole thing is sent. (3) If the call hits `ContextWindowExceeded` it does not give
up — `if turn_input_len > 1 { history.remove_first_item(); retries = 0; continue; }`, commented *"Trim from
the beginning to preserve cache (prefix-based) and keep recent messages intact."* (4) The summary is the
**last assistant message of that turn** (`get_last_assistant_message_from_turn(...).unwrap_or_default()`),
wrapped as `format!("{SUMMARY_PREFIX}\n{summary_suffix}")`. (5) The replacement history is
`build_compacted_history(Vec::new(), &user_messages, &summary_text)` — the portable part:

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

So the new window is **the most recent ≤20k tokens of real user messages, verbatim (newest kept, the
boundary message middle-truncated), then one summary item last**. `is_summary_message()` stops a previous
summary being retained as a "user message", so summaries do not accumulate; the remote path bounds
scaffolding separately with `RETAINED_MESSAGE_TOKEN_BUDGET = 64_000` for client-authored developer messages
and `MAX_RETAINED_AGENT_MESSAGE_TOKENS = 10_000`. (6) `replace_compacted_history(...)` persists
`CompactedHistoryMetadata { message, window_number, window_ids, compaction_response_id,
compaction_model_hash }` — `window_ids` so a later resume can tell which compaction produced the current
window. (7) Tokens are recomputed and a warning is emitted: *"Long threads and multiple compactions can
cause the model to be less accurate. Start a new thread when possible…"* — the honest admission that
repeated folding degrades.

**Path B — `compact_token_budget.rs`, no model.** Present at the pin, gated on `Feature::TokenBudget`:
`if turn_context.config.features.enabled(Feature::TokenBudget) { /* Compaction is the reset request, so
force a new context window */ crate::compact_token_budget::run_inline_auto_compact_task(...).await?; return
Ok(()); }`. The body is three steps: pre-compact hooks, `sess.start_new_context_window(step_context,
world_state)`, post-compact hooks. That rebuilds history as *initial context only* and records `message:
String::new()` — **no summary at all**. Its doc comment: *"Token-budget compaction skips model/server
summarization and installs a fresh context window instead."* A frontier lab ships "keep the world state,
drop the transcript" behind a flag.

**Triggers.** `token_status.token_limit_reached` fires pre-turn, mid-turn, and on model downshift; reasons
are enumerated in `compact_model_fallback.rs` (`user_requested`, `context_limit`, `model_downshift`,
`comp_hash_changed`) — the last meaning a change in the compaction model's compatibility hash re-compacts
history. **Evaluation: none** (§5).

## 4. Other agents

Read from each upstream file. **None of these projects publishes a benchmark number attributed to its
compaction method** — no ablation, no A/B, no score delta.

- **SWE-agent** (`sweagent/agent/history_processors.py`, MIT) — `LastNObservations(n, polling=1)`. Its
  docstring: *"Elide all but the last n observations or remove tagged observations. This is our most classic
  history processor, used in the original paper to elide but the last 5 observations. Elided observations are
  replaced by 'Old environment output: (n lines omitted)'."* It also carries
  `always_remove_output_for_tags = {"remove_output"}` and `always_keep_output_for_tags = {"keep_output"}` —
  **masking driven by a semantic tag, not only by position** — plus `ClosedWindowHistoryProcessor`,
  `CacheControlHistoryProcessor` and `RemoveRegex`. It is the masking arm of *The Complexity Trap* (a cost
  win, not a solve-rate claim).
- **Aider** (`aider/history.py`, Apache-2.0) — `ChatSummary(models, max_tokens=1024)` with `too_big()`,
  `tokenize()` and a recursive `summarize_real(messages, depth)`: a model-driven head-summarizer triggered by
  a token budget, not a window. Its repo map (`aider/repomap.py`, PageRank over code symbols, `--map-tokens`)
  is classified by *The Complexity Trap* Table 1 as `SELECT` at code-symbol granularity — code selection, not
  conversation compaction.
- **Cline** (`sdk/packages/core/src/extensions/context/`, Apache-2.0) — ships **two** paths:
  `basic-compaction.ts` (no model: keep typed user messages, summarize tool activity to text, budget
  projection, `ensureFilesSection`, `extractFileOps`) and `agentic-compaction.ts` (summarizer model with a
  `MIN_AGENTIC_SUMMARY_INPUT_TOKENS = 1_024` floor). Shared helpers include `findLatestSummaryIndex`, i.e.
  summaries do not accumulate.
- **Roo Code** (`src/core/condense/index.ts`, Apache-2.0) — `summarizeConversation`, a `SUMMARY_PROMPT`,
  `getMessagesSinceLastSummary`, `getEffectiveApiHistory`; the trigger is a percentage of the context window
  (`MIN_CONDENSE_THRESHOLD = 5`, `MAX_CONDENSE_THRESHOLD = 100`). No file in its eval tree references
  condensing.
- **Goose** (`crates/goose/src/context_mgmt/mod.rs`, Apache-2.0) — `compact_messages`,
  `DEFAULT_COMPACTION_THRESHOLD: f64 = 0.8` (compact at 80% of the window),
  `TOOLCALL_SUMMARIZATION_BATCH_SIZE = 10`, and fixed continuation text appended after the summary (*"Your
  context was compacted… Do not mention that you read a summary…"*).
- **Continue** (`core/util/conversationCompaction.ts`, `extensions/cli/src/compaction.ts`, Apache-2.0) —
  `compactConversation` chains summaries through per-message `conversationSummary` fields and skips history
  after the latest summary; auto-compaction uses `AUTO_COMPACT_BUFFER_CAP = 15_000` and
  `AUTO_COMPACT_BUFFER_RATIO = 0.8` and appends a `COMPACTION_PROMPT` (~150 tokens). Its tests cover
  infinite-loop and prune-to-fit behaviour, not scores.

## 5. Proven vs merely shipped

**Proven — one strategy only: observation masking.** *The Complexity Trap* (JetBrains Research,
arXiv:2508.21433v3, DL4C @ NeurIPS '25) is the only controlled ablation I found that isolates a compaction
method on a coding benchmark. Abstract, verbatim: *"We find that a simple environment observation masking
strategy halves cost relative to the raw agent while matching, and sometimes slightly exceeding, the solve
rate of LLM summarization."* Its Table 3 (SWE-agent, SWE-bench Verified, 95% bootstrap CIs) reports
Qwen3-Coder 480B Raw 53.4% / $1.29, Observation Masking 54.8% / $0.61, LLM-Summary 53.8% / $0.64 — masking is
cheaper and numerically higher, though the intervals overlap. §5.1 repeats the probe on **OpenHands v0.43.0**
(Gemini 2.5 Flash, no thinking, 50-instance slice, turn limit 250, `llm`-Summary N=21/M=10, masking M=10 and
M=58) and reports the result generalises *"after tuning"*, warning that the masking window *"is an
agent-specific hyperparameter that requires tuning"* — OpenHands needed M=58 where SWE-agent needed M=10,
*"because OpenHands retains such retry turns."* A delegated recomputation from the released raw runs
(`huggingface.co/datasets/JetBrains-Research/the-complexity-trap`) gives 20/50 raw-noop, 21/50
LLMSummarizing, 22/50 masking; I did not re-run it here, so treat the counts as indicative and the paper's
qualitative claim as the citable part. §5.2 adds the mechanism: summary generation is 2.86–7.2% of instance
cost and *"Once we subtract these summarization costs from the total, the efficiency difference … largely
disappears."*

**Shipped but unproven: every other OpenHands condenser.** `AmortizedForgetting`, `RecentEvents`,
`BrowserOutput`, `ConversationWindow`, `StructuredSummary`, `LLMAttention`, `CondenserPipeline` — no
published OpenHands number exists for any of them, on any benchmark. The v0 harness selects a condenser by
name via `EVAL_CONDENSER` and documents the noop default, so the A/B is *runnable* in-repo and was never
published. The current `OpenHands/benchmarks` harness is the opposite: it hard-codes `CONDENSER_DEFAULTS =
{"enable_condenser": True, "condenser_max_size": 240, "condenser_keep_first": 2}`
(`benchmarks/swebench/config.py:10–14` at `main`) and exposes `--enable-condenser`/`--disable-condenser`
(`benchmarks/utils/args_parser.py:113–138`) — the benchmark everyone quotes is run **with** an LLM
summarizing condenser, and no published run reports the flag off. The only OpenHands-authored number is the
blog's, verbatim: *"On the subset tested, the context condensation strategy solves an average of **54%** of
instances, while the baseline agent only solves an average of **53%**"* — a subset of SWE-bench Verified
with no instance count, model, config, or raw data (its other claim, *"Up to 2x per-turn API cost
reduction"*, is a chart). The SDK tech report merely cites that blog and contains "ablation" zero times.
"OpenHands proved its condenser helps" is **not supportable**; the honest statement is that the condenser is
in the code and on by default in the benchmark harness, and the only controlled public measurement is
third-party, n=50, showing no significant solve-rate difference.

**Codex: shipped, never evaluated.** No benchmark or ablation of Codex compaction exists in the repo or in
OpenAI's docs. `codex-rs/core/tests/suite/compact.rs` (5405 lines) asserts request shape, hooks, token
accounting and event order against `wiremock` — zero score/resolve-rate strings. Public docs describe
`/compact` mechanically ("Summarize the visible chat to free tokens"). The one third-party number mentioning
Codex compaction (a GPT-5.1-Codex-Max table "with compaction") is a model evaluation run *with* compaction
enabled, not a delta attributed to it, and its primary source returned HTTP 403.

**Better than a recency window: nothing proven.** No published number shows any strategy here beating a
*tuned* recency window on solve rate. The nearest defensible statement is the reverse: masking and a recency
window both beat no management at all on cost, and masking matches LLM summarization on solve rate at
roughly half the cost.

## 6. What sctxx should copy, ranked

1. **Observation masking as the shape of S2, not a fallback.** sctxx already substitutes placeholders for
   successful tool results (spec §7.3: `[read src/auth.ts: 340 lines]`). That *is* masking, and it is the only
   thing here with an ablation behind it. The transferable refinement is a **graded, tag-aware** window:
   SWE-agent masks by observation tag as well as position, and the OpenHands probe needed M=58 where SWE-agent
   needed M=10 because scaffolds retain different turn types. sctxx's window is the episode-boundary cut in
   `src/pipeline/segment.rs::plan`; making the cut per-tier (`Tier::ToolResultOk` masked earliest,
   `Tier::User` never) reuses the existing `Tier` order in `src/vendor/codex/tiered_input.rs` instead of
   adding a concept.
2. **Codex's user-message retention rule → S4 tail.** The most portable idea here is
   `build_compacted_history`'s budget: *N tokens of verbatim real user messages, newest kept, the oldest
   boundary message truncated, summary last*. sctxx's tail is a recency window over rows; the Codex rule says
   the tail should be **user messages first, budgeted backwards from the newest**, not "the last X tokens of
   everything". A change inside `src/pipeline/segment.rs` plus one budget constant, composing with the
   existing `Tier::User > …` order rather than competing with it.
3. **`SUMMARY_PREFIX` / item-level provenance → S3 fold and render.** Codex prepends a fixed sentence telling
   the *reader* that the text came from another model and that tool state is real, and marks the generated
   summary with its own `ContentItemKind("compaction.summary")`. sctxx has `prompts/handoff_preamble.md` and a
   low-trust `prior_summaries` slot in `prompts/fold_user.md` for *provider* summaries, but its own fold
   output carries no equivalent self-description.
4. **Cline's `findLatestSummaryIndex` → S3 chunking.** Cline and Codex both refuse to feed a previous summary
   back in as ordinary content; sctxx's `later_index` and state handle this for the fold, and the invariant is
   worth asserting in S5: no chunk's input may contain a summary it did not itself produce.
5. **`minimum_progress` → S5 verify.** The current SDK refuses a condensation that would forget under 10% of
   the view. Offline, the analogue is a precondition the verifier states in the artifact: if the chosen budget
   folds less than X% of the transcript, say so rather than silently emitting a low-value fold.
6. **Codex's token-budget path as the `--llm none` story.** A frontier lab ships a compaction mode whose
   entire output is "the world state, transcript discarded" with an empty message. sctxx's deterministic
   artifact already does more, so this is validation, not a feature — worth one sentence in the docs.
7. **Not worth copying: `LLMAttentionCondenser`.** It spends a full call over the entire view to rank ids,
   then discards the ranking when the model returns too few and back-fills from the newest events anyway — it
   degrades to a recency window.

## 7. What does not apply to an offline one-shot compactor

- **Tombstones and `View` replay.** `Condensation` is a tombstone over a live append-only log, replayed by
  `View`. sctxx has a finished log and an active-branch index: filter, never replay a protocol. The
  transferable part is only that the *decision* (which span was dropped) must be explicit — sctxx does this
  with gap markers carrying the omitted event range.
- **Tuning a window to a scaffold.** There is no live scaffold at handoff time and the consumer agent is
  unknown, so any hard-coded window is a guess. Keep the pointers that make a wrong guess recoverable rather
  than trying to guess right.
- **Trigger policy.** `HARD`/`SOFT`, `minimum_progress` retries, hard resets, `CompHashChanged`,
  `ModelDownshift`, `AutoCompactWindowIds`, Goose's 0.8 threshold, Roo's percentage thresholds and Continue's
  auto-compaction buffer all answer "what does the live loop do when it is about to fail". A one-shot
  compactor never fails mid-loop and cannot ask again.
- **Prompt-cache economics.** The OpenHands README's reason to condense *regularly* ("condensation destroys
  the prompt cache"), SWE-agent's `polling` parameter and `StructuredSummaryCondenser` setting
  `caching_prompt = False` are live-loop cost arguments with no offline analogue.
- **Recursive summarization.** Both OpenHands, Codex, Aider, Roo and Continue summarize summaries. Codex's own
  warning — *"multiple compactions can cause the model to be less accurate"* — supports sctxx's current
  design: derive the artifact from the transcript once and keep prior provider summaries in a low-trust slot
  rather than folding them forward.
