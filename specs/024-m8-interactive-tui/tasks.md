# Tasks: 024 - M8 Interactive TUI: find a session, extract it, hand it off

**Status**: Active · **Spec**: [`spec.md`](spec.md) · **Plan**: the spec's Scope/Success criteria, plus
ADR 0003 (stack and MSRV), ADR 0004 (launch and seeding), ADR 0005 (discovery boundary)
**Evidence**: [`evidence/`](evidence/)

The ID space here is authoritative. Tasks are checked only after a RED/GREEN loop, the gate, and (where
the task produces a finding rather than code) an evidence file.

Conventions inherited from the block: the TUI is a **view** over the existing pipeline (FR-025). No task
may add a second discovery path, a second config system, a second artifact format, or a TUI-only
capability. Every task below reuses the library function the CLI already calls; where it cannot, the
task says which library function is being wrapped, not reimplemented.

## Done

- [x] **T2401** [US2] Session browser: list, fuzzy search, filters, preview
  - Why: the entry point of the block, and the only slice that needs nothing from the tickets
  - Depends on: nothing
  - Touches: `src/tui/{mod,browser,ui}.rs`, `src/cli/mod.rs` (`--tui`), `src/lib.rs`, `Cargo.toml`
    (feature `tui`), `CHANGELOG.md`
  - RED/GREEN proof: `cargo test --all-features --lib tui` (16 tests) and
    `cargo test --all-features --test cli tui_`
  - Acceptance: 643 real sessions listed with claude and codex rows; `/` searches fuzzily over title,
    first message, id and directory; a pasted id or path matches exactly and outranks every fuzzy hit;
    `a`/`r`/`p` filter by agent, recency and "this project"; re-filtering rebuilds from the full
    discovery snapshot so toggling a filter off restores exactly what was there; the preview renders
    from discovery alone and never opens the transcript, so browsing costs nothing
  - Covers FR-005, FR-006, FR-007, FR-010 and the first half of FR-009
  - Evidence: frame captured through a PTY at 130x40; commit `db8e53f`

- [x] **T2402** [FR-002/FR-003] `--tui` is a top-level flag and refuses a non-terminal stdout
  - Why: an agent or a pipeline that pipes output must keep getting CLI behaviour, never a TUI waiting
    on a key press — and `sctxx extract --tui` must be a usage error, not a silently ignored flag
  - Depends on: T2401
  - Touches: `src/cli/mod.rs` (`command: Option<Command>`, `interactive`), `tests/cli.rs`
  - RED/GREEN proof: `cargo test --all-features --test cli tui_refuses_to_run_without_a_terminal` and
    `tui_is_a_top_level_flag_only`; both are feature-aware so the minimal build asserts the refusal
    instead
  - Acceptance: piped stdout exits 2 naming the CLI alternatives; `--tui` with a subcommand is a usage
    error; a build without the `tui` feature still accepts the flag and fails with a sentence, matching
    how `--llm api:` behaves without `api`
  - Covers FR-001, FR-002, and the parts of FR-003 and FR-004 that need no rail

- [x] **T2405** [FR-009, FR-010, US4] The preview shows the session's contents, read off the UI thread
  - Why: T2401's preview was discovery metadata only, and FR-009 asks what is *in* the session. The
    obvious implementation — parse on selection — freezes the UI for seconds per cursor move, so the
    transcript is read on a worker and the pane says so meanwhile
  - Depends on: T2401
  - Touches: `src/tui/preview.rs` (new), `src/tui/work.rs` (new), `src/tui/mod.rs` (settle delay and
    bounded cache), `src/tui/ui.rs` (the contents section), `src/ir.rs` (`Diagnostic::label`)
  - RED/GREEN proof: `cargo test --all-features --lib tui` — 33 tests, the lib suite at 238
  - Acceptance: user turns on the active branch, live/total events, files touched busiest-first, the
    last command with its status, unresolved error signatures, provider compactions **split into
    re-anchors and history discards**, and the diagnostic count are all shown; a read that fails shows
    its reason and the session stays selectable (FR-010); **no LLM backend is reachable from this
    path** — it is the deterministic ledgers, and the module does not reference `llm`
  - Evidence: renders against the real stores through a PTY at 150x44 on both debug and release (the
    pane showed `12 user turns on the active branch`, `6753 live of 14151`); five `TestBackend` render
    tests assert the frame's *contents*, which is what catches clipping
  - Note: three rules make it usable and each has a test — a 150 ms settle delay because `j` is a held
    key, a generation counter so a superseded read is dropped **before it starts** rather than after,
    and a 256-entry cache so a resting cursor does not re-read
  - Covers FR-009, FR-010, and the transferable half of FR-013's progress signalling

## Open

- [ ] **T2403** [FR-003] The pane rail: SCTXX first, switch by key and by click, `?` keymap
  - Why: every later pane task needs a place to live; the rail is the block's frame and is currently
    missing, which is why the TUI shows one pane and no navigation
  - Depends on: T2401
  - Touches: `src/tui/{mod,ui}.rs`, `src/tui/panes.rs` (new)
  - RED/GREEN proof: `cargo test --all-features --lib panes`
  - Acceptance: five rail entries in the spec's order, SCTXX selected on start, key and click both
    switch, `q` quits from any pane, `?` overlays the keymap; an unavailable pane says why rather than
    disappearing
  - Reference: `spec.md` "The screen" table. No croft code (FR-026)

- [ ] **T2404** [FR-008, SC-002] Progressive discovery: paint before the list is complete
  - Why: **measured, not assumed.** A warm `list_all` over the 643 real sessions on this machine takes
    **0.75 s**, against SC-002's <300 ms first paint, and it runs to completion before the terminal is
    even initialised. The background-work mechanism this needs already landed with T2405; what remains
    is making the *producer* stream rather than return a `Vec`
  - Depends on: T2405
  - Touches: `src/adapters/discovery.rs` (an additive callback/iterator variant beside `list_all`, so
    the CLI keeps its signature), `src/tui/mod.rs`
  - RED/GREEN proof: `cargo test --all-features --lib progressive`
  - Acceptance: first paint before every session is summarised; keys stay responsive while discovery
    runs; a session arriving mid-filter is filtered, not appended blindly; the <300 ms warm first paint
    is measured and recorded, or the claim is dropped
  - Note: additive only. `list`/`list_all`/`summarize`/`resolve` stay the definition of discovery
    (FR-029, ADR 0005) and the CLI must not change behaviour

- [ ] **T2406** [FR-011, SC-004, US5] Extraction form mirroring every `extract` flag
  - Why: SC-004 requires the form and clap cannot drift. The only way to guarantee that is to derive the
    form from the clap definition rather than restating it
  - Depends on: T2403
  - Touches: `src/tui/form.rs` (new), `src/cli/mod.rs` (expose the `extract` command for enumeration)
  - RED/GREEN proof: `cargo test --all-features --lib the_form_matches_clap`
  - Acceptance: `--mode`, `--llm`, `--budget`, `--tail`, `--focus`, `--since-compact`,
    `--max-bad-lines`, `--redact`, `--include-sidechains`, `--no-verify`, `--layers` all present with
    today's defaults; the test fails if a flag is added to clap and not to the form, and if the form
    grows a control the CLI does not have
  - Note: this is the task that makes "no TUI-only capability" (FR-025) checkable rather than aspirational

- [ ] **T2407** [FR-012, US5] Backend availability in the form, probed the way `doctor` probes it
  - Why: the choice should be informed rather than guessed, and `--llm none` must always be offered
    because it always works
  - Depends on: T2406
  - Touches: `src/tui/form.rs`, reusing `llm::Selection` and doctor's probe
  - RED/GREEN proof: `cargo test --all-features --lib unavailable_backends`
  - Acceptance: an unavailable backend is listed as unavailable **with its reason**, never hidden;
    `none` is always present; the same probe function serves `doctor` and the pane, so the two answers
    cannot disagree

- [ ] **T2408** [FR-013] Run the extraction with streaming progress and working cancellation
  - Why: extraction is the block's first long-running action, and a TUI that blocks during it is worse
    than the CLI it replaces
  - Depends on: T2407
  - Touches: `src/tui/run.rs` (new), reusing `pipeline::extract` and its stage progress
  - RED/GREEN proof: `cargo test --all-features --lib progress_stages`
  - Acceptance: `parse`, `ledgers`, `segment`, `fold`, `verify`, `since-compact` progress reaches the
    pane; cancellation leaves no temp state behind and the process exits cleanly; a pipeline error is
    shown in the pane with its exit code, not a panic

- [ ] **T2409** [FR-014, FR-015] Destination selection, written paths, and the git-ignore warning
  - Why: the CLI's non-ignored-output-directory warning must appear here too — the same rule and the
    same wording, not a second implementation of it
  - Depends on: T2408
  - Touches: `src/tui/run.rs`, reusing `pipeline::reconcile::is_git_ignored` and the CLI's warning text
  - RED/GREEN proof: `cargo test --all-features --lib reuse_the_cli_warning`
  - Acceptance: directory (five files), single `.md`/`.json`, and "handoff" all reachable; the pane
    reports every written path; the warning is the CLI's string, asserted by comparing against the
    constant the CLI uses

- [ ] **T2410** [FR-016, FR-016a, FR-023] Canvas pane: read the artifact in place, L0–L3, `expand`
  - Why: FR-016's premise is that a developer who has to leave the TUI to read the artifact will not
    read it — so the artifact must open itself the moment extraction succeeds
  - Depends on: T2409
  - Touches: `src/tui/canvas.rs` (new), reusing the layer renderer and `expand`'s pointer resolution
  - RED/GREEN proof: `cargo test --all-features --lib the_artifact_is_readable_at_l0`
  - Acceptance: on success the pane opens at L0 with no further keypress; L0–L3 step; `[evt a–b]`
    pointers resolve through the same `expand` path the CLI uses and never re-implement it; ledgers are
    visible alongside; an existing artifact on disk opens by path (FR-016a)
  - Note: read-only. Croft's editor is 24,751 lines precisely because it is not (FR-026)

- [ ] **T2411** [FR-017, US1] Detect the coding agents actually installed on this machine
  - Why: **new code with nothing to reuse.** agentman has no detector at all — only
    `dir.is_dir()` — and the launcher's detector reads a ~30-agent catalogue but no versions
    (ADR 0005)
  - Depends on: T2403
  - Touches: `src/agents/mod.rs` (new), `src/tui/agents.rs` (new)
  - RED/GREEN proof: `cargo test --all-features --lib detects_the_installed_agents` (temp-dir `PATH`)
  - Acceptance: probes binaries (`which`-equivalent) **and** stores, honours `CODEX_HOME` /
    `CLAUDE_CONFIG_DIR`, reports the version it found, and distinguishes "installed", "installed but
    the version is not one we have verified", and "not installed"; no network, and no dependence on a
    real `$HOME`
  - Reference: the launcher's catalogue for *which* agents to probe, not for how

- [ ] **T2412** [FR-018, FR-019, ADR 0004] Seeding templates per agent, version-pinned, with a fallback
  - Why: the last mile, and the reason the block exists. ADR 0004 fixes the shape: the artifact travels
    as a **path** or over **stdin**, never inline; every row falls back to the cwd route
  - Depends on: T2411, T2410
  - Touches: `src/agents/seeding.rs` (new), `src/cli/config` (templates, per §9.3's discipline)
  - RED/GREEN proof: `cargo test --all-features --lib seeding_templates`
  - Acceptance: Claude Code `--append-system-prompt-file <path>`, Pi `--append-system-prompt <path>`,
    Codex interactive positional pointer, headless `codex exec -` over stdin; each row records the
    version it was checked against; an unverified version selects the cwd fallback and **says so**;
    the generated argv is asserted per row, so a change to it fails a test rather than a launch

- [ ] **T2413** [FR-020, FR-021, SC-006] Launch safety: no shell, pre-checked paths, allowlist, cwd
  - Why: transcript content is data and must never become part of a command line (constitution I,
    `AGENTS.md` rule 5). Claude Code also fails **lazily and silently** on an unreadable file flag, so
    sctxx must validate what the agent will not (ADR 0004)
  - Depends on: T2412
  - Touches: `src/agents/launch.rs` (new), `tests/cli.rs`
  - RED/GREEN proof: `cargo test --all-features --lib a_transcript_command_never_becomes_an_argument`
    (SC-006's planted command-like string) and `the_launch_pre_checks_every_path_it_names`
  - Acceptance: the child is spawned with an argument vector — no shell, ever; a missing or unreadable
    named path fails before spawning, with the reason in the pane; only allowlisted agents launch; the
    exact command is shown before it runs; cwd is the session's cwd when it still exists, and a usable
    directory is offered when it does not
  - Evidence: SC-006's planted-string test is the block's proof that the TUI adds no execution path

- [ ] **T2414** [FR-021a, FR-021b] Re-redact before egress, and keep launch a separate confirmation
  - Why: egress to a different tool is the one boundary where redundancy is cheap and a mistake is
    unrecoverable; and producing an artifact must never start another agent as a side effect
  - Depends on: T2413
  - Touches: `src/agents/launch.rs`, `src/redact` reuse
  - RED/GREEN proof: `cargo test --all-features --lib re_redacted_at_egress`
  - Acceptance: a second pass runs over the artifact text immediately before it is handed over; the
    pane reports the count and classes removed, never the secrets; `--redact strict` applies to the
    handoff only and never to the artifact on disk; stopping at the file is a complete, ordinary flow
  - Note: third and fourth redaction of the same text on the way out. That is the point.

- [ ] **T2415** [FR-022, US1] Terminal pane: a PTY the pane owns and outlives nothing
  - Why: US1 ends with a running agent, and the launch is only honest if the child dies with the pane
  - Depends on: T2413
  - Touches: `src/tui/terminal.rs` (new); `portable-pty` + `vt100` + `tui-term`
  - RED/GREEN proof: `cargo test --all-features --lib the_child_dies_with_the_pane`
  - Acceptance: resize reaches the child; the child is killed and the reader thread joined on drop, on
    quit, and on cancellation; a crashed child degrades to a message in the pane rather than taking the
    TUI down; a child that ignores SIGTERM is escalated, not leaked
  - Reference: croft's `Drop` discipline is the idea worth having (ADR 0003); the code is not copied

- [ ] **T2416** [FR-003] Files pane: file tree of the session's cwd
  - Why: makes the TUI a place to work rather than a dialog; deliberately the cheapest of the borrowed
    panes
  - Depends on: T2403
  - Touches: `src/tui/files.rs` (new); `tui-tree-widget` (3 deps) + `ignore`
  - RED/GREEN proof: `cargo test --all-features --lib the_tree_respects_gitignore`
  - Acceptance: lazy per-directory expansion; `.gitignore` respected via `ignore`; a cwd that no longer
    exists says so instead of showing an empty tree (spec Edge cases)

- [ ] **T2417** [FR-003] Search pane: find in files
  - Why: the last borrowed pane, and the one that costs **no new dependency** — `ignore`'s walker plus
    the `regex` and `memchr` sctxx already has (FR-026)
  - Depends on: T2416
  - Touches: `src/tui/search.rs` (new)
  - RED/GREEN proof: `cargo test --all-features --lib search_streams_and_cancels`
  - Acceptance: results stream while the walk runs; typing re-targets the search rather than queueing
    one per keystroke (debounce); cancelling stops the walk; the flat `SearchHit { path, line_no,
    line_text }` shape the CLI-adjacent model already implies

- [ ] **T2418** [FR-023, FR-024, FR-025] The remaining subcommands, reachable and un-duplicated
  - Why: "nothing is TUI-only; nothing in the CLI is missing from the TUI" is only true if the last
    few entry points exist, and the pane must not grow state the CLI does not have
  - Depends on: T2410
  - Touches: `src/tui/panes.rs`, `src/tui/pager.rs` (new)
  - RED/GREEN proof: `cargo test --all-features --test cli every_subcommand_is_reachable_from_the_tui`
  - Acceptance: `show`/`expand` land in Canvas, `doctor`/`schema` in a pager, `skill install` as an
    action, `list`/`find`/`verify`/`redact` reachable; **no TUI-specific config file** — settings come
    from the same config the CLI reads; the test enumerates clap's subcommands so a new one cannot be
    silently absent

- [ ] **T2419** [ADR 0004, FR-019] Verify each seeding row end to end, against a real launch
  - Why: ADR 0004 records what was *probed* — flag existence, usage strings, Pi's file-vs-text rule —
    and is explicit that a real launch is unverified. Starting sessions and spending tokens is why this
    is its own task, and why each row's status is honest until it runs
  - Depends on: T2415, and a real artifact from T2410
  - Touches: `docs/adr/0004-handoff-launch-and-seeding.md` (status column only), `evidence/T2419.md`
  - Acceptance: Claude Code 2.1.268, Codex 0.153.4 and Pi 0.85.1 each start a new session whose first
    turn contains the handoff; the version, the exact argv, whether the session is left resumable, and
    the fallback are recorded per row; a row that fails is demoted to the cwd fallback in the table
    rather than left claiming otherwise
  - Blocked on: nothing but consent — this task starts real agent sessions

- [ ] **T2420** [FR-026b, §16] Measure the release binary against the <15 MB target
  - Why: the spec requires the size be measured when the feature lands, "not assumed"; the TUI adds
    ratatui, crossterm, portable-pty, vt100, tui-term, tui-tree-widget, tui-markdown, tui-input
  - Depends on: T2417
  - Touches: `docs/SCTXX-SPEC.md` §16 (result), `CHANGELOG.md`
  - Acceptance: release binary size for each shipped target recorded, with the `tui` and no-`tui`
    difference; if the target is exceeded, the fact is recorded and the block proposes a trim rather
    than quietly missing it

- [ ] **T2421** [FR-025] `--tui` reaches the docs and the Agent Skill
  - Why: the feature does not exist for a user who cannot find it, and the skill is how a receiving
    agent learns the tool has a TUI
  - Depends on: T2418
  - Touches: `docs/SCTXX-SPEC.md` §3, `README.md`, `skill/SKILL.md`, `skill/references/cli.md`,
    `docs/index.html`
  - RED/GREEN proof: `cargo xtask gen-skill` produces no diff after running, and
    `cargo test --all-features --test cli help_lists_tui`
  - Acceptance: `--tui` documented with its TTY requirement and its feature gate; the keymap in the docs
    matches `?`; the website gains the pane; `gen-skill` is re-run rather than hand-edited

## Out of scope for this block

- Croft code. No line is copied, so no MIT notice is owed (FR-026, FR-028). If a later slice copies
  one, the machinery — `LICENSE-MIT`, `NOTICE`, `src/vendor/croft/README.md`, the header check — is
  specified and waiting (ADR 0003).
- LSP, DAP, debugger, collaboration, editor, or vim mode. Not an IDE.
- A second session scanner. `adapters::discovery` is the only one, and agentman's walker is not
  adopted (FR-029, ADR 0005).
- Sharing discovery with agentman. Deferred with a trigger: after
  `specs/022-m7-additional-adapters/` lands and the adapter set stops moving (ADR 0005).
- Session resume, forking, or writing into any agent's session store. agentman owns resume; sctxx
  seeds a *new* session (ADR 0005).
- The four agents agentman knows and sctxx does not (OpenClaude, Codewhale, DSH, ACRYL) — that is
  `specs/022-m7-additional-adapters/`.

## Risks carried by this block

- **Unverified launch rows (T2419).** Every seeding row degrades to the cwd route, which asks nothing
  of the agent, so an undetected flag change costs a fallback rather than the feature.
- **Undocumented Claude Code flag.** `--append-system-prompt-file` is not in `--help`. ADR 0004 makes
  it an optimisation over the fallback rather than load-bearing, so its removal is survivable.
- **Binary size (T2420).** Eight new crates behind one feature; measured, not assumed.
- **Scope.** M8 is the largest block in the repo and its second half is genuinely large — five panes
  plus a launch layer. T2403 gates every pane task, so the block can stop after T2410 with a coherent
  product: find, extract, read. T2411–T2415 are the handoff, T2416–T2418 are the borrowed surface.
