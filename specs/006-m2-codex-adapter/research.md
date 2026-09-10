# Research: Codex CLI compaction — algorithm, evidence, and what sctxx ports

**Block**: `specs/006-m2-codex-adapter/` (M2) · **Date**: 2026-09-11 · **Author**: agent (DSH session),
reviewed by the maintainer
**Unblocked**: [Determine whether Codex `compacted` lines carry readable summaries](../000-wayfinding/issues/05-codex-compacted-readability.md)
is answered below (Answers 1–3); see also [ADR 0002](../../docs/adr/0002-codex-compaction-algorithm-reuse.md).

Spec references: `docs/SCTXX-SPEC.md` §2.1, §6.2, §7.2, §7.3, §8, §11, §19 item 2.

## Decision

sctxx does **not** re-implement Codex's live compaction loop. Codex's compaction is an in-loop
mechanism that trades history for a summary under a context-window budget; sctxx is an offline
extractor over a *finished* transcript. What sctxx takes from it is four specific, portable things,
and one of them changes the Codex adapter:

1. **Two kinds of `compacted` line must be distinguished.** A `compacted` item with
   `window_number == null` is a *legacy history reset*; an item with `window_number != null` is a
   *windowed re-anchor* that leaves the full transcript intact. The adapter must record which one it
   saw instead of treating every `compacted` line as equivalent. (New IR field; see Consequences.)
2. **The post-compaction retention policy** — newest-first user-message fill under a token budget,
   with the summary kept **last** as the anchor — is adopted as the shape of the deterministic
   `--since-compact` seed (spec §3.4/§12.2), not as a replacement for the fold.
3. **Token-budget compaction is independent validation of the deterministic-first design.** Codex now
   ships a compaction path that performs *no summarization at all* and rebuilds the window from
   canonical context plus retained evidence. sctxx's `--llm none` artifact is the offline analogue and
   needs no new mechanism.
4. **Codex's Phase-1 tiered evidence budgeting stays vendored** as it is (`src/vendor/codex/tiered_input.rs`);
   the snapshot confirms the constants and the fill algorithm are unchanged in shape (Answer 7).

## Snapshot identity and a provenance problem

Everything below was read from the reference clone at `codex/` in the maintainer's checkout. That
clone **cannot be identified as the pinned commit**:

- `codex/codex-rs/Cargo.toml:153-154` → `[workspace.package] version = "0.0.0"` (every crate is
  `version.workspace = true`).
- `codex/codex-cli/package.json` → `"@openai/codex"`, `"version": "0.0.0-dev"`.
- `codex/` has **no `.git`** and is gitignored by the worktree (`.gitignore`: `/codex/`), so there is
  no commit hash to check against `818f1cca8ccf8899f0f4d59336baebaccf358eed`.
- The tree contains modules that post-date the pinned commit's documented surface
  (`core/src/compact_remote_v2.rs`, `core/src/compact_token_budget.rs`, `core/src/state/auto_compact_window.rs`,
  memory templates under `ext/memories/`, and a relocated `prompts/templates/compact/`).
  `codex-rs/rust-toolchain.toml` pins `1.95.0`.
- The snapshot can be *placed* even though it cannot be *pinned*: `codex-rs/build-info/src/lib.rs:18-27`
  stamps `STABLE_GIT_COMMIT` through `option_env!` and reports `is_source_build()` when the version is
  `0.0.0`, so a source checkout carries no commit id at runtime either; and
  `codex/announcement_tip.toml` marks version range `^0.(0..119).` as outdated, which puts this tree on
  the **≥0.120 development line** — well past the spec's pinned revision.

**Consequence for licensing**: `AGENTS.md`, `docs/SCTXX-SPEC.md` §2.3 and Appendix A,
`src/vendor/codex/README.md`, and the prompt headers all assert the pinned hash. That assertion is
currently unverifiable from this clone. Tracked as
[Pin the Codex vendoring source](../000-wayfinding/issues/11-codex-vendoring-pin.md); it is a
governance item, not a blocker for this block.

## Verified facts

All paths are relative to `codex/codex-rs/`. Line numbers are from the snapshot above.

### 1. Three compaction implementations, one dispatch table

`core/src/tasks/compact.rs:31-64` chooses:

| Condition | Implementation | Summarizes? |
|---|---|---|
| `config.features.enabled(Feature::TokenBudget)` | `compact_token_budget::run_manual_compact_task` | **No** |
| provider `RemoteCompactionSupport::V2` | `compact_remote_v2::run_remote_compact_task` | Server-side |
| `RemoteCompactionSupport::Unsupported` | `compact::run_compact_task` with `config.compact_prompt` or `compact::SUMMARIZATION_PROMPT` | Locally, in-loop |

`InitialContextInjection` (`core/src/compact.rs:72-78`) records the invariant that decides placement:
pre-turn/manual compaction uses `DoNotInject` (full initial context is re-injected at the next regular
turn); **mid-turn** compaction uses `BeforeLastUserMessage` because "the model is trained to see the
compaction summary as the last item in history".

### 2. The compaction prompt is nine lines

`prompts/templates/compact/prompt.md` (`SUMMARIZATION_PROMPT`) asks for: current progress and key
decisions; important context, constraints, or user preferences; what remains to be done; critical
data, examples, or references — "concise, structured, and focused on helping the next LLM seamlessly
continue the work".

`prompts/templates/compact/summary_prefix.md` (`SUMMARY_PREFIX`) is the handoff preamble:
"Another language model started to solve this problem and produced a summary of its thinking process.
You also have access to the state of the tools that were used by that language model. Use this to
build on the work that has already been done and avoid duplicating work. Here is the summary produced
by the other language model, use the information in this summary to assist with your own analysis:"

sctxx's `prompts/handoff_preamble.md` is already derived from the second (`derived_from` front-matter,
spec §8.6, Appendix A). The first is the required `--baseline codex-compact` text and is **not yet**
in `prompts/` — see Candidate tasks.

### 3. Local compaction: build the replacement history

`core/src/compact.rs`

- `COMPACT_USER_MESSAGE_MAX_TOKENS: usize = 20_000` (`:61`).
- Summary text = `format!("{SUMMARY_PREFIX}\n{summary_suffix}")` (`:354`), where `summary_suffix` is
  the **last assistant message of the compaction turn** (`:352-353`). An empty summary is replaced by
  `"(no summary available)"` (`:744-748`).
- `is_summary_message` (`:593-595`) tests `starts_with(SUMMARY_PREFIX + "\n")`: the summary itself is
  excluded from the retained user messages, so a re-compaction does not retain the previous summary as
  if it were a human turn.
- `build_compacted_history_with_limit` (`:678-755`) is the retention policy:
  1. walk user messages **newest-first** (`user_messages.iter().rev()`, `:687`);
  2. keep each while it fits the remaining budget (`approx_token_count`, `:691-694`);
  3. the message that crosses the boundary is **middle-truncated** to the remaining budget with
     `truncate_text(msg, TruncationPolicy::Tokens(remaining))` and then the loop stops (`:695-707`);
  4. reverse the selection back to chronological order (`:709`);
  5. append the summary **last** as a `user`-role fragment
     (`CompactionSummary`, `ContentItemKind("compaction.summary")`, `core/src/context/compaction_summary.rs`).
- **What does not survive locally**: images and audio are dropped from the retained history
  (`:511-528`, `:713-723`) — the replacement history is text-only.
- `insert_initial_context_before_last_real_user_or_summary` (`:607-663`) re-injects canonical context
  at a derived boundary: immediately **before the last real user message**; failing that, before the
  last user-message-like item (i.e. the summary); failing that, before the last compaction item;
  otherwise append. The compaction item therefore stays last.
- Overflow and failure handling (`:267-348`): on `ContextWindowExceeded` while compacting it calls
  `history.remove_first_item()` — drop the **oldest** item, to preserve the prefix cache — and retries;
  other errors retry with `backoff` up to `stream_max_retries`; `SessionBudgetExceeded` aborts.
- On success it emits a warning (`:399-402`): "Heads up: Long threads and multiple compactions can
  cause the model to be less accurate. Start a new thread when possible to keep threads small and
  targeted." — an upstream admission that repeated compaction degrades, which is the gap sctxx exists
  to fill.
- `core/src/compact_model_fallback.rs:10-20` retries a failed compaction with the *current* model for
  `InvalidRequest`, `UnexpectedStatus`, `ContextWindowExceeded`, `UsageLimitReached`, `ServerOverloaded`,
  `InternalServerError`, `RetryLimit`.

### 4. Token-budget compaction: no summary at all

`core/src/compact_token_budget.rs:20-29` and `core/src/session/mod.rs:4373-4417`
(`start_new_context_window`):

> "Token-budget compaction skips model/server summarization and installs a fresh context window
> instead. It is still modeled as compaction so compact hooks and `ContextCompaction` turn items
> observe the same lifecycle."

The new history is `build_initial_context_with_world_state(step_context, world_state)` chained with
retained client-authored developer messages (only when `Feature::RetainClientDeveloperMessages`),
budgeted by `compact_remote_v2::truncate_retained_messages_for_remote_compaction(...,
RETAINED_MESSAGE_TOKEN_BUDGET)`. It is written with `message: String::new()` — **there is deliberately
no summary**. The window number and window ids are advanced and persisted.

This is the direct upstream precedent for sctxx's `--llm none` artifact: a handoff can be built from
canonical context plus retained evidence, deterministically, with no model call.

### 5. Trigger and budget math

`core/src/session/context_window.rs:43-100`:

- `full_context_window_limit = context_window * effective_context_window_percent / 100` (`:74-76`),
  with `effective_context_window_percent` defaulting to **95** (`protocol/src/openai_models.rs:385-387`).
- The configured scope limit is
  `ModelInfo::auto_compact_token_limit() = min(config limit, context_window * 9 / 10)`
  (`protocol/src/openai_models.rs:515-524`).
- The auto-compaction scope is either `Total` (all active tokens) or `BodyAfterPrefix`
  (`active_context_tokens - prefill_baseline`, i.e. tokens added since the window's baseline, `:60-72`).
  The baseline prefers a server-observed value over an estimate
  (`core/src/state/auto_compact_window.rs:104-118`).
- `token_limit_reached = scope_tokens >= scope_limit + fallback_buffer || active >= full_context_window_limit`
  (`:88-95`, reached-test at `:107-109`) — the hard window cap triggers compaction independently of the
  configured scope limit. Trigger sites: pre-turn (`core/src/session/turn.rs:1233-1258`), mid-turn
  (`:601-627`), model switch as `CompHashChanged`/`ModelDownshift` (`:1335`, `:1383`), and manual
  `Op::Compact` (`core/src/session/handlers.rs:688`).
- Token counting is an **estimate**, not a tokenizer: `Session::get_total_token_usage`
  (`core/src/session/mod.rs:1354`) → `ContextManager::get_total_token_usage`
  (`core/src/context_manager/history.rs:678`) uses the last server-reported total plus a
  bytes/4 estimate for items added since (`utils/string/src/truncate.rs:4,71-74`). This is the same
  4-bytes-per-token convention sctxx vendors.

`core/src/state/auto_compact_window.rs` keeps `window_number`, `AutoCompactWindowIds { first_window_id,
previous_window_id, window_id }`, the prefill baseline, and one-shot delivery flags for the
token-budget reminder and the auto-compact fallback. All of this is *live-loop accounting* and has no
offline analogue: sctxx never needs to decide *when* to compact, only *what a past compaction meant*.

`sctxx`'s own budgets (spec §8.5: `chunk_tokens` 24,000, `state_tokens` 6,000, `--budget` 8,000,
`--tail` 12,000) are artifact budgets, not context-window budgets, and stay as they are.

### 6. Remote v2 retention and trimming

`core/src/compact_remote_v2.rs:73-77`:

- `RETAINED_MESSAGE_TOKEN_BUDGET: usize = 64_000`
- `MAX_RETAINED_AGENT_MESSAGE_TOKENS: i64 = 10_000`
- `MAX_REMOTE_COMPACTION_V2_STREAM_RETRIES: u64 = 2`
- `truncate_retained_messages_for_remote_compaction(items, budget)` (`:591`) bounds the retained
  developer messages; images get their own budget and a retained-image count is reported
  (`:503-507`).

Note the asymmetry: the **local** path retains human user messages under 20,000 tokens; the **remote
v2** path retains client-authored developer messages under 64,000 tokens, filtered by
`is_retained_for_remote_compaction_v2` (`:534-578`) and budget-walked by `truncate_retained_messages`
(`:598-692`), with a separate image budget and atomic image/label retention
(`core/src/compact_remote_v2_images.rs:31-99`). The remote path's *summary* is produced server-side and,
for OpenAI-hosted models, returned as a `ResponseItem::Compaction { encrypted_content }` — opaque
locally, so the replacement history is not readable. This is the fact spec §19 item 2 was waiting for.

**What does not survive remotely**: tool calls and their outputs, reasoning, system items, and
inter-agent progress/completion messages are all dropped from the replacement history
(`core/src/compact_remote_v2.rs:546-577`). The positive rule is: keep `user` messages that parse as a
user turn or hook prompt; keep `AgentMessage`s unless they are descendant progress
(`author` = `recipient + "/"` and text starts `Message Type: MESSAGE`) or a completion
(`Message Type: FINAL_ANSWER`), and only when ≤ `MAX_RETAINED_AGENT_MESSAGE_TOKENS`; keep `developer`
messages only when client-authored and the retain feature is on. The upstream test expectation is
`[dev, sys, user, hook, assistant, final, FunctionCall, Compaction(old)] → [user, hook, Compaction(new)]`
(`:828-866`). Before the request, a pre-flight forced trim rewrites oversized tool outputs to the
literal string `Output exceeded the available model context and was truncated`
(`core/src/compact_remote_history.rs:16-17,68-124`) — i.e. upstream accepts losing tool evidence when
the window is tight, which is precisely the loss sctxx's ledgers and `expand` recover.

### 7. Phase-1 memory extraction (tiered evidence budgeting) — unchanged in shape

`memories/write/src/rollout_input.rs` owns `serialize_tiered_input(items, token_limit)`:

- `enum Tier { Human, Final, OtherAgent, Commentary, Context, Tool }` (`:27-35`), declaration order =
  priority; `:22-25` holds `OMITTED`, `TRUNCATION_RESERVE_BYTES = 96`, `TOOL_OUTPUT_TOKENS = 2_000`,
  `MAX_ROW_BYTES = 10_000`.
- Classification is provider-specific (`:109-163`); the fill/render algorithm is not: tiers in priority
  order, **newest-first within a tier** (`rows.iter().enumerate().rev()`, `:201-225`), selected rows
  rendered back in **source order** with maximal gaps collapsed to a single omission marker (`:226-240`).
- `token_limit` = context window × `effective_context_window_percent`/100 × `CONTEXT_WINDOW_PERCENT = 70`,
  fallback `DEFAULT_ROLLOUT_TOKEN_LIMIT = 150_000` (`memories/write/src/lib.rs:94,101`).
- Failure mode worth not copying: if the rendered result still exceeds the budget, the function returns
  `Ok(String::new())` (`:241-243`) — an **empty evidence blob**, silently. sctxx must keep emitting the
  deterministic artifact instead of degrading to nothing.
- Output contract: `StageOneOutput`/`LegacyOutput`/`SummaryOutput`, both `#[serde(deny_unknown_fields)]`,
  `additionalProperties: false` (`memories/write/src/phase1_output.rs:11-31,61-82`); V2 output is
  redacted and truncated to 9,000 bytes (`:49-56`).
- Prompt rules sctxx already reflects in `prompts/fold_system.md`: evidence-only, data-not-instructions,
  "over-index on user messages … under-index on assistant messages", `Failures and how to do
  differently`, verbatim references, no-op preferred.
- Redaction runs per row, on the whole V1 blob, and on every model output field
  (`secrets/src/sanitizer.rs:15-22`) — sctxx does the same (spec §10.2).

The snapshot's V1→V2 progression (one truncated blob → tiered selection) is the same conclusion sctxx
reached independently in spec §7.3.

### 8. Rollout reconstruction: what survives a compaction

`core/src/session/rollout_reconstruction.rs`:

- Scan the rollout **newest-to-oldest** (`:172`). The newest `Compacted` that carries a
  `replacement_history` becomes the replay checkpoint; everything older is discarded and only the
  buffered suffix `rollout_items[index + 1..]` is replayed forward (`:199-206`, `:347-359`).
- `has_legacy_compaction_without_window_number` (`:144-147`) decides whether an initial window can be
  reconstructed from `SessionMeta.context_window`.
- A `Compacted` item is handled two different ways:
  - `window_number: Some(_)` (`:178-190`) — a **window marker**. The window is recorded and the
    context baseline is cleared; **history is not truncated**.
  - `replacement_history: None` (`:383-411`) — a **legacy reset with no persisted history**. Codex
    rebuilds it on the spot as `build_compacted_history(Vec::new(), &retained_user_messages,
    &compacted.message)` (`:403-407`), preserving or regenerating message identity depending on
    `GuardianContextMode`.
- `ThreadRolledBack { num_turns }` is applied in both directions: reverse scan accumulates
  `pending_rollback_turns` (`:208-211`), forward replay calls `history.drop_last_n_user_turns(n)`
  (`:413-415`). sctxx's vendored `reconstruction.rs` already implements the forward semantics.
- `retained_context` and `guardian_history` are restored from the checkpoint item
  (`:350-354`); `world_state` replay resets its baseline at every `Compacted` and merges patches
  (`:440-468`).

`CompactedItem` fields, `history/src/lib.rs:190-207`: `message`, `replacement_history`,
`guardian_history`, `retained_context`, `mcp_resource_origins`, `window_number`, `first_window_id`,
`previous_window_id`, `window_id`, `compaction_response_id`, `latest_token_usage_record`.

## The algorithm in pseudocode

```text
# Codex local compaction (core/src/compact.rs)
on trigger(reason ∈ {user_requested, context_limit, model_downshift, comp_hash_changed}):
    history += [SUMMARIZATION_PROMPT]                      # the 9-line prompt
    summary_suffix = last assistant message in the compact turn
    summary_text   = SUMMARY_PREFIX + "\n" + summary_suffix
    retained       = newest-first user messages, skipping any that start with SUMMARY_PREFIX,
                     filled to COMPACT_USER_MESSAGE_MAX_TOKENS (20_000),
                     boundary message middle-truncated to the remainder
    new_history    = retained (chronological) + [CompactionSummary(summary_text)]   # summary LAST
    if mid-turn: insert build_initial_context(world_state) before the last real user message
    replace_history(new_history); advance_window(); recompute_tokens()
    warn("long threads and multiple compactions reduce accuracy")

# Codex token-budget compaction (compact_token_budget.rs + start_new_context_window)
on trigger:
    new_history = build_initial_context(world_state)
                + retained developer messages (≤ RETAINED_MESSAGE_TOKEN_BUDGET = 64_000)
    replace_history(new_history, message = "")             # NO summary
    advance_window()

# Codex resume (rollout_reconstruction.rs)
scan newest → oldest until the newest Compacted with replacement_history
    → checkpoint; replay only items after it
ThreadRolledBack(n)  → drop the newest n user-turn segments
Compacted(window_number = Some) → window marker; history untouched
Compacted(replacement_history = None) → rebuild = retained user messages (≤20_000) + message
```

## How this maps onto sctxx

| Codex mechanism | sctxx today | Action |
|---|---|---|
| `prompt.md` (9-line summarization prompt) | not present | add `prompts/baseline_codex_compact.md` verbatim-with-attribution for `sctxx eval --baseline codex-compact` (spec §8.6, Appendix A) — M5 gated |
| `summary_prefix.md` | `prompts/handoff_preamble.md` (derived) | none |
| Retained newest user messages ≤ 20,000 tokens, summary last | spec §7.2 keeps *all* user messages, each ≤ 1,500 tokens | keep sctxx's policy; adopt the budget-based shape for the `--since-compact` seed |
| Initial context re-injected above the last real user message | n/a offline | informs seed ordering in the artifact |
| `window_number` distinguishes re-anchor from reset | `NativeCompaction { evt, summary }` — window fields ignored | **add `windowed: bool`** (and stop treating every `compacted` line as a reset) |
| `replacement_history` present ⇒ history reset | adapter reads it only for text | record the flag; it is the real "since-compact" boundary |
| Token-budget compaction (no model call) | `--llm none` deterministic artifact | none — cross-validation only |
| `ThreadRolledBack` replay | `src/vendor/codex/reconstruction.rs` | none |
| Tiered evidence budgeting | `src/vendor/codex/tiered_input.rs` | none; never copy the "return empty on overflow" branch |
| `retained_context` / `world_state` / `guardian_history` | repo reconciliation (S5) + ledgers | none — sctxx's equivalent is more portable |
| Compaction-model fallback chain | n/a (no in-loop compaction) | none |

Two gaps are already visible in the shipped code and are independent of this ticket:

- `Session.native_compactions` is **populated and never read** — no consumer in `src/pipeline/`.
  Spec §7.2 ("Native compactions … passed to the fold as low-trust seeds") and §7.3 (the
  `[prior-summary low-trust]` row) are therefore only half-wired: masking does render the summary row,
  but the seed list itself reaches nothing.
- There is no `--since-compact` flag in `src/cli/` although spec §3.4 defines it.

## Assumptions

- The snapshot is a recent Codex development build, so the newer `window_number`/token-budget
  behaviour is **upstream-current** and the legacy path is retained for compatibility. Verified by the
  presence of both paths and the `window_number.is_none()` compatibility branch; not verified against
  a tagged release.
- A `window_number`-bearing `compacted` item always leaves the transcript intact. Verified from the
  reverse/forward replay logic (`:178-190`, `:379-412`), not from a real rollout file.
- Real-rollout confirmation (one OpenAI-hosted model, one non-hosted) is still owed before the
  adapter change ships; the fixtures under `tests/fixtures/codex/` are synthetic.

## Alternatives considered

- **Port Codex's compaction loop into sctxx.** Rejected: it is in-loop and context-window-driven;
  sctxx's product boundary is post-hoc extraction from a finished file (roadmap "Explicitly deferred":
  live in-loop compaction before M6).
- **Model Codex's `ResponseItem` enum** to read `compacted` faithfully. Rejected as before (spec
  decision D-2): `serde_json::Value` projections keep the adapter tolerant and the crate publishable.
- **Replace sctxx's user-message policy with Codex's 20,000-token retention.** Rejected: sctxx's
  artifact is not a context window; it renders all user messages and lets `expand` recover detail. The
  budget-shaped selection is only right where a *bounded seed* is needed.
- **Mirror Codex's window/`world_state` accounting in the IR.** Rejected (Ponytail): offline, the only
  fact that matters is whether a boundary reset the history.

## Consequences

- **IR change (architectural, plan mode first).** `NativeCompaction` gains a flag distinguishing a
  windowed re-anchor from a legacy replacement-history reset; `ir.v1` has not been frozen (M7), so no
  version bump is needed yet, but the schema under `schemas/` and the adapter snapshots must be
  regenerated in the same change.
- The Codex adapter must parse `payload.window_number` / `payload.replacement_history.is_some()`.
  Its current mapping (`compacted` → `NativeCompactionSummary`, else `System{native_compaction}`) is
  already correct, including the empty-`message` case, because `str_field` filters empty strings
  (`src/adapters/mod.rs:272-277`) — no change needed there.
- **Licensing:** the pin assertion in `AGENTS.md` §"Hard rules" 1, spec §2.3/Appendix A, the vendor
  README, and every vendored file header must be reconciled with ticket 11 before M4 publishing.
- The "return empty on overflow" behaviour in Codex's tiered budgeting is an anti-pattern for sctxx:
  budgets must degrade to the deterministic artifact, never to nothing (spec §16, §12.2).
- `--since-compact` should target the newest legacy *reset* boundary when one exists, and otherwise
  the oldest window boundary, seeding with that item's `message` as a low-trust summary.

## Candidate tasks for this block

IDs are provisional; `/speckit-tasks` owns the final list and order.

- [ ] **T0601** — fixture: add a redacted Codex rollout containing a `compacted` item with
  `window_number` set and one with `replacement_history` only.
  *Proof*: `cargo test --all-features adapters::codex::compaction_kinds` + an insta snapshot of the IR.
- [ ] **T0602** — IR: add the windowed-vs-legacy flag to `NativeCompaction`; regenerate `schemas/ir.v1.json`.
  *Proof*: schema diff committed, IR snapshot updated, `cargo xtask gen-schemas` idempotent.
- [ ] **T0603** — adapter: populate the flag from `payload.window_number` /
  `payload.replacement_history`; keep the existing text fallback.
  *Proof*: T0601 snapshot green; a unit test asserts an empty `message` yields `System{native_compaction}`.
- [ ] **T0604** — pipeline: consume `Session.native_compactions` as low-trust seeds in the fold
  (spec §7.2/§7.3) instead of only rendering the masked row.
  *Proof*: mock-backend pipeline snapshot shows the seed line in the fold prompt.
- [ ] **T0605** — CLI: `extract --since-compact`, resolving to the newest legacy reset (else the oldest
  window), with a one-line stderr note naming the boundary event.
  *Proof*: `assert_cmd` test for the flag plus an artifact excerpt.
- [ ] **T0606** — prompts: add `prompts/baseline_codex_compact.md` (verbatim Codex `prompt.md` with the
  Apache-2.0 header) and its row in `prompts/README.md`. **M5-gated** (`specs/016-*`); do not land
  before the eval harness exists.
- [ ] **T0607** — resolver for ticket 11: record the snapshot's real provenance or re-pin the clone,
  then update the six places that assert `818f1cc` (documentation-only, no code).
