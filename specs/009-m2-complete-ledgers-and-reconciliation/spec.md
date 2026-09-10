# Feature Specification: M2 - Complete ledgers, masking, and repo reconciliation

**Feature Branch**: `009-m2-complete-ledgers-and-reconciliation`
**Created**: 2026-09-10
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M2; docs/SPEC.md §7, §10.1, §10.2

## Scope seed

- Full S1 ledgers (plan, git, native compactions), S2 masking/tiers/segmentation/chunking (vendored tier budgeting).
- S5 reconciliation with the read-only git allowlist; redaction on every path.
- 10-session evaluation corpus with baseline deterministic-probe F1 for `--llm none`.

## Unlocked by

- No open Wayfinder ticket; unlocked when the previous milestone's exit criterion is met.

## Next step

Not approved scope. When unlocked, set `.specify/feature.json` to `specs/009-m2-complete-ledgers-and-reconciliation` and run
`/speckit-specify` to replace this stub with a full specification.
