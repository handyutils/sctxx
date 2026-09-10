# Feature Specification: M0 - Redaction and fixture intake policy

**Feature Branch**: `002-m0-redaction-and-fixture-policy`
**Created**: 2026-09-10
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M0; docs/SCTXX-SPEC.md §10.2, §15.1, §17

## Scope seed

- `sctxx redact <file> [--strict] [--out PATH]` on raw JSONL, independent of adapters.
- Vendored Codex secret patterns plus the additional classes in spec §10.2; strict mode.
- Fixture contribution policy (consent, strict redaction, human review), `tests/fixtures/<agent>/README.md` template, case-insensitive-safe naming.

## Unlocked by

- No open Wayfinder ticket; unlocked when the previous milestone's exit criterion is met.

## Next step

Not approved scope. When unlocked, set `.specify/feature.json` to `specs/002-m0-redaction-and-fixture-policy` and run
`/speckit-specify` to replace this stub with a full specification.
