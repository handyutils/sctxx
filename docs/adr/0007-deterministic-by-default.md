# ADR 0007 — Deterministic by default; the model is opt-in

- **Status**: accepted
- **Date**: 2026-09-11
- **Affects**: `docs/SCTXX-SPEC.md` §3.4 (`sctxx extract` defaults), §3.3 (command tree),
  `skill/references/cli.md`, `README.md`, `CHANGELOG.md`
- **Related**: [ADR 0004](0004-handoff-launch-and-seeding.md) (what a handoff does),
  [ADR 0006](0006-hand-over-the-terminal-to-the-launched-agent.md)

## Context

`sctxx extract` defaulted to `--llm auto`, and `auto` resolves to the best available backend — on the
maintainer's machine, `cli:claude`. Nothing about that default said what it would cost.

Measured, on a real session, with `sctxx extract <ref> --dry-run` (which calls no model):

```text
events:         103727 (46634 active, 273 user turns)
masked rows:    17088  (812830 tokens)
chunks to fold: 40     (807372 tokens)
recency tail:   92 rows (5458 tokens)
backend:        cli:claude
planned calls:  41 fold + 40 premap
```

**About 813,000 input tokens across 81 model calls**, from a command whose documentation says it
"turns a session into a compact, verified, provenance-linked handoff artifact" and whose constitution
says "deterministic first".

Two things made this worse than a bad default:

1. **The default was reached by accident.** The TUI's extraction form put `enter` on "run", so a
   keystroke that reads as "next field" started those 81 calls, with no confirmation, no estimate, and
   no way to stop it. That is fixed in the TUI (ADR-free: the run row is explicit and cancellable), but
   the default that made it expensive is this decision.
2. **`--dry-run` misreported it.** For a `--llm none` run it still printed `planned calls: 1 fold`,
   because it reported what a fold *would* call rather than what the run *will* call — and that number
   is exactly what someone consults before deciding.

## Decision

**`--llm` defaults to `none`.** The deterministic artifact is the default product; a model is asked for
by name.

`auto`, `cli:<agent>`, `api:<provider>`, and `mock` are all unchanged and still available. What changes
is that none of them happens unless a developer writes it.

The reasoning is not cost alone:

- **The deterministic artifact is complete.** Every `[evt a–b]` pointer, every ledger, the recency tail,
  the retrieval index. A receiving agent can follow any pointer with `sctxx expand` and reach the
  original evidence. This is the thing the tool promises.
- **The fold is an enhancement, not a prerequisite.** It adds typed items, decisions, and constraints
  read out of the transcript by a model. Valuable, and orthogonal to whether a handoff works.
- **Determinism is the constitution's first rule.** A default that silently leaves the machine and
  spends a subscription contradicts it.
- **It is the difference between 24 seconds and an unbounded wait.** The same 103k-event session:
  `--llm none` produced all five files in **24 seconds, 0 tokens**. The fold is 81 calls to a model.

## Consequences

- **A breaking change to a default**, recorded in the CHANGELOG under 0.3.0. Anyone who scripted
  `sctxx extract` expecting a folded artifact gets the deterministic one; `report.json` records
  `llm: none`, and the fix is one flag. Pre-1.0, and the spec's §3.4 table is updated in the same
  change.
- **`sctxx handoff` and the TUI's `h` both call the deterministic path through one shared function**
  (`ExtractArgs::deterministic`), so the two cannot disagree about what a handoff costs. There is no
  second definition of "the cheap one".
- **`--dry-run` now reports the run being planned**, not the run that could have been. Under `none`,
  planned calls and estimated prompt tokens are both zero. A dry run whose numbers describe a different
  command is worse than no dry run.
- **The expensive path is still one flag away**, which is the point: it is a decision, not a trap.

## What this does not change

The fold, the ops model, the validation, and the anchoring are untouched. Nothing about what an
LLM-folded artifact contains changes; only whether you get one without asking.
