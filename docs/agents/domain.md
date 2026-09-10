# Domain Docs

How the engineering skills consume this repo's domain documentation when exploring the codebase.

## Before exploring, read these

- **`CONTEXT.md`** at the repo root, if it exists.
- **`docs/adr/`** — ADRs that touch the area you are about to work in.
- **`.specify/memory/constitution.md`** — binding sctxx laws.
- **`docs/SCTXX-ROADMAP.md`** — direction, current milestone, invariants.
- **`docs/SPEC.md`** — architecture reference; read the sections the active block cites.
- **`specs/000-wayfinding/map.md`** — resolved decisions and open questions.
- **`src/vendor/codex/README.md`** — what was vendored from Codex, from which commit, and how it changed.

If `CONTEXT.md` or `docs/adr/` do not exist, proceed silently. The `/domain-modeling` skill creates them
lazily when terms or decisions actually get resolved. Superpowers brainstorming writes design notes to
`docs/superpowers/specs/` and plans to `docs/superpowers/plans/`, also lazily.

## File structure

Single-context repo:

```text
/
├── CONTEXT.md                 ← created lazily by domain-modeling
├── docs/adr/                  ← created lazily
├── docs/SCTXX-ROADMAP.md
├── docs/SPEC.md
├── docs/workmethodology/
├── docs/agents/
├── specs/                     ← Spec Kit + Wayfinder
├── src/  tests/  prompts/  schemas/  skill/  npm/  xtask/
└── .specify/memory/constitution.md
```

## Use the glossary's vocabulary

Use the terms defined in `docs/SPEC.md` and the glossary in `AGENTS.md`: IR, active branch, ledgers, masked
rows, tail, premap, fold, ops, items (goal, constraint, decision, dead end, env fact, open thread, current
step, next action, question), layers L0–L3, host mode, probes. One term, one meaning; do not invent synonyms
("summary" is not "artifact", "turn" is not "episode").
