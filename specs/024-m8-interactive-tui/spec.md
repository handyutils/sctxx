# Feature Specification: M8 - Interactive TUI: find a session, extract it, hand it off

**Feature Branch**: `024-m8-interactive-tui`
**Created**: 2026-09-11
**Status**: Active — all four open questions resolved 2026-09-11 (ADR 0003, ADR 0004, ADR 0005);
`tasks.md` is authoritative and T2401–T2402 are done
**Input**: new roadmap milestone M8; `docs/SCTXX-SPEC.md` §3, §12, §13; direction from the maintainer,
2026-09-11

> **Roadmap change.** This block adds a milestone that does not exist in `docs/SCTXX-ROADMAP.md`.
> The methodology requires an explicit decision for that, and the maintainer made one: `sctxx --tui`
> is the feature that turns extraction from a utility you remember into the way you move work between
> agents. The roadmap gains M8; the invariant list and exit criteria are unchanged.

## Why this exists

`sctxx` today is a command you have to *remember*. To move work from one agent to another you must
know the session id, remember `extract`, pick a backend, and know where the artifact should land —
and then, separately, start the receiving agent and tell it to read the file. Every step is a place
to stop.

That last mile is the whole point. Extraction without handoff still leaves you typing the same
sentence into a fresh session. This block closes it:

```text
pick agent ─► filter/search ─► pick session ─► extract ─┬─► write artifacts to a chosen path
                                                        └─► launch a new session in a chosen
                                                            installed agent, context pre-loaded
```

The measure of success is that a developer who has never read the docs can go from "the session I
want is in Codex" to "the new agent is already working on it" without leaving the terminal or
knowing a single flag.

## Actors and goals

- **The developer mid-task.** Finished with one agent, wants another to continue, and does not want to
  lose momentum or context. Primary actor; every requirement serves their flow.
- **The receiving agent.** Starts already knowing the goal, the current step, the constraints, and
  what was already tried — because the handoff was seeded before it opened its mouth.
- **The maintainer.** Wants the TUI to be a *view* over the existing pipeline: no second IR, no second
  config system, no parallel artifact format, and no new licensing class without attribution.

## The screen

An icon rail on the left; the selected pane fills the rest, with a status line at the bottom.

| Rail position | Pane | Origin |
|---|---|---|
| **1 (default)** | **SCTXX** — session browser, filters, preview, extraction, handoff | new, this block |
| 2 | Files — file tree of the session's `cwd` | reuse, croft |
| 3 | Search — find-in-files | reuse, croft |
| 4 | Terminal — PTY running the launched agent session (and any shell) | reuse, croft |
| 5 | Canvas — the rendered artifact, layer by layer | reuse, croft |

The SCTXX pane is the one that must be excellent. The other four are borrowed surface area: they make
the tool a place to work rather than a dialog, but none of them is the reason a developer opens it.

## User scenarios

**US1 — Hand off to another agent (P0).** I open `sctxx --tui`, the SCTXX tab is already showing my
recent sessions. I type a few characters of a keyword, arrow down to the one I want, and press enter
to see a preview. I press `h` for handoff, choose `claude` from the list of installed agents, choose
where the artifact goes, and confirm. The Terminal pane opens with a *new* Claude Code session whose
first message already contains the handoff. I keep working without retyping the goal.

**US2 — Find the session I mean (P0).** I do not know the id and there are 640 sessions on this
machine. I filter by agent (`codex`), narrow by date (this week), and type a fuzzy match on what I
remember of the first message; the list narrows as I type. I can also paste a session id or a
transcript path and jump straight to it.

**US3 — Extract to a file and stop (P0).** I want the artifact, not a new agent: I extract to
`.sctxx/`, the pane tells me it wrote five files and that the directory is not git-ignored (the same
warning the CLI gives), and I can open the Canvas pane to read `handoff.md` without leaving the TUI.

**US4 — Understand a session before extracting (P1).** Before committing to a backend choice I want
to know what is in there: how many events, how many are live after rollbacks, the user turns, the
files touched, the unresolved errors. The preview shows the deterministic ledgers, which cost nothing
and need no model.

**US5 — Choose the backend knowingly (P1).** Extraction can run with `--llm none`, with a CLI backend
(`cli:claude`, `cli:codex`, `cli:pi`), or with an API backend. The pane shows which are actually
available on this machine — the same answer `sctxx doctor` gives — so the choice is informed rather
than guessed, and `--llm none` is always offered because it always works.

**US6 — Every CLI capability is reachable (P2).** The pane lists what `sctxx` can already do and lets
me run it: `list`, `find`, `show`, `extract`, `expand`, `verify`, `redact`, `skill`, `schema`,
`doctor`. Nothing is TUI-only; nothing that exists in the CLI is missing from the TUI.

## Functional requirements

**Launch and layout**

- **FR-001** `sctxx --tui` starts the interactive mode. It must not change the behaviour of any
  existing invocation, and it must not print anything but the TUI on a terminal.
- **FR-002** When stdin or stdout is not a TTY, `--tui` fails with a usage error (exit 2) explaining
  that interactive mode needs a terminal — agents pipe output and must keep getting the CLI behaviour.
- **FR-003** The rail's first entry is SCTXX and is selected on start. Panes are switchable by key and
  by click; `q` quits; `?` shows the keymap.
- **FR-004** The TUI never writes partial artifacts to the terminal. All payload goes to files or to
  the panes; stdout stays reserved exactly as it is today once the TUI exits.

**Session browser (US1, US2)**

- **FR-005** The list is populated from the existing discovery layer
  (`adapters::discovery::list_all` / `list` / `summarize`) — not a second scanner. Fields shown per
  row: agent, id (short), started-at, cwd, first user message, event count.
- **FR-006** Filters, applied together and reflected immediately: agent (any / one / several), date
  range (today, this week, this month, custom), cwd (this project / any), and a free-text query.
- **FR-007** The free-text query is fuzzy over title, first user message, and user messages; a
  session id or a transcript path matches exactly and ranks first.
- **FR-008** The list stays responsive with 1,000+ sessions: filtering never blocks the UI, and the
  first paint happens before every session has been summarised (progressive fill).
- **FR-009** Selecting a session shows a preview built from the deterministic ledgers only: goal, user
  turns, files touched, last command status, unresolved errors, native compactions, and the live/total
  event counts. No model is called to render a preview.
- **FR-010** A session whose file cannot be parsed shows its diagnostic and stays selectable; one bad
  session never empties the list.

**Extraction (US3, US5)**

- **FR-011** The extraction form exposes exactly the `extract` options that exist —
  `--mode`, `--llm`, `--budget`, `--tail`, `--focus`, `--since-compact`, `--max-bad-lines`,
  `--redact`, `--include-sidechains`, `--no-verify`, `--layers` — with today's defaults, and no
  option that the CLI does not have.
- **FR-012** Available backends are probed the way `doctor` probes them; unavailable ones are shown
  as unavailable with the reason, never hidden silently.
- **FR-013** Progress streams into the pane using the pipeline's existing stage/message progress
  (`parse`, `ledgers`, `segment`, `fold`, `verify`, `since-compact`), and cancellation leaves the
  process and any temp state clean.
- **FR-014** The destination is chosen explicitly: a directory (all five files), a single `.md` or
  `.json` path, or "handoff" which implies a directory. The pane reports the written paths.
- **FR-015** The git-ignore warning the CLI emits for a non-ignored output directory is shown in the
  pane too — the same rule, the same wording, not a second implementation of it.
- **FR-016** The extraction result is readable **immediately and in place**: on success the Canvas pane
  opens the artifact at L0 without another keypress, and the reader can step through L0–L3, follow
  `[evt a–b]` pointers through the existing `expand` path, and see the ledgers alongside. The point is
  to understand what is about to be handed to another agent *before* handing it over — a developer who
  has to leave the TUI to read the artifact will not read it.
- **FR-016a** The same view is reachable for an existing artifact on disk (a previous run, a colleague's
  `.sctxx/handoff.md`) by path, so the pane is a viewer and not only a receipt.

**Handoff (US1 — the reason for this block)**

- **FR-017** The TUI lists installed coding agents by **probing binaries** (`which`-equivalent) and
  their stores, reporting the version it found. Note: `agentman` does *not* do this — it only checks
  whether a store directory exists under `$HOME` — so this is new code, informed by the launcher's
  detector (`~/.scripts/aiagents_scripts/detect-all-coding-ai-agents-on-this-machine-and-suggest-missing.py`),
  whose catalogue probes agents as `cmd:<binary>` / `path:` / `app:` / `glob:`.
- **FR-018** Choosing a target launches a **new** session of that agent in the Terminal pane, seeded
  with the extracted context so its first turn already has the goal, the constraints, and the next
  action.
- **FR-019** Seeding is designed per agent and verified against the installed CLI version. There is no
  existing mechanism to reuse: the maintainer's launcher has **no** prompt, file, or stdin channel —
  its only input is extra argv, and its only context pre-loading is the cwd (so the agent picks up
  `AGENTS.md`). The candidate surfaces to specify and test are the positional prompt
  (`claude "<handoff>"`, `pi "<handoff>"`), a non-interactive exec form (`codex exec "<handoff>"`), and
  the artefacts-in-cwd route (`--out .sctxx/` then a one-line prompt naming the file). Whichever is
  chosen per agent must be recorded with the CLI version it was verified on, and must fall back to the
  cwd route rather than guessing.
- **FR-020** The launch passes the handoff **as an argument or a path**, never as interpolated shell
  text. Transcript content is data; it must not become part of a command line (constitution I and
  `AGENTS.md` rule 5).
- **FR-021** Only agents on an allowlist may be launched, the launch is shown before it runs, and the
  cwd is the session's `cwd` when it still exists (the artifact's reconciliation already knows this).
- **FR-021a** **The handoff is re-redacted immediately before it is passed to another agent**, as a
  second pass over the artifact text, and the pane reports what was removed (count and classes, never
  the secrets). The artifact is already redacted three times on the way out (masked rows, model output,
  rendered file), so this is deliberately redundant: egress to a different tool is the one boundary
  where redundancy is cheap and a mistake is unrecoverable. `--redact strict` is available here and
  applies to the handoff only, never silently to the artifact on disk.
- **FR-021b** The launch is a **separate, explicit confirmation** from extraction. Producing an artifact
  never starts another agent as a side effect; a developer who only wanted the file must be able to stop
  at the file.
- **FR-022** The terminal pane owns its child process: it dies with the pane, with the TUI, or on
  cancellation, and a crashed child degrades to a message rather than taking the TUI down.

**Everything else (US6)**

- **FR-023** Every existing subcommand is reachable and its output is shown in the appropriate pane
  (`show`/`expand` in Canvas, `doctor`/`schema` in a pager view, `skill install` as an action).
- **FR-024** The TUI writes no new state beyond what the CLI already writes (cache, artifacts). There
  is no TUI-specific config file; settings come from the same config the CLI reads.
- **FR-025** The TUI is inside the existing crate: library code stays the source of truth, and no
  behaviour exists only in the TUI.

## Reuse: croft (MIT) and agentman

Two existing codebases are in scope, and they are in scope differently.

**croft** (`github.com/vitali87/croft`, MIT, checked out at `../croft`, v0.1.941) is a VS Code-shaped
TUI in Rust: 178 files, **281,489 lines (~161k non-test)**, plus LSP, DAP, MCP, sqlite, docx, iTerm2,
ghostty, collaboration, and agent lanes that `sctxx` has no use for. Its stack is the one this block
wants: **ratatui 0.30 + crossterm 0.29**, **portable-pty 0.9 + alacritty_terminal 0.26** for the
terminal, **`ignore` + `grep-searcher`/`grep-regex`/`globset`** for search, and **tree-sitter** for
highlighting.

A survey (2026-09-11) found the porting cost is real but bounded, and unevenly distributed:

| Pane | croft module | Prod LOC | Verdict |
|---|---|---|---|
| File tree | `src/widgets/file_tree.rs` | 1,774 | Portable — hand-rolled, lazy per-directory `read_dir`, no `ignore` use despite its own docs claiming otherwise |
| Search | `src/widgets/search.rs` + `file_finder.rs` | 2,201 + 1,060 | Portable — `ignore::WalkBuilder::build_parallel` + `grep_searcher`, worker thread with debounce and cancellation; the finder is fuzzy Cmd-P |
| Terminal | `src/widgets/terminal.rs` | 5,108 | **The one worth porting.** PTY + `alacritty_terminal` grid, reader thread, OSC sniffers, and correct `Drop` (child killed, reader thread joined) |
| Canvas | `src/widgets/editor.rs` | 13,937 | **Reference only.** It is a full editable editor — tabs, vim mode, LSP semantic tokens, ten viewer types. `sctxx` needs a read-only artifact viewer |

**All four widgets have zero `crate::app` back-references**, which is why porting is possible at all.
What comes along regardless is the chrome: `theme.rs` (1,181) — every pane carries a `Theme` — plus
icons, gradient, scrollbar, hover, prefs, workspace, and output, roughly **28–30k lines for bare
panes**. At full fidelity — LSP, viewers, vim, OSC rewinding, shell integration — it is **55–70k of
161k (35–45%)**. The genuinely un-draggable part is Croft's `App` routing: ~6k lines of
render/key/mouse dispatch that assumes Croft's global state and its `Pane`/`SidebarView` enums.

Two constraints this creates:

- **MSRV.** Croft is edition 2024 with a pinned toolchain of **1.97.1** and no `rust-version` field;
  `sctxx`'s MSRV is **1.88** once this block lands (FR-026a), so any ported code must compile at 1.88
  or be rewritten — a ported file that quietly raises it further breaks a CI job and a contract.
- **The licence facts are in ticket 12**, including the no-deferral rule (FR-028).

- **FR-026** **Reference only — no croft code is copied for the first slice.** The panes are written
  on lighter crates that do the same job, chosen with dependency counts off crates.io:
  `ratatui` 0.30 + `crossterm` 0.29 (rendering), `tui-tree-widget` 0.24 (file tree — 3 deps, instead of
  croft's 3,434 hand-rolled lines), `ignore` 0.4 (`crate::WalkBuilder`-style walking that respects
  `.gitignore`), `fuzzy-matcher` 0.3 (1 dep, the `SkimMatcherV2` agentman uses), `portable-pty` 0.9 +
  `vt100` 0.16 + `tui-term` 0.3 (terminal — vt100's 3 deps replace `alacritty_terminal`'s 17, because
  sctxx runs one agent in a pane and is not a terminal emulator), `tui-markdown` 0.3 (artifact view),
  `tui-input` 0.15 (fields). Find-in-files adds **no** dependency: the `ignore` walker plus the
  `regex` and `memchr` crates `sctxx` already has. Full table and reasoning:
  [`docs/adr/0003-tui-stack-and-msrv.md`](../../docs/adr/0003-tui-stack-and-msrv.md).
- **FR-026a** **MSRV moves 1.85 → 1.88**, forced by `ratatui` 0.30.1+, `ignore` 0.4.31+ and
  `tui-markdown`. It is recorded in `Cargo.toml` (`rust-version`), the CI MSRV job, the README, and the
  CHANGELOG. `rust-version` is per-package, so the `tui` feature cannot have its own — the bump is the
  price of one binary with a `--tui` flag, and it is paid once, deliberately.
- **FR-026b** The TUI ships behind a `tui` feature. `--no-default-features` (the `minimal` build) stays
  free of ratatui and friends, and the release-binary size is measured when the feature lands against
  §16's <15 MB target — not assumed.
- **FR-027** If a specific croft function is later judged worth copying, it is ported *then*, with the
  MIT copyright notice, a header naming the upstream path and version, and the change made — the same
  discipline `src/vendor/codex/` uses for Apache-2.0 code. Croft has no per-file headers and no NOTICE
  file (0 of 178 files), so those are ours to add.
- **FR-028** **Attribution is not deferred.** Stripping Croft branding is right — MIT grants no
  trademark rights — but MIT requires its copyright and permission notice be retained in copies and
  substantial portions of the software. Because FR-026 copies nothing, no notice is owed *today*; the
  machinery (`LICENSE-MIT`, a `NOTICE` entry, `src/vendor/croft/README.md`, and coverage by
  `scripts/check-vendor-headers.sh`) is specified so the first copied line is compliant on arrival
  rather than retrofitted.

**agentman** (`../handyutils/agentman`, the maintainer's own project) is already *"local-first TUI
session management for the coding agents you use every day"* — a ratatui TUI over the agent stores,
published to crates.io and npm. It is close enough to half of this block that the relationship must be
a decision, not an accident. **This block does not ship a second session scanner.**

A survey of it (2026-09-11) says what to take and what not to:

- **Take:** its scanning is what a TUI needs it to be — `ratatui` 0.30 + `crossterm` 0.29, fuzzy
  ranking with `fuzzy-matcher`'s `SkimMatcherV2`, and a `Session` model
  (`agent, id, title, project, path, modified, created, last_used, size_bytes, capabilities, diagnostic`)
  that is a superset of what `sctxx`'s `SessionSummary` carries. Its **fuzzy search** and **pane
  structure** are the reusable ideas, and `sctxx` has no fuzzy matcher today.
- **Do not take:** its discovery. It is one generic walker — recurse, accept any `*.json`/`*.jsonl`,
  then guess metadata by searching the first JSON object for keys named `sessionId`/`title`/`cwd`. That
  is structurally lossier than `sctxx`'s per-agent adapters and wrong in ways already observed:
  `.jsonl.zstd` is not matched, so it finds **zero** DSH sessions; OpenClaude's `<uuid>.replay.json`
  indexes as a duplicate session; Codewhale's nested `runtime/state.json` indexes as junk. It also
  never sets `created` despite a "Created" column, and never counts messages.
- **Useful side effect:** agentman knows four agents `sctxx` does not — OpenClaude, Codewhale, DSH,
  ACRYL — which is source material for `specs/022-m7-additional-adapters/`, not for this block.
- **Where it connects:** agentman's `launch_command` for its supported agents is the closest existing
  thing to FR-018, and it likewise has no way to seed a fresh session.

- **FR-029** Discovery comes from exactly one place: `sctxx`'s own `adapters::discovery`
  (`list`, `list_all`, `summarize`, `resolve`). The plan must either reuse it, extract the shared
  piece and consume it, or give a stated reason for duplicating — never fork it. If agentman and
  `sctxx` are to share discovery, that is a decision for the Wayfinder ticket below, because the two
  currently model the same domain differently and one of them is lossier.

## Non-goals

- Not an IDE. No LSP, DAP, debugger, collaboration, or editor feature comes with the borrowed panes.
- Not a session *writer*. Handoff launches a new session in the agent's own way; `sctxx` never
  injects into or edits an existing session store.
- No TUI-only capability: anything the TUI can do, the CLI can do.
- No second config system, no TUI-specific cache, no telemetry.

## Edge cases

- No sessions found at all; every store missing; a store present but empty.
- A session file being written *right now* by a live agent (read-only, may be truncated mid-line).
- A session whose `cwd` no longer exists — handoff must offer a usable directory rather than failing.
- Target agent installed but logged out, or installed at a version whose flags differ; the launch must
  fail with the agent's own message visible, not a panic.
- A handoff whose artifact already exists (overwrite? refuse? — decided in `clarify`).
- Terminal pane resized while a child is running; a child that ignores SIGTERM.
- Very long session ids and paths in a narrow terminal.

## Success criteria

- **SC-001** From `sctxx --tui`, a developer unfamiliar with the CLI reaches a launched handoff in
  under 30 seconds and without typing a path.
- **SC-002** Filtering 1,000+ sessions has no perceivable input lag; the first paint is <300 ms warm.
- **SC-003** `sctxx --tui` with piped stdout exits 2 with a clear message, and no existing CLI test
  changes behaviour.
- **SC-004** Every `extract` flag has a control in the form, enumerated by a test that compares the
  form against clap — so the two cannot drift.
- **SC-005** The panes borrowed from croft carry their attribution, and
  `scripts/check-vendor-headers.sh` fails if a derived file loses it.
- **SC-006** The TUI adds no path that executes transcript content; a test plants a command-like
  string in a session and asserts it never reaches a process argument.

## Dependencies

- **croft** (`../croft`) — MIT; panes 2–5.
- **agentman** (`../handyutils/agentman`) — the maintainer's own session-discovery TUI; the
  relationship is an open question below.
- The maintainer's agent launcher (`~/.local/bin/aiagents/launch-coding-agent`, and its detector at
  `~/.scripts/aiagents_scripts/detect-all-coding-ai-agents-on-this-machine-and-suggest-missing.py`) —
  reference for FR-017/FR-019, not a dependency.
- Existing `sctxx` surfaces reused as-is: `adapters::discovery`, `pipeline::{extract, write_all}`,
  `llm::Selection`, `pipeline::reconcile::is_git_ignored`, `skill`.

## Open questions → Wayfinder tickets before this block is specified

All four are resolved. The first three gate the plan and are closed; the block is specified from
`plan.md` onward with no open question.

1. **croft reuse boundary and MIT mechanics** — resolved 2026-09-11:
   [ADR 0003](../../docs/adr/0003-tui-stack-and-msrv.md). Reference only; the panes are built on
   lighter crates; the MIT machinery exists and is unused because nothing was copied.
2. **sctxx ↔ agentman** — resolved 2026-09-11:
   [ADR 0005](../../docs/adr/0005-sctxx-agentman-boundary.md). `sctxx` keeps
   `adapters::discovery` as the only scanner and as the definition of discovery semantics; no code is
   shared in either direction yet; the boundary is the versioned `list --json` / `show --json`
   contract; a shared crate is deferred with an explicit trigger (after block 022). The TUI continues,
   scoped to the handoff rather than to browsing.
3. **Launch and seeding per agent** — resolved 2026-09-11:
   [ADR 0004](../../docs/adr/0004-handoff-launch-and-seeding.md). A per-agent, version-pinned template
   table; the artifact travels as a path (Claude Code, Pi) or over stdin (headless Codex), never
   inline; every row falls back to the cwd route; the launch pre-checks the paths it names.
4. **TUI as a published feature** — resolved 2026-09-11: default-on behind a `tui` feature, excluded
   by `--no-default-features`, so the deterministic minimal build stays free of the viewport crates.
   Shipped that way in the first slice; the release-binary size is measured against §16's <15 MB target
   when the feature is complete (FR-026b).
