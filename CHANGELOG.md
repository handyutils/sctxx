# Changelog

All notable changes to sctxx are recorded here. This project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, minor releases may
change CLI flags; the artifact schemas are versioned independently and change only with a schema
bump and a compatibility note.

## [Unreleased]

### Added

- **`extract --since-compact`**: start from the newest provider compaction boundary and keep that
  boundary's summary as a low-trust seed. The run names the boundary on stderr; a session that never
  compacted is a notice rather than an error.
- **Compaction kind in the IR**: `NativeCompaction` now records whether a boundary was a *window
  re-anchor* (Codex `compacted` with a `window_number`, transcript intact) or a *legacy history
  reset*. `--since-compact` prefers the newest reset, and falls back to the earliest re-anchor.
  See `docs/adr/0002-codex-compaction-algorithm-reuse.md`.

### Fixed

- Reading Codex `compacted.replacement_history`: a message envelope nests its text under `content`,
  so a local compaction summary was previously read as empty. Nesting is now walked, bounded to eight
  levels so a corrupt session file cannot exhaust the stack.
- **The published crate no longer contains the website's `node_modules`.** Cargo matches the `include`
  list the way gitignore does, so the bare `README.md`, `LICENSE`, `NOTICE`, and `CHANGELOG.md` entries
  matched files at every depth — 118 of them came from `website/node_modules`. Every entry is now
  anchored to the package root, the tarball is back to 66 files, and CI fails on any file outside the
  expected set.

## [0.1.0] - 2026-09-11

First release. Reads a coding-agent session from disk and writes a verified, provenance-linked
handoff artifact another agent can continue from.

### Added

- **Adapters** for Claude Code, Codex CLI, and Pi, each resolving the live conversation:
  `parentUuid` trees with `logicalParentUuid` across compaction boundaries, `ThreadRolledBack`
  replay, and Pi's v2+ tree walk. Claude Code subagent transcripts stored in
  `<session-id>/subagents/` are recognized and attached with `--include-sidechains`.
- **Deterministic ledgers**: files touched, commands with last-known status, error signatures
  with resolution state, the last published plan, git activity, and user messages.
- **Anchored fold**: an isolated premap pass, a sequential fold emitting typed operations, and a
  final pass over the recency tail. Every operation passes validation gates before it can change
  state, with one repair turn on rejection.
- **Validation gates**: provenance must lie inside the chunk the model was shown and cite an
  event it actually saw; a constraint must quote a human message verbatim; constraints can never
  be dropped; length and per-kind limits are enforced. Rejections are recorded in `state.json`.
- **Repository reconciliation** over a read-only git allowlist, marking items `verified`,
  `stale`, or `contradicted`. The repository always wins.
- **Redaction** of 12 secret classes by default and 15 with `--redact strict`, applied before
  any model call, on model output, and on the rendered artifact.
- **Commands**: `list`, `find`, `show`, `extract`, `expand`, `verify`, `redact`, `skill`,
  `schema`, `doctor`.
- **Backends**: `none`, `cli:claude`, `cli:codex`, `cli:pi` (isolated temp cwd, tools disabled),
  `api:anthropic`, `api:openai`, `api:compat` for any OpenAI-compatible endpoint, and `auto`.
- **Agent Skill** (`sctxx skill install`) for Claude Code, Codex, and Pi, which refuses to
  overwrite a locally modified `SKILL.md`.
- **Published contracts**: exit codes, and the `sctxx.handoff/v1`, `state.v1`, `ops.v1`, and
  `ir.v1` JSON Schemas, printable with `sctxx schema`.

### Provenance

Includes code derived from [OpenAI Codex](https://github.com/openai/codex) (Apache-2.0) at
commit `818f1cca8ccf8899f0f4d59336baebaccf358eed`: UTF-8-safe truncation, secret redaction,
tiered evidence budgeting, rollback-aware replay, and the `apply_patch` header grammar. See
`src/vendor/codex/README.md`.

### Known gaps

Tracked in `docs/SCTXX-ROADMAP.md`: the probe loop and `sctxx eval` (M5); cache, resume, and
incremental updates; host mode and an MCP server (M6). `--mode full` currently behaves as
`standard` and says so.

[Unreleased]: https://github.com/handyutils/sctxx/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/handyutils/sctxx/releases/tag/v0.1.0
