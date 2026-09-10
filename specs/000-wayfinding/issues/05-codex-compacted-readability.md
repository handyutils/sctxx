# Determine whether Codex `compacted` lines carry readable summaries

Type: research
Status: resolved (2026-09-11)

## Question

At the pinned Codex commit and in real rollouts from current Codex CLI versions, when does a `compacted`
rollout line contain readable summary text, and when does it carry only replacement history or encrypted
remote-compaction items? What should the adapter emit in each case?

Verify against the pinned source (`codex-rs/history`, `codex-rs/core/src/compact*.rs`) and at least two real
rollouts (one using an OpenAI-hosted model, one using another provider).

Unblocks: `specs/006-m2-codex-adapter/`. Spec reference: `docs/SCTXX-SPEC.md` §6.2, §19 item 2.

## Answer

Answered from the reference clone's source (a `0.0.0-dev`, ≥0.120-line build — see
[Pin the Codex vendoring source](11-codex-vendoring-pin.md); the clone cannot be tied to the pinned
commit, so the *shape* below is verified but the revision is not). Full evidence:
[`specs/006-m2-codex-adapter/research.md`](../../006-m2-codex-adapter/research.md). Decision:
[ADR 0002](../../../docs/adr/0002-codex-compaction-algorithm-reuse.md).

**Readable text is not guaranteed. There are three cases, and the deciding fields are
`message`, `replacement_history`, and `window_number` on the `compacted` item
(`history/src/lib.rs:190-207`).**

1. **Local summarization ran → readable.** `core/src/compact.rs:352-354` writes
   `message = SUMMARY_PREFIX + "\n" + <last assistant message of the compaction turn>`, and
   `replacement_history` holds the retained user messages plus that summary as a
   `compaction.summary` user fragment (`:665-755`). Both are readable.
2. **Remote compaction → not readable.** The server returns a
   `ResponseItem::Compaction { encrypted_content }`; the replacement history contains an opaque item
   (`core/src/compact_remote_v2.rs:433-510`). Only the boundary is recoverable locally.
3. **Token-budget compaction → deliberately empty.** `core/src/session/mod.rs:4373-4417`
   (`start_new_context_window`) writes `message: String::new()` and performs **no summarization at
   all**; the new window is rebuilt from canonical context plus retained developer messages. There is
   no summary to read.

**The semantic answer matters more than the text.** `window_number == null` with a
`replacement_history` is a legacy **history reset** — everything before it is gone.
`window_number != null` is a **window marker**: the full transcript is still intact and the item only
resets the token baseline (`core/src/session/rollout_reconstruction.rs:144-147,178-190,379-412`).
Treating the second as a reset would discard the entire pre-window history of a session that was never
truncated.

**What the adapter should emit — already correct as shipped.**

- text present → `EventKind::NativeCompactionSummary { text }`
- no text (encrypted, or empty `message`) → `EventKind::System { subtype: "native_compaction" }`

`src/adapters/codex.rs:90-103` does exactly this, and the empty-`message` case is handled because
`str_field` filters empty strings (`src/adapters/mod.rs:272-277`). No change is required here.

**One gap remains, and it is not about readability:** the IR loses the reset-vs-re-anchor distinction.
`NativeCompaction { evt, summary }` (`src/ir.rs:254-259`) records neither `window_number` nor whether
a `replacement_history` existed, so `--since-compact` (spec §3.4) cannot be defined correctly and the
low-trust seeds cannot be ordered. The fix is scoped in the research doc's candidate tasks (T0601–T0605).

**Still owed before the adapter change ships:** confirmation against two *real* rollouts — one
OpenAI-hosted (encrypted path) and one non-hosted (local path). The fixtures currently under
`tests/fixtures/codex/` are synthetic. Until then, case 2 is verified from code only.
