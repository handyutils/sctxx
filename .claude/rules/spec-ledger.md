---
paths:
  - "specs/**"
  - "docs/SCTXX-ROADMAP.md"
  - ".specify/**"
---

# Spec ledger and roadmap

- `docs/SCTXX-ROADMAP.md` changes only after an explicit direction decision; never check implementation items
  in it.
- A `spec.md` with `**Status**: Stub` is not approved scope. Before `/speckit-specify`, set
  `.specify/feature.json` to that folder so the stub is filled in place.
- Check a task in `tasks.md` only when `evidence/T###.md` exists with RED, GREEN, gate, and the full commit SHA.
- A block is Completed only after `/speckit-converge` finds no specified-but-unbuilt work.
- Stopped work gets `invalidated.md` (or a `Superseded` status naming the successor); never leave it looking
  active.
- Wayfinder: claim (`Status: claimed`) before working a ticket; resolve by appending `## Answer`, setting
  `Status: resolved`, and adding a one-line gist to `specs/000-wayfinding/map.md`. `Type: grilling` tickets
  are answered by the user only.
- Never put unredacted session content in any ledger file.
