# Feature Specification: M1 - IR and Claude Code adapter

**Feature Branch**: `003-m1-ir-and-claude-code-adapter`
**Created**: 2026-09-10
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M1; docs/SCTXX-SPEC.md §5, §6.1, §6.3, §3.2

## Scope seed

- IR types limited to the event kinds the Claude Code adapter produces; invariants tested.
- Claude Code discovery (`~/.claude/projects`, `CLAUDE_CONFIG_DIR`, `--claude-root`) and content detection.
- Active-branch resolution over `parentUuid` trees and `logicalParentUuid` compaction boundaries; sidechains reduced to spawn/result.
- `claude:<id|prefix|last>` reference resolution; `show --view ir|raw`.

## Unlocked by

- [Lock the sctxx first destination](../000-wayfinding/issues/01-lock-destination.md)
- [Confirm Claude Code as the first provider for the walking skeleton](../000-wayfinding/issues/02-confirm-first-provider.md)
- [Map Claude Code subagent and sidechain storage across versions](../000-wayfinding/issues/04-claude-code-sidechain-layout.md)

## Next step

Not approved scope. When unlocked, set `.specify/feature.json` to `specs/003-m1-ir-and-claude-code-adapter` and run
`/speckit-specify` to replace this stub with a full specification.
