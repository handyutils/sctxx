# Determine whether Codex `compacted` lines carry readable summaries

Type: research
Status: open

## Question

At the pinned Codex commit and in real rollouts from current Codex CLI versions, when does a `compacted`
rollout line contain readable summary text, and when does it carry only replacement history or encrypted
remote-compaction items? What should the adapter emit in each case?

Verify against the pinned source (`codex-rs/history`, `codex-rs/core/src/compact*.rs`) and at least two real
rollouts (one using an OpenAI-hosted model, one using another provider).

Unblocks: `specs/006-m2-codex-adapter/`. Spec reference: `docs/SPEC.md` §6.2, §19 item 2.
