# Changelog

All notable changes to sctxx are recorded here. This project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html). Before 1.0, minor releases may
change CLI flags; the artifact schemas are versioned independently and change only with a schema
bump and a compatibility note.

## [Unreleased]

### Added

- **The artifact now opens itself in `sctxx --tui`.** When an extraction finishes, the pane switches to
  the artifact at L0 without another keypress, and `c` returns to it. Layers L0–L3 step by number or
  tab, and the whole thing scrolls — the point being that a developer who has to leave the TUI to read
  what they just produced will not read it.
  The `[evt a–b]` pointers in the artifact are live: put the cursor on one and press enter, and the
  events behind it are read through **the same code `sctxx expand` uses**, not a second implementation
  of it. Reading takes the whole body, because prose in half a terminal is not reading.
  `o` opens an artifact you already have — a previous run's `.sctxx/`, or a colleague's `handoff.md` —
  by path, so the pane is a viewer and not only a receipt.

- **`h` in `sctxx --tui` hands the work to another agent.** It lists the agents installed on this
  machine with the version each reported and whether the seeding channel was verified on it, shows the
  exact command that will run, and — only on a second, explicit confirmation — hands the terminal over
  so the new session starts with the handoff already in its first turn.
  The artifact travels as a **path**, never as content on a command line, and the pointer sentence is
  fixed prose plus that path, so nothing from a session can become part of a command. An agent at a
  version whose seeding was never verified gets the fallback route with no flags at all, and the pane
  says so rather than guessing.
  The launched agent is a full-screen application, so it gets the whole terminal rather than a pane
  inside the TUI, which also means no PTY or terminal-emulator dependency
  ([ADR 0006](docs/adr/0006-hand-over-the-terminal-to-the-launched-agent.md)).
  Verified against real launches of Claude Code 2.1.268, Codex CLI 0.153.4 and Pi 0.85.1: each
  returned the source session's id, which appears only inside the artifact, so the handoff genuinely
  reaches the receiving agent. Note that an agent asked to work in a directory it has not seen before
  will first ask you to trust it — that prompt is the agent's, and sctxx never bypasses it.

- **`sctxx doctor` reports the coding agents a handoff can launch**, with the version each one
  answered and whether that version is one the seeding channel was verified on. An agent installed at
  an unverified version is reported as installed *with the fallback named*, never quietly treated as
  ready. The detector asks whether the *binary* is on `PATH`, not whether a store directory exists —
  a leftover directory must not advertise an agent that cannot be launched.

## [0.2.0] - 2026-09-11

### Added

- **`sctxx --tui`: an interactive session browser** (M8, first slice). It lists every session the CLI
  can see and lets you narrow it without knowing an id: `/` searches fuzzily across titles, first
  messages, ids and directories, and `a` / `r` / `p` filter by agent, recency (24h / 7d / 30d) and
  "this project". A pasted id or path matches exactly and outranks every fuzzy hit. The preview pane
  shows the session's agent, id, time, cwd, size, title, first message and path — read from discovery,
  with the transcript untouched, so browsing costs nothing.
  Behind the `tui` feature (on by default, excluded by `--no-default-features`).
- `sctxx --tui` refuses to start when stdout is not a terminal, with exit 2 and a message naming the
  CLI alternatives, so a piped or agent-driven invocation can never hang on a key press.
- **`sctxx --tui`'s preview now shows what is *in* a session**, not just where the file is: user turns
  on the active branch, live versus total events, the files the work was about (busiest first), the
  last command and whether it failed, unresolved error signatures, provider compactions split into
  window re-anchors and history discards, and any parse diagnostics. All of it comes from the
  deterministic ledgers, so no model, key, or network is involved.
  Reading a transcript takes seconds, so it happens on a worker thread: the pane says it is reading
  while it works, a 150 ms settle delay keeps a held `j` from queuing reads for sessions you are
  already past, a superseded read is dropped before it starts, and results are cached so returning to
  a session is instant. A session that cannot be read shows why and stays selectable.
- **`sctxx --tui` can extract.** `e` opens an extraction form and `enter` runs it. The form is built
  from clap's own definition of `extract`, and submitting goes back through clap, so it cannot gain a
  field the CLI does not have, lose one it does, or accept a value the CLI would reject — there is a
  test that fails if the form and the command line ever disagree. Progress streams into the pane stage
  by stage, and when it finishes the pane reports every file written, the handoff path, and the CLI's
  own "not ignored by git" warning, through the CLI's own function rather than a second copy of the
  wording.
  A relative destination is resolved against the session's project, which is what a developer means by
  `.sctxx/` when they opened the TUI from somewhere else.

- **`sctxx update` updates an installed copy the way it was installed.** sctxx ships through crates.io
  and npm, which are updated by different tools, so it decides from its own executable path: inside a
  `node_modules` directory means npm (`npm install -g sctxx@latest`), cargo's bin directory means
  `cargo install sctxx --force`. It prints what it detected, why, and the exact command before running
  it, so the choice can be argued with rather than trusted. `--check` prints the plan and stops.
  An install it did not make — a distribution package, a container image, a checkout build — is
  refused with both installer commands named rather than guessed at.

### Changed

- **MSRV 1.85 → 1.88.** `ratatui` 0.30.1+, `ignore` 0.4.31+ and `tui-markdown` all require it, and the
  TUI is in this crate. `rust-version` is per-package, so a feature cannot carry its own. Recorded in
  `Cargo.toml`, the CI job, the constitution, and `AGENTS.md`; reasoning in
  `docs/adr/0003-tui-stack-and-msrv.md`.

### Fixed

- **`--llm cli:<agent>` no longer writes sessions into your agent's history.** Every completion left a
  real session behind: the backends run the agent in a scratch directory, but Claude Code records a
  session *per working directory*, so each call created `~/.claude/projects/<scratch>/…jsonl` whose
  transcript was sctxx's own prompt — and `sctxx list` then reported those as sessions. The scratch cwd
  never prevented this; it only named the pollution. Each template now passes its CLI's own persistence
  switch (`claude --no-session-persistence`, `codex exec --ephemeral`, `pi --no-session`), and a test
  fails if a template loses it.
- The `cli:` templates now record the agent CLI version each argv was verified against, and report it in
  a backend failure, so a CLI that moves a flag is diagnosable from the error rather than from silence.
  Verified on Claude Code 2.1.268, Codex 0.153.4, Pi 0.85.1.
- Sessions already written by earlier versions are **not** deleted: they are in your store, and removing
  them is your call. They are identifiable by a `sctxx-llm-` path component under the system temp
  directory.

## [0.1.3] - 2026-09-11

### Documentation

- **The npm packages have READMEs, so `npmjs.com/package/sctxx` is no longer an empty page.** The
  earlier versions were published without one, and npm versions are immutable, so it took a release to
  fix. The page now carries the quick start, five concrete use cases, the artifact's layers, and — as
  the first question a careful engineer asks — exactly which algorithms are ported from OpenAI's Codex
  CLI, which upstream file each one comes from, and what it does for you. It also says what is *not*
  taken: no dependency on any `codex-*` crate, and no affiliation with or endorsement by OpenAI.

## [0.1.2] - 2026-09-11

### Fixed

- **npm: the Windows-on-ARM package is now `sctxx-windows-arm64`.** npm's spam detection refuses
  `sctxx-win32-arm64` for this publishing account — it did so at 0.1.0 and again at 0.1.1 — and
  because that package publishes fifth in the loop, its failure also skipped `sctxx-win32-x64@0.1.1`
  and the `sctxx` wrapper, leaving `npm i -g sctxx` serving 0.1.0. A different name publishes without
  complaint, so the package is renamed and the seven names now ship together. Nothing a user types
  changes: the wrapper still resolves the right binary per platform.

## [0.1.1] - 2026-09-11

Found by pointing the tool at real sessions for the first time, plus npm as a third install channel.

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

### Fixed

- **Every artifact's header reported `masked: 0, artifact: 0`.** The line a reader uses to judge how
  much of the session was discarded said nothing was. Neither number is computable where it was
  written — the masked count belongs to the pipeline, and the artifact's size is the size of the text
  being rendered — so both now come from the caller, with a first render measuring the artifact so the
  header can state its own size.
- **`extract --out` outside a repository printed git's own errors.** The git-ignore check used
  `Command::status`, which hands the child the parent's stderr, so `fatal: not a git repository`
  appeared at the user from a check whose normal answer is exactly that.

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

[Unreleased]: https://github.com/handyutils/sctxx/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/handyutils/sctxx/releases/tag/v0.2.0
[0.1.3]: https://github.com/handyutils/sctxx/releases/tag/v0.1.3
[0.1.2]: https://github.com/handyutils/sctxx/releases/tag/v0.1.2
[0.1.1]: https://github.com/handyutils/sctxx/releases/tag/v0.1.1
[0.1.0]: https://github.com/handyutils/sctxx/releases/tag/v0.1.0
