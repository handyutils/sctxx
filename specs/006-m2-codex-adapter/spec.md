# Feature Specification: M2 - Codex CLI adapter

**Feature Branch**: `006-m2-codex-adapter`
**Created**: 2026-09-10
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M2; docs/SCTXX-SPEC.md §6.1, §6.2, Appendix A

## Scope seed

- Rollout line decoding, `.jsonl.zst`, `sessions/` and `archived_sessions/`, `CODEX_HOME`.
- Vendored rollback replay (`ThreadRolledBack`), forks with `--include-fork-parent`, `request_user_input` pairing, `apply_patch` header parsing.

## Unlocked by

- [Determine whether Codex `compacted` lines carry readable summaries](../000-wayfinding/issues/05-codex-compacted-readability.md)

## Next step

Not approved scope. When unlocked, set `.specify/feature.json` to `specs/006-m2-codex-adapter` and run
`/speckit-specify` to replace this stub with a full specification.
