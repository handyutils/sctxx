# Feature Specification: M1 - Deterministic handoff walking skeleton

**Feature Branch**: `004-m1-deterministic-handoff-skeleton`
**Created**: 2026-09-10
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M1; docs/SCTXX-SPEC.md §7.2 (subset), §7.3 tail, §12, §3.4–3.5

## Scope seed

- Minimal ledgers: files touched, commands with last status, unresolved error signatures, user messages.
- Recency tail, L0/L1/L3 markdown render with `[evt a–b]` pointers.
- `sctxx extract <ref> --llm none --out`, `sctxx expand`.
- Acceptance: < 5 s on a real ≥1,000-event session on the M1 Max; a Codex session continues correctly from the artifact.

## Unlocked by

- [Lock the sctxx first destination](../000-wayfinding/issues/01-lock-destination.md)
- [Choose where handoff artifacts live and whether sctxx hides them from git](../000-wayfinding/issues/07-artifact-location-policy.md)

## Next step

Not approved scope. When unlocked, set `.specify/feature.json` to `specs/004-m1-deterministic-handoff-skeleton` and run
`/speckit-specify` to replace this stub with a full specification.
