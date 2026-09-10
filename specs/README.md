# specs/ — Spec-Driven Development for sctxx

This tree is the working surface for three complementary methods:

| Method | What it owns here | When to use it |
| --- | --- | --- |
| [GitHub Spec Kit](https://github.com/github/spec-kit) | per-block `spec.md` / `plan.md` / `tasks.md` / `research.md` / `evidence/` | After a decision is made and a block can be specified |
| [Wayfinder](https://github.com/mattpocock/skills/tree/main/skills/engineering/wayfinder) (Matt Pocock) | `000-wayfinding/map.md` + child tickets | When the way is still foggy; tickets decide, they do not build |
| Matt Pocock SDD (`to-spec`, `to-tickets`, `triage`) | tracker conventions in `docs/agents/` | Publishing a PRD and slicing implementation tickets |

Governing policy: [`.specify/memory/constitution.md`](../.specify/memory/constitution.md).
Direction: [`docs/SCTXX-ROADMAP.md`](../docs/SCTXX-ROADMAP.md).
Architecture reference: [`docs/SCTXX-SPEC.md`](../docs/SCTXX-SPEC.md).
Working method: [`docs/workmethodology/sctxx-hybrid-engineering-methodology.md`](../docs/workmethodology/sctxx-hybrid-engineering-methodology.md).

## Layout

```text
specs/
  README.md
  000-wayfinding/                          Wayfinder map + decision tickets
    map.md
    issues/NN-slug.md
  001-m0-repo-foundation/                  M0
  002-m0-redaction-and-fixture-policy/     M0
  003-m1-ir-and-claude-code-adapter/       M1 walking skeleton
  004-m1-deterministic-handoff-skeleton/   M1
  005-m1-agent-skill-install/              M1
  006 … 009                                M2 three providers + honest deterministic artifact
  010 … 012                                M3 anchored LLM fold
  013 … 014                                M4 public v0.1.0
  015 … 016                                M5 measured quality
  017 … 021                                M6 everywhere agents work
  022 … 023                                M7 breadth and 1.0
```

Every block starts as a stub `spec.md` with `**Status**: Stub`. A stub is not approved scope. Fill it
only after the Wayfinder tickets listed under "Unlocked by" are resolved.

## Agent commands

Spec Kit skills live in `.claude/skills/speckit-*` and `.agents/skills/speckit-*`.

Typical order for one block:

1. Resolve the Wayfinder ticket that makes the question sharp.
2. Point `.specify/feature.json` at the block folder (so Spec Kit fills the stub instead of creating a new folder).
3. `/speckit-specify` → `spec.md`, then its requirements checklist.
4. `/speckit-clarify` for material uncertainty only.
5. `/speckit-plan` → `plan.md` (+ `research.md`, `contracts/`, `data-model.md` as needed).
6. `/speckit-tasks` → `tasks.md`; `/speckit-analyze` for consistency.
7. `/speckit-implement` for the current vertical slice only, with TDD and per-task evidence.
8. `/speckit-converge` before marking the block Completed.

## Tracker split

- **Decisions** stay in Wayfinder tickets (`000-wayfinding/issues/`) and, when architectural, in `docs/adr/`.
- **Feature specs** stay in `specs/NNN-*`.
- **GitHub Issues** are optional promotion, not the default store, so the checkout stays usable offline and
  across agents.

See `docs/agents/issue-tracker.md`.
