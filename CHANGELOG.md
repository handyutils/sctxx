# Changelog

All notable changes to sctxx are recorded here. This project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, minor releases may
change CLI flags; the artifact schemas are versioned independently and change only with a schema
bump and a compatibility note.

## [Unreleased]

### Added

- **npm distribution**: `npm i -g sctxx` installs the prebuilt binary for the user's platform. A
  wrapper package declares six per-platform packages as `optionalDependencies` (each carrying `os`
  and `cpu`), so npm installs exactly one and needs no Rust toolchain and no postinstall download.
  Published with `--provenance`. The packages are unscoped (`sctxx-darwin-arm64` and friends) rather
  than the `@sctxx/*` the spec proposed, because npm scopes need an organisation this account does
  not have; the command users type is unchanged. See `npm/README.md`.
- **`extract --max-bad-lines <RATE>`** (spec §3.1): the fraction of lines allowed to fail parsing,
  default `0.02`. It was documented and internally supported but never exposed, so a session with one
  unrecognized line type from a newer provider version failed with exit 5 and no way out. The error now
  names the flag.
- **`extract --out` warns when the artifact directory is not git-ignored.** An artifact quotes the
  session — user messages verbatim, paths, error output — so a `git add -A` in the user's project
  could commit and push it. `extract` now checks with `git check-ignore` (read-only, allowlisted) and
  prints the exact command to exclude the directory. sctxx never edits your git config itself.
- **Prior provider summaries now reach the fold as low-trust seeds** (spec §7.2). If a provider
  compacted before the chunk being folded, the fold prompt carries those summaries with their event
  numbers, framed as a hint rather than evidence — a chunk several turns after a compaction boundary
  previously saw nothing of what came before it. Bounded to the three most recent, 400 tokens each, so
  a heavily compacted session cannot push the transcript out of the prompt. `fold_user` is version 2.

## [0.1.0] - 2026-09-11

First release. Reads a coding-agent session from disk and writes a verified, provenance-linked
handoff artifact another agent can continue from.

### Added

- **`extract --since-compact`**: start from the newest provider compaction boundary and keep that
  boundary's summary as a low-trust seed. The run names the boundary on stderr; a session that never
  compacted is a notice rather than an error.
- **Compaction kind in the IR**: `NativeCompaction` records whether a boundary was a *window
  re-anchor* (Codex `compacted` with a `window_number`, transcript intact) or a *legacy history
  reset*. `--since-compact` prefers the newest reset, and falls back to the earliest re-anchor.
  See `docs/adr/0002-codex-compaction-algorithm-reuse.md`.

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

### Fixed

- Reading Codex `compacted.replacement_history`: a message envelope nests its text under `content`,
  so a local compaction summary was previously read as empty. Nesting is now walked, bounded to eight
  levels so a corrupt session file cannot exhaust the stack.
- **The published crate no longer contains the website's `node_modules`.** Cargo matches the `include`
  list the way gitignore does, so the bare `README.md`, `LICENSE`, `NOTICE`, and `CHANGELOG.md`
  entries matched files at every depth — 118 of them came from `website/node_modules`. Every entry is
  anchored to the package root, and CI fails on any file outside the expected set.
- Windows: `.gitattributes` pins LF in the working tree. `source_hash` is a hash of the session file's
  bytes, so a CRLF checkout produced different hashes and failed the pipeline snapshots there while
  macOS and Linux passed.

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
