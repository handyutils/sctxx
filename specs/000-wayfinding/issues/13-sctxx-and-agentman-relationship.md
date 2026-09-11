# Decide how sctxx and agentman relate before the TUI duplicates session discovery

Type: research
Status: open

## Question

`sctxx --tui` (block `specs/024-m8-interactive-tui/`) browses sessions in a ratatui TUI.
[agentman](https://github.com/handyutils/agentman) — the maintainer's own project, at
`../handyutils/agentman` — **is already a ratatui TUI that browses sessions**, published to crates.io
and npm. Two tools, one domain, one owner: which one owns discovery, and what does the other do with it?

## What a survey of agentman found (2026-09-11)

- Stack: Rust edition 2024, MSRV 1.88, `ratatui` 0.30 + `crossterm` 0.29, `fuzzy-matcher`
  (`SkimMatcherV2`). Files: `src/{main,lib,app,model,adapters,ui}.rs`, npm wrapper `npm/bin/agentman.js`.
- **It has no installed-agent detection at all.** No binary probing, no `PATH` lookup, no version
  reads. Its only test is `home.join(relative).is_dir()` against a hardcoded `ROOTS` table of eight
  store paths (`src/adapters.rs:141-166`). It does not honour `CODEX_HOME`/`CLAUDE_CONFIG_DIR`.
- **Its discovery is one generic walker, not per-agent adapters** (`src/adapters.rs:168-224`): recurse
  every subdirectory, accept any `*.json`/`*.jsonl`, then guess metadata by searching the first
  JSON-parsing line for keys named `sessionId`/`session_id`/`id`, `title`/`summary`/`name`,
  `cwd`/`project`/`working_directory`. Fallback id is the file stem.
- **Known losses from that design:** `.jsonl.zstd` is not matched, so it finds **zero** DSH sessions
  while its README advertises DSH resume; OpenClaude's `<uuid>.replay.json` indexes as a duplicate
  session; Codewhale's nested `<uuid>/runtime/state.json` indexes as junk; `created` is never set
  although the UI has a "Created" column; message counts do not exist.
- It filters by agent (number keys) and fuzzy-searches `title + id + project`. **No date filter, no cwd
  filter** — both of which block 024 requires.
- Its `Session` model is a superset of `sctxx`'s `SessionSummary`:
  `agent, id, title, project, path, modified, created, last_used, size_bytes, capabilities, diagnostic`.
- It carries four agents `sctxx` lacks — OpenClaude, Codewhale, DSH, ACRYL — which is input to
  `specs/022-m7-additional-adapters/`, not to the TUI.

## Also in the picture

The maintainer's launcher (`~/.local/bin/aiagents/launch-coding-agent`) and its detector
(`~/.scripts/aiagents_scripts/detect-all-coding-ai-agents-on-this-machine-and-suggest-missing.py`, a
~30-agent pipe-delimited catalogue probed as `cmd:`/`path:`/`app:`/`glob:`, no versions) are a third
implementation of "find the agents on this machine".

## Decide

1. **Who owns discovery.** Options: (a) `sctxx` keeps `adapters::discovery` and agentman consumes it
   as a library; (b) a small shared crate owns it and both consume that; (c) agentman gains the TUI
   handoff feature and `sctxx --tui` never ships; (d) deliberate duplication, with the reason written
   down. Note that (a) or (b) require agentman to *lose* its lossier walker, which is a change to
   another published tool.
2. **Who owns launching.** agentman already has a `launch_command` per agent; block 024 needs one
   too, plus seeding. Duplicating it twice over is the outcome to avoid.
3. **What `sctxx --tui` is for if agentman exists.** The honest framing is that block 024's novelty is
   not browsing — it is *extract → hand off with context pre-loaded*. If that is the case, the
   decision may be that the TUI is thin and the handoff is the product.

## Evidence to produce

A one-page recommendation with the boundary named (which repository owns which function), the
migration cost for whichever tool loses code, and a note for `docs/SCTXX-ROADMAP.md` if the answer
changes M8's scope. Unblocks `specs/024-m8-interactive-tui/` FR-029. Spec ref: block 024, "Reuse"
section.
