# Tasks: 024 - M8 Interactive TUI: find a session, extract it, hand it off

**Status**: Active · **Spec**: [`spec.md`](spec.md) · **Plan**: the spec's Scope/Success criteria, plus
ADR 0003 (stack and MSRV), ADR 0004 (launch and seeding), ADR 0005 (discovery boundary)
**Evidence**: [`evidence/`](evidence/)

> **The main line, stated once.** Grab any session from any agent → extract its context → wire that
> context into a new session of any coding agent. Everything in this block exists to serve that
> sentence, and anything that does not is secondary. The two keypresses are `h` then `enter`; the
> verification is T2419; the token cost of the *default* path is zero by construction, because the
> deterministic artifact is the product and the model is an option.

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

- [x] **T2406** [FR-011, SC-004, US5] The extraction form is clap's own definition of `extract`
  - Why: SC-004 requires the form and clap cannot drift, and the only way to guarantee that is to stop
    restating the CLI. The form reads `ExtractArgs::command()` and submits through
    `ExtractArgs::parse_argv()`, so clap decides which fields exist and which values are legal
  - Depends on: T2401
  - Touches: `src/tui/form.rs` (new), `src/cli/extract.rs` (`command`, `parse_argv`, `options`),
    `src/cli/mod.rs` (the module is crate-visible for this reason only)
  - RED/GREEN proof: `cargo test --all-features --lib tui::form` — 12 tests, including
    `every_flag_the_cli_has_is_a_field_and_nothing_else_is`
  - Acceptance: **stronger than specified.** The acceptance asked for the FR-011 flags plus a test that
    the two lists agree; instead the list is not written down twice at all. Presentation is derived
    too — clap's `SetTrue` makes a toggle and `get_possible_values` makes a picker, so there is no
    table mapping a field to a widget
  - Note: `--mode`, `--format`, `--redact` and `--progress` now carry `value_parser`, which makes clap
    their single source of truth **and** catches a bad `--format` before the extraction instead of
    after it. `the_only_argument_without_a_flag_is_the_session_reference` fails if a new positional is
    added, so one cannot silently vanish from the UI

- [x] **T2409** [FR-014, FR-015] Destination, written paths, and the CLI's own git warning
  - Why: the CLI's "this directory is not ignored by git" warning must appear in the pane too, and the
    surest way is for the pane to call the CLI's function rather than repeat its sentence
  - Depends on: T2405
  - Touches: `src/pipeline/mod.rs` (`Written`, `write_destination`), `src/cli/extract.rs`
    (`git_track_warning` now returns the message; `write_output` is four lines), `src/tui/{work,ui}.rs`
  - RED/GREEN proof: `cargo test --all-features --lib a_finished_run_says_where_the_artifact_went` and
    `cargo test --all-features --test cli extract_to_a_named_file_writes_only_that_file`
  - Acceptance: directory (five files), a single `.md`/`.json`, and any path are all reachable; the pane
    reports every written path and the handoff path; the git warning is **the CLI's function**, not its
    wording copied, which is stronger than the constant the acceptance asked for
  - Note: `write_destination` moved the directory-vs-file rule into the library so the CLI and the TUI
    cannot disagree about what a destination means, and a new CLI test covers the named-file branch
    that nothing had covered before. A relative destination resolves against the session's project

- [x] **T2411** [FR-017, US1] Detect the coding agents actually installed on this machine
  - Why: **new code with nothing to reuse.** agentman has no detector at all — only `dir.is_dir()` —
    and the launcher's catalogue has no versions. A handoff needs the binary, its version, and whether
    that version is one a seeding channel was verified on (ADR 0004, ADR 0005)
  - Depends on: nothing (the form and the browser do not need it)
  - Touches: `src/agents/mod.rs` (new), `src/lib.rs`, `src/llm/cli.rs` (the PATH lookup is now shared),
    `src/cli/doctor.rs`
  - RED/GREEN proof: `cargo test --all-features --lib agents::` — 11 tests
  - Acceptance: probes binaries **and** stores, honours `CLAUDE_CONFIG_DIR` / `CODEX_HOME`, reports the
    version it found, and distinguishes installed / installed-at-an-unverified-version / not installed;
    no network, no real `$HOME` in tests
  - Evidence: on this machine all three are detected at exactly the versions ADR 0004 was verified on
    (Claude Code 2.1.268, Codex CLI 0.153.4, Pi 0.85.1), each reported as `seeding verified on this
    version`. Two tests run real processes: one binary that answers `--version`, one that hangs and is
    abandoned after five seconds
  - Note: a store directory with no binary is **not** an install — `a_store_directory_with_no_binary_is_not_an_install`
    is named after the mistake agentman makes, and a binary with no readable version is installed but
    unverified rather than silently ready

- [x] **T2412** [FR-018, FR-019, ADR 0004] Seeding templates per agent, version-pinned, with a fallback
  - Why: the last mile, and the reason the block exists. ADR 0004 fixes the shape: the artifact travels
    as a **path**, never inline, and every row falls back to the cwd route
  - Depends on: T2411, T2409
  - Touches: `src/agents/seeding.rs` (new), `src/agents/mod.rs` (`is_known_agent`, the allowlist),
    `src/tui/{mod,ui,work}.rs`
  - RED/GREEN proof: `cargo test --all-features --lib agents::seeding` — 14 tests
  - Acceptance: Claude Code `--append-system-prompt-file <path>`, Pi `--append-system-prompt <path>`,
    Codex a positional pointer, each asserted argument by argument; each row carries the version an
    agent is compared against; an unverified version selects the cwd fallback and the pane **says so**;
    **the artifact's contents never cross argv** — asserted directly
  - Evidence: `running_a_launch_passes_the_argv_through_untouched` runs a real process and asserts the
    argv arrived as exactly three arguments with the pointer whole, which is only true because there is
    no shell in the path
  - Note: ADR 0004 also specifies the headless form (`codex exec -`, artifact on stdin). It is not
    implemented: nothing consumes it yet, and the terminal handover is the interactive path. Recorded
    rather than written speculatively

- [x] **T2413** [FR-020, FR-021, SC-006] Launch safety: no shell, pre-checked paths, allowlist, cwd
  - Why: transcript content is data and must never become part of a command line, and Claude Code fails
    **lazily and silently** on an unreadable file flag — so sctxx validates what the agent will not
  - Depends on: T2412
  - Touches: `src/agents/seeding.rs`, `src/tui/mod.rs` (the two-step confirm, `Step::HandOver`)
  - RED/GREEN proof: `cargo test --all-features --lib no_text_from_a_session_reaches_a_process_argument`
    (SC-006's planted string) and the four launch-refusal tests
  - Acceptance: the child is spawned with an argument vector — no shell anywhere; a missing, unreadable,
    or non-file handoff fails **before** anything is spawned, with the reason in the pane; the allowlist
    is structural (`is_known_agent`, with a test that `rm` cannot be launched); the exact command is on
    screen before the terminal is handed over; cwd is the session's directory when it still exists and a
    usable directory when it does not
  - Evidence: `a_handoff_that_vanished_is_caught_before_the_terminal_is_handed_over`; the whole flow is
    `tui::tests::the_handoff_is_two_steps_and_the_first_one_runs_nothing`
  - Note: choosing and confirming are separate steps because FR-021b requires that extraction never
    launches anything by itself
  - **Amended 2026-09-11:** the handoff extracts for itself. Requiring a prior extraction was the
    block's own purpose pushed behind its own machinery. The confirmation now names the destination in
    full (absolute, not `.sctxx/`), states that the extraction is deterministic and costs no tokens,
    and shows the exact command — worked out with `Launch::plan`, which plans without requiring the
    artifact to exist, while `Launch::run` still checks it immediately before spawning (ADR 0004)

## Open

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

- [ ] **T2407** [FR-012, US5] Backend availability in the form, probed the way `doctor` probes it
  - Why: the choice should be informed rather than guessed, and `--llm none` must always be offered
    because it always works
  - Depends on: T2406
  - Touches: `src/tui/form.rs`, reusing `llm::Selection` and doctor's probe
  - RED/GREEN proof: `cargo test --all-features --lib unavailable_backends`
  - Acceptance: an unavailable backend is listed as unavailable **with its reason**, never hidden;
    `none` is always present; the same probe function serves `doctor` and the pane, so the two answers
    cannot disagree
  - Note: T2406 left `--llm` a text field, which is honest but unhelpful — the value space is dynamic
    (`cli:<agent>`, `api:<provider>/<model>`), so clap cannot enumerate it and nothing yet shows what
    this machine actually has. That is this task

- [ ] **T2408** [FR-013] Cancel a running extraction, and let a long one be interrupted
  - **Half done.** The run itself and its progress landed with T2406: `enter` runs the pipeline on the
    worker, every stage the pipeline reports reaches the pane, the last few are kept as a short log, a
    pipeline error is shown as a message rather than a panic, and `running_the_form_writes_a_handoff`
    exercises the whole path offline against a fixture.
  - **Not done: cancellation.** Once the pipeline is running it cannot be interrupted; the pane says
    nothing it cannot do, and the form is left intact so the run can be repeated. Cancelling means
    threading a flag through `pipeline::extract`, which is the real content of this task
  - Why: a `--llm cli:claude` extraction can take minutes, and a TUI that cannot be stopped is worse
    than the CLI it replaces
  - Depends on: T2406 (done)
  - Touches: `src/pipeline/mod.rs` (a cancellation check inside the stage loop), `src/tui/work.rs`
  - RED/GREEN proof: `cargo test --all-features --lib cancelling_leaves_nothing_behind`
  - Acceptance: a running extraction can be stopped from the pane; no partial artifact is left behind;
    the process stays clean; the existing progress tests keep passing

- [x] **T2410** [FR-016, FR-016a, FR-023] Canvas: read the artifact in place, L0–L3, `expand`
  - Why: FR-016's premise is that a developer who has to leave the TUI to read the artifact will not
    read it — so the artifact must open itself, and the pane must be a viewer rather than a receipt
  - Depends on: T2409
  - Touches: `src/tui/canvas.rs` (new), `src/pipeline/artifact.rs` (new), `src/cli/discover.rs`
    (`expand` now calls the shared function), `src/tui/{mod,ui,work}.rs`
  - RED/GREEN proof: `cargo test --all-features --lib tui::canvas` (12) and the render tests in
    `tui::ui`; the end-to-end `running_the_form_writes_a_handoff` now also asserts the canvas opened
  - Acceptance: on success the pane opens at L0 **with no further keypress** (asserted on a real
    extraction, not a mock); L0–L3 step by number and by tab; `[evt a–b]` pointers resolve through
    **the same function the CLI uses** — `pipeline::artifact::expand_ranges`, which `sctxx expand` now
    calls too, so there is no second implementation to drift; the ledgers are visible alongside, which
    they are because the renderer emits them in L1 and the canvas reads the artifact rather than
    re-rendering it; an existing artifact opens by path (FR-016a), including a colleague's
  - Evidence: the real 58 KB artifact carries `## L0 · Brief`, `## L1 · Items`,
    `## L2 · Recent activity (masked, evt 1058–1961)` and `## L3 · Retrieval`, with 14 pointers — the
    format the canvas parses
  - **Bug found by writing the tests:** with the cursor pinned to the top visible line, a pointer on
    the second line of a *short* artifact was unreachable, because there was nothing to scroll. The
    canvas now has a cursor the window follows, with
    `every_line_is_reachable_even_in_a_document_shorter_than_the_window` as the regression test
  - Note: reading takes the whole body. Prose in half a terminal is not reading, and the list is not
    needed while the artifact is open

- [ ] **T2403** [FR-003] The pane rail — **deferred until there are panes to switch between**
  - A rail exists to move between panes. With only SCTXX and the canvas, both of which are reached by
    a key and both of which want the whole body, a rail would be decoration. It lands with the Files
    pane (T2416) and Search (T2417), which are the first two that genuinely sit side by side. The
    original T2403 entry is kept below for its acceptance criteria.
  - Original acceptance: five rail entries in the spec's order, SCTXX selected on start, key and click
    both switch, `q` quits from any pane, `?` overlays the keymap; an unavailable pane says why rather
    than disappearing

- [ ] **T2414** [FR-021a, FR-021b] Re-redact before egress, and keep launch a separate confirmation
  - **Half done.** The separate confirmation landed with T2413: choosing an agent and confirming the
    command are two steps, and an extraction on its own launches nothing. **Re-redaction at egress did
    not**, and is the whole of what remains
  - Why: egress to a different tool is the one boundary where redundancy is cheap and a mistake is
    unrecoverable; and producing an artifact must never start another agent as a side effect
  - Depends on: T2413
  - Touches: `src/agents/launch.rs`, `src/redact` reuse
  - RED/GREEN proof: `cargo test --all-features --lib re_redacted_at_egress`
  - Acceptance: a second pass runs over the artifact text immediately before it is handed over; the
    pane reports the count and classes removed, never the secrets; `--redact strict` applies to the
    handoff only and never to the artifact on disk; stopping at the file is a complete, ordinary flow
  - Note: third and fourth redaction of the same text on the way out. That is the point.

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

- [x] **T2419** [ADR 0004, FR-019] Verify each seeding row against a real launch
  - Why: ADR 0004 recorded its rows as *probed* — the flags existed, but no artifact had ever reached a
    model through them. A claim about another tool is worth exactly as much as the last time it was run
  - Depends on: T2412, T2413
  - RED/GREEN proof: n/a — this is an evidence task, and its output is
    [`evidence/T2419.md`](evidence/T2419.md)
  - Acceptance: for each of Claude Code, Codex and Pi, on the installed versions, the artifact reached
    the receiving agent's first turn and the version and exact command are recorded
  - **Result: three of three verified for content delivery**, against a real 58 KB artifact. The test is
    content-level, not flag-level: the artifact contains the source session's id and the pointer does
    not, so an agent that answers with that id read the *handoff*. Claude Code 12.0 s, Pi 15.0 s, Codex
    20.5 s, all exit 0
  - **Finding:** neither interactive launch reached a first turn — both stopped at the agent's own
    **trust prompt** for an unfamiliar directory. Correct behaviour, and precisely what
    `--dangerously-bypass-…` exists to skip and sctxx never passes. The confirming pane now says so, so
    it is not a surprise. `the_confirmation_shows_the_exact_command_before_it_runs` asserts the wording
  - **Still unverified, and said so rather than implied:** first-turn delivery in the interactive form
    past that prompt, and resumability. Reaching either would mean accepting trust on the developer's
    behalf or launching into a live project
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

## From review feedback 1 (2026-09-11)

The provenance half of [`REVIEW_FEEDBACK_1.md`](../../docs/REVIEW_FEEDBACK_1.md) is answered by
[`CODEX-PROVENANCE-AUDIT.md`](../../docs/CODEX-PROVENANCE-AUDIT.md) and ticket 15. These are its
engineering asks, which are about the artifact rather than about provenance.

- [ ] **T2422** [FR-016] End-state reconciliation is authoritative
  - Why: `CurrentStep` and `NextAction` must be produced only *after* the recency tail and the
    last-known command and git state are consumed, and an action followed by success must resolve
    automatically. The review calls this the single rule that would have prevented the largest error
    in the artifact it examined
  - Depends on: T2410
  - Acceptance: a next-action whose command later succeeded is not rendered as pending; the final pass
    is the only writer of current-step state

- [ ] **T2423** [FR-009] Cross-check L0 against L2 mechanically
  - Why: before rendering, ask whether the tail contains evidence later than a proposed next action
    that indicates success, replacement, or a changed direction. The review notes this needs no model
    call
  - Depends on: T2422
  - Acceptance: a contradiction between L0 and L2 is either resolved or reported in the artifact; on
    the session that prompted the review, the check fires

- [ ] **T2424** [FR-009] Separate the historical ledger from the active workset in L1
  - Why: "548 more files touched" and dozens of historical errors are forensics, not the first thing an
    agent needs. The review's most concrete product suggestion
  - Acceptance: L1 leads with an **Active Workset** — relevant spec, latest touched files, current
    dirty files, latest relevant test, latest commits, currently unresolved errors — with the historical
    ledger kept and retrievable
  - Note: `/Users/musichen/.claude/projects/` — the artifact this was written from reports 18 uncommitted
    changes and 86 missing files, neither of which appears in L0 or L1 today

- [ ] **T2425** [FR-009] Goal evolves; stale counts and provider summaries are warnings, not metadata
  - Why: three separate asks from the review that share a cause — the header knows things the brief does
    not say. L0's goal on a ten-day session is provenance, not the active objective; a `stale: 100`
    header line is a first-class warning; and Claude's own compaction summary is often the highest-value
    semantic object in a long session, currently buried in L2
  - Acceptance: `Original goal` is kept as provenance and the active goal comes from folded state; a
    large stale/contradicted count is stated in L0; each provider compaction summary is surfaced in L0
    as a **low-trust** seed with its event pointer, and corroborated against deterministic evidence

## Out of scope for this block

- **T2415 (an embedded terminal pane) — superseded by ADR 0006.** The launched agent is itself a
  full-screen application, so it gets the whole terminal rather than a rectangle inside another TUI:
  sctxx restores the terminal, runs the child with inherited stdio, and re-initialises when it exits.
  This removed `portable-pty`, `vt100` and `tui-term` from the dependency list. The child is still
  owned — spawned, waited for, its exit status reported — which is what FR-022 was protecting. An
  embedded pane is not foreclosed; it is a new decision with its own evidence.
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
