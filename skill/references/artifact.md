# The handoff artifact

`sctxx extract --out .sctxx/` writes five files. Read `handoff.md`; the rest exist so a claim can
be checked or a decision reviewed.

| File | What it is | When you need it |
| --- | --- | --- |
| `handoff.md` | The artifact, in layers | Always |
| `handoff.json` | The same content, schema `sctxx.handoff/v1` | Programmatic use |
| `state.json` | Every item including superseded, resolved, and dropped ones, plus the operation audit trail | "Why does the artifact claim this?" |
| `ledgers.json` | All deterministic records: files, commands, errors, plan, git | Full file or command lists |
| `report.json` | Diagnostics, token counts, timings, backend warnings | Something looks wrong |

## Layers

**L0 Brief** (≤ 1,200 tokens) - goal, current step, next actions, hard constraints with the
user's verbatim words, dead ends, verify-first commands, and what changed in the repository since
the session ended. If you read nothing else, read this.

**L1 Items** - every active item with its id, confidence, verification status, and `[evt a–b]`
provenance, then the ledgers: files touched, last known command status, unresolved errors, the
last published plan, and git activity.

**L2 Recency tail** - masked rows from the end of the session, near-verbatim. This is what the
agent was actually doing when the session stopped.

**L3 Retrieval** - the source file and ready-to-run `sctxx expand` commands.

## Item ids

An id says what the item is: `G`oal, `C`onstraint, `D`ecision, dead end (`X`), env `F`act,
`O`pen thread, current `S`tep, `N`ext action, `Q`uestion.

## Reading the annotations

- **confidence** `high | medium | low` - how well the evidence supported the claim.
- **verification** - the result of comparing the item against the current repository:
  - `verified` - the files it names exist and were not touched after the session.
  - `stale` - a file it names is missing or changed since the session. Check before acting.
  - `contradicted` - the repository disproves it. Do not act on it.
  - `unchecked` - reconciliation did not apply, usually because the item names no path.
- **`[evt a–b]`** - the canonical event range that justifies the item. Expand it to see the
  original rows.
- **`inferred`** on a file - the path came from a shell-command heuristic, not an edit tool. It is
  a hint, not a fact.
- **low-trust prior summary** - the session contained a provider compaction summary. It was used
  only as a seed; it is lossy and may be wrong.

## What is guaranteed

- Every item cites at least one event range inside the session.
- Every constraint quote appears verbatim in a message a human actually typed.
- Files, commands, errors, plans, and git activity are computed in Rust from the transcript, not
  asked of a model. They are true even when `llm: none`.
- Secrets were redacted before any model call and again on output. Redaction is best-effort
  pattern matching, so never paste artifact content into a public issue without reading it.
