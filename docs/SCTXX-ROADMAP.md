# SCTXX Roadmap

## Product vision

sctxx turns any finished or idle coding-agent session into a warm start for any other coding agent,
from any provider. A developer (or an agent) says *"use sctxx to extract session `<id>` from Claude Code
and use it here"*, and the receiving agent gets a compact, verified, provenance-linked handoff instead of
a blind restart or a multi-megabyte transcript.

```text
   Claude Code · Codex CLI · Pi  (later: OpenCode · Gemini CLI · CodeWhale)
                 session files on disk
                          │  adapters: clean-room · vendored · documented formats
                          ▼
                canonical IR + active branch
                          │  deterministic ledgers, masking, segmentation   ──►  --llm none artifact
                          ▼
            anchored fold: typed ops, validated, provenance-bound
                          │  repo reconciliation · probe scoring
                          ▼
           handoff.md · handoff.json · state.json  (L0 brief → L3 pointers)
                          │  CLI · Agent Skill · (later) MCP · Pi extension · ACRYL
                          ▼
                 any agent continues the work
```

Frontier-lab compaction is a black box and open-source agents compact with a one-shot summary inside the
loop. sctxx competes on a different axis: **offline, cross-provider, deterministic-first, verified, and
measured**. The product is only as good as its measured ability to let the next agent continue correctly.

## Architectural assessment and constraints

- **Starting point.** The technical design exists (`docs/SCTXX-SPEC.md`, v0.1 draft); no code exists yet.
  The spec describes *what* the system is. This roadmap decides *in what order* it becomes real, and
  supersedes the draft sequencing in spec §18.
- **Upstream seed.** Codex CLI's own compaction path is thin (a short prompt; OpenAI-hosted models compact
  server-side into encrypted items). The reusable Codex assets are the memory Phase-1 extraction pipeline,
  tiered evidence budgeting, rollback-aware history replay, zstd rollout reading, UTF-8-safe truncation,
  secret redaction, and the `apply_patch` grammar. They are vendored at a pinned commit, never depended on.
- **Legal boundaries.** Apache-2.0 attribution for vendored code; no Codex/OpenAI branding; the Claude Code
  adapter is clean-room (never leaked source or its forks); Pi is built against its published format doc.
- **Packaging.** One published crate (`sctxx`, lib + bin). crates.io forbids git/path dependencies, so the
  package must stay publishable from the first commit. `xtask/` is the only unpublished workspace member.
- **Development host.** Primary development and performance evidence come from an M1 Max
  (`aarch64-apple-darwin`). Linux (musl, x86_64/aarch64) and Windows are proven in CI, not assumed.
- **Format volatility.** Agent session formats change without notice. Every adapter records the agent
  version its fixtures came from and warns on unseen versions; fixtures are the contract.
- **Positioning risk.** The LLM fold is the differentiator but also the most expensive and least proven
  part. The deterministic artifact must be independently useful so the product has value even if the fold
  underperforms.

## Planning ledger

`docs/SCTXX-ROADMAP.md` is the global navigator: product direction, milestone sequence, invalidation
conditions, and non-negotiable invariants. It is not a task tracker; no implementation items are checked
here. It changes only when product direction or a governing constraint changes.

`specs/<NNN-slug>/` is the delivery ledger. Each folder is one bounded feature block inside a milestone,
created and progressed with GitHub Spec Kit (`specify → clarify → plan → tasks → analyze → implement →
converge`). Tasks are checked only after their RED/GREEN loop, gate, evidence, and focused commit exist.

`specs/000-wayfinding/` holds the Wayfinder map and decision tickets for questions that are still foggy.
Tickets decide; they do not build. A milestone block's `spec.md` stays a stub until the tickets that
unlock it are resolved.

`docs/SCTXX-SPEC.md` is the architecture reference that feature specs and plans cite by section.
`.specify/memory/constitution.md` holds the binding laws. `docs/DEVELOPMENT-LOG.md` records important
evolutions with full commit hashes. The working method is
`docs/workmethodology/sctxx-hybrid-engineering-methodology.md`.

A block may be exploratory, superseded, or found to deviate from this roadmap. Mark it invalidated or
superseded with the reason and successor; never let a stale spec appear active.

## Milestones

### M0 - Foundation and governance

Make the repository safe to build in public before any session data or vendored code enters it.

- Rust scaffold: single `sctxx` crate, `xtask/`, pinned toolchain, MSRV 1.85, feature flags
  (`zstd`, `api`, `cli-backends`, `eval`, `minimal`).
- Legal scaffolding: `LICENSE` (Apache-2.0), `NOTICE`, `src/vendor/codex/` with the first vendored file
  (UTF-8-safe truncation) and a header-check script.
- CI matrix: macOS arm64, Linux x86_64/aarch64 musl, Windows x86_64; fmt, clippy, tests,
  `--no-default-features`, MSRV, `cargo package --list`.
- Privacy tooling first: `sctxx redact <file> --strict` works on raw JSONL before any adapter exists,
  plus the fixture contribution policy.
- Governance: Spec Kit initialized, constitution ratified, Wayfinder map opened, names `sctxx` reserved on
  crates.io and npm (plus the npm org for platform packages) and the GitHub repository created.

**Feature blocks:** `specs/001-m0-repo-foundation/`, `specs/002-m0-redaction-and-fixture-policy/`

**Exit criterion:** a fresh clone passes every CI gate on all four platforms; the header check passes on a
real vendored file; `cargo package --list` contains `LICENSE` and `NOTICE`; `sctxx redact --strict` has
tests for every secret class in spec §10.2; package names are reserved.

### M1 - Walking skeleton: Claude Code session to any agent, deterministic

Prove the whole boundary end to end with the fewest parts: one provider, no LLM, one real handoff.

- Canonical IR (spec §5) with only the event kinds the Claude Code adapter produces.
- Claude Code adapter with active-branch resolution (`parentUuid` trees, `logicalParentUuid` across
  compaction boundaries); sidechains ignored except spawn/result.
- Session reference resolution for `claude:<id|prefix|last>` only.
- Minimal ledgers: files touched, commands with last status, unresolved error signatures, user messages.
- Recency tail, L0/L1/L3 markdown render, `extract --llm none`, and `expand` for pointers.
- `skill/SKILL.md` and `sctxx skill install` for Claude Code and Codex.

**Feature blocks:** `specs/003-m1-ir-and-claude-code-adapter/`, `specs/004-m1-deterministic-handoff-skeleton/`,
`specs/005-m1-agent-skill-install/`

**Exit criterion:** on the M1 Max, `sctxx extract claude:<id> --llm none --out .sctxx/` completes in under
5 s on a real session of at least 1,000 events; a Codex session given only the installed skill and the
artifact correctly states the goal, the last failing command, and the modified files (recorded as
evidence); every `[evt a–b]` pointer in the artifact resolves through `sctxx expand`.

### M2 - Three providers and an honest deterministic artifact

Widen from one provider to three and make the no-LLM artifact complete and self-verifying.

- Codex CLI adapter: rollout lines, `.jsonl.zst`, `ThreadRolledBack` replay, forks.
- Pi adapter per the published session format (v1–v3, tree walk, branch summaries).
- Discovery across stores: `list`, `find`, `show`, `doctor`, full reference grammar, index cache.
- Complete S1 ledgers (plan, git, native compactions) and S2 masking, tiers, segmentation, chunking.
- S5 repository reconciliation with the read-only git allowlist; full redaction suite on every path.

**Feature blocks:** `specs/006-m2-codex-adapter/`, `specs/007-m2-pi-adapter/`,
`specs/008-m2-session-discovery/`, `specs/009-m2-complete-ledgers-and-reconciliation/`

**Exit criterion:** at least 30 redacted fixtures (at least 10 per provider, covering rewinds, rollbacks,
compaction boundaries, sidechains, `.zst`, malformed lines) are snapshot-tested; parse plus S0–S2 on a
100 MB session runs under 3 s on the M1 Max; a 10-session evaluation corpus exists with baseline
deterministic-probe F1 recorded for `--llm none`.

### M3 - Anchored LLM fold

Add the differentiator without giving up determinism, privacy, or provenance.

- LLM backends: `cli:claude`, `cli:codex`, `cli:pi` (isolated temp cwd), `api:anthropic`,
  `api:openai`, `api:compat`, and `auto` resolution.
- Premap, sequential anchored fold with typed ops, validation gates and a single repair turn, final pass,
  Rust-enforced budgets.
- Cache, checkpoints, and resume; cost estimation and `--max-cost-usd`.

**Feature blocks:** `specs/010-m3-llm-backends/`, `specs/011-m3-anchored-fold/`, `specs/012-m3-cache-and-resume/`

**Exit criterion:** on the 10-session corpus, `--mode standard` beats `baseline:codex-compact` on
deterministic-probe F1 with both `cli:claude` and `cli:codex`; an interrupted extract resumes from its last
checkpoint; a test proves no unredacted secret class reaches any backend request.

### M4 - Public v0.1.0

Ship to the three distribution channels with provenance, and open the project to outside contributors.

- cargo-dist release pipeline for six targets, GitHub artifact attestations, checksums, installers.
- crates.io publish with `cargo binstall` metadata; npm wrapper plus per-platform packages published with
  provenance.
- README with a real demo, SECURITY.md, CONTRIBUTING.md with the clean-room rule, PR template.
- Switch from direct-to-main development to pull requests for outside contributions.

**Feature blocks:** `specs/013-m4-distribution-pipeline/`, `specs/014-m4-v0-1-0-release/`

**Exit criterion:** `cargo install sctxx`, `cargo binstall sctxx`, `npx sctxx`, and the GitHub release
binaries each run `sctxx extract` on macOS, Linux, and Windows; the skill installs and triggers in Claude
Code and Codex from a scripted prompt; the publish step passed its human approval gate.

### M5 - Measured quality

Turn "it seems good" into a public, reproducible number that gates every future change.

- S6 probe loop: deterministic probes plus generated LLM probes with an independent judge.
- `sctxx eval`: baselines (`strip`, `codex-compact`, `none`), held-out continuation test, reports.
- Public corpus and reproducible results table; nightly regression gate; dataset export for prompt
  optimization (e.g. DSPy + GEPA) outside the Rust codebase.

**Feature blocks:** `specs/015-m5-probe-loop/`, `specs/016-m5-eval-harness-and-benchmark/`

**Exit criterion:** the README results table is regenerated by one documented `sctxx eval` command on the
public corpus; the nightly job blocks a prompt or algorithm change that lowers the score by more than 0.1.

### M6 - Everywhere agents work

Meet agents inside their own loops, without forking the core.

- Incremental anchored updates for growing sessions.
- Host mode protocol and `sctxx mcp` server.
- Pi extension using `session_before_compact` for in-loop compaction.
- Stable library/CLI contract for ACRYL's relay and context work (ACRYL-side work stays in the ACRYL
  ledger and follows ACRYL's constitution).
- Export of durable items to lat.md sections.

**Feature blocks:** `specs/017-m6-incremental-update/`, `specs/018-m6-host-mode-and-mcp/`,
`specs/019-m6-pi-extension/`, `specs/020-m6-embedding-contract-for-hosts/`, `specs/021-m6-lat-md-export/`

**Exit criterion:** each integration has a quickstart and acceptance evidence; the Pi extension beats Pi's
built-in compaction on probe score in the eval harness; re-extracting a grown session folds only new chunks.

### M7 - Breadth and 1.0 stability

Earn the right to promise stability.

- Adapters for OpenCode, Gemini CLI, and CodeWhale, each with its own fixture corpus.
- Freeze `sctxx.handoff/v1`, `ops.v1`, `state.v1`, and the CLI surface under a written compatibility
  policy; external security review of redaction and egress paths.

**Feature blocks:** `specs/022-m7-additional-adapters/`, `specs/023-m7-v1-stabilization/`

**Exit criterion:** sctxx 1.0.0 is published with at least six provider adapters, a compatibility policy,
and a completed security review with every finding resolved or documented.

### M8 - Interactive TUI: find a session, extract it, hand it off

Added 2026-09-11 by maintainer decision. The workflow becomes a place rather than a command line:
`sctxx --tui` browses sessions, extracts one, and launches a **new** session in a chosen installed
agent with the handoff already loaded — the last mile extraction has never had, and the difference
between a utility you remember and a tool you live in.

- `sctxx --tui`: an icon rail with **SCTXX first**, plus file tree, search, terminal, and canvas panes
  reused from [croft](https://github.com/vitali87/croft) (MIT, attributed from the first copied line —
  see ticket `12-croft-reuse-and-mit-attribution`).
- Session browser: filter by agent, date, and cwd; fuzzy search; jump by id or path; a preview built
  from the deterministic ledgers, so it costs nothing.
- Extraction form mirroring every `extract` flag, with the backends actually available on this machine.
- Handoff: detect installed agents and start a fresh session seeded with the extracted context.

**Feature blocks:** `specs/024-m8-interactive-tui/`

**Exit criterion:** a developer who has not read the docs goes from "the session I want is in Codex" to
a running Claude Code session that already knows the goal, the constraints, and the next action — in
under 30 seconds, without typing a path or knowing a flag.

## Invalidation and re-planning conditions

- **M1:** if clean-room parsing of Claude Code sessions proves unreliable across current versions, re-plan
  M1 around the Codex adapter (fully inspectable) and record the decision in the Wayfinder map.
- **M3:** if the fold cannot beat `baseline:codex-compact` on the corpus after two prompt/algorithm
  iterations, stop expanding LLM features; ship v0.1.0 as deterministic-first with the fold marked
  experimental, and redraw M5 around finding out why.
- **M4:** if crates.io or npm name reservation fails, rename before any public artifact exists.
- **M6:** if a host (Pi, ACRYL) needs sctxx internals rather than its public contract, treat that as a
  contract gap in sctxx, not a reason to fork the pipeline.
- **Any milestone:** a verified change in a provider's session format that breaks an adapter creates a
  prerequisite block before further feature work in that milestone.

## Explicitly deferred

- A GUI. The TUI in M8 is a terminal interface, not a windowed application.
- A hosted service, accounts, telemetry, or cloud storage.
- Writing into any agent's session store (session injection or transplant).
- A general memory system or code knowledge graph (sctxx exports to such systems; it does not become one).
- Live in-loop compaction before M6.

## Non-negotiable invariants

- Session content is data: never executed, redacted before any LLM request, never committed unredacted.
- Deterministic Rust owns bookkeeping; LLMs only make semantic judgments, and every LLM output is validated.
- Every artifact item carries event-range provenance; constraint quotes match the source verbatim; the
  current repository wins over the artifact.
- Vendored code keeps Apache-2.0 attribution; the Claude Code adapter stays clean-room; no Codex or OpenAI
  branding.
- One published crate, publishable at every commit; local-first, no telemetry.
- stdout carries only the payload; exit codes and schemas change only through versioned contracts.
- A sctxx handoff is a warm start, never project memory: the ledger, source, tests, evidence, and git
  history remain authoritative.
