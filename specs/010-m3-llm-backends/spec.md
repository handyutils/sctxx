# Feature Specification: M3 - LLM backends

**Feature Branch**: `010-m3-llm-backends`
**Created**: 2026-09-10
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M3; docs/SCTXX-SPEC.md §9

## Scope seed

- `LlmBackend` trait, JSON schema handling and repair, `cli:claude|codex|pi` in isolated temp cwd, `api:anthropic|openai|compat`, `auto` resolution, cost estimation.

## Unlocked by

- [Decide whether v0.1.0 includes the LLM fold](../000-wayfinding/issues/03-v0-1-release-scope.md)
- [Verify non-interactive command templates for agent CLIs used as LLM backends](../000-wayfinding/issues/06-cli-backend-command-templates.md)

## Next step

Not approved scope. When unlocked, set `.specify/feature.json` to `specs/010-m3-llm-backends` and run
`/speckit-specify` to replace this stub with a full specification.
