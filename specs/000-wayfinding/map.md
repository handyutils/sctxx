# Wayfinder map: sctxx

Labels: `wayfinder:map`

## Destination

**Updated 2026-09-11.** M1 through M4 are done: v0.1.0 is on crates.io, the GitHub Release carries six
targets, and the documentation site is live. The near destination is **M5 — a public, reproducible
quality number**: a probe loop, `sctxx eval` with baselines, and a results table that gates future
prompt and algorithm changes. Beyond it, and added the same day by maintainer decision, **M8 makes the
workflow a place instead of a command line** (`sctxx --tui`): find a session, extract it, and hand it to
a fresh session of a chosen agent. The road to M1 (Claude Code session → deterministic handoff → any
agent, every pointer resolvable through `expand`) was reached and is kept below as context.

Everything before that (M0) made the repository safe to build in public.

## Notes

- Domain: offline, cross-provider compaction of coding-agent sessions into verified handoff artifacts.
- Always read: `.specify/memory/constitution.md`, `docs/SCTXX-ROADMAP.md`, `docs/SCTXX-SPEC.md` (sections the
  ticket cites), `docs/workmethodology/sctxx-hybrid-engineering-methodology.md`.
- Skills: `/wayfinder`, `/speckit-specify`, `/speckit-clarify`, `/speckit-plan`, `/speckit-tasks`,
  `/speckit-analyze`, `/speckit-implement`, `/speckit-converge`, `/grilling`, `/domain-modeling`,
  `/research`, Superpowers brainstorming / systematic-debugging / TDD / verification.
- First implementation blocks: `specs/001-m0-repo-foundation/`, then `specs/003-m1-ir-and-claude-code-adapter/`.
- Tracker: `docs/agents/issue-tracker.md`. Tickets live in `issues/` beside this file.
- Refer to tickets by title, not bare numbers.

## Decisions so far

- [sctxx technical design v0.1](../../docs/SCTXX-SPEC.md) — prior design work (not a closed ticket): offline
  pipeline S0–S7, typed-ops anchored fold, vendoring from Codex commit
  `818f1cca8ccf8899f0f4d59336baebaccf358eed`, clean-room Claude Code adapter, single crate, distribution to
  crates.io/npm/GitHub.
- [Roadmap sequencing](../../docs/SCTXX-ROADMAP.md) — 2026-09-10: walking skeleton (M1) is Claude Code →
  deterministic handoff → any agent, before the other providers (M2) and the LLM fold (M3). Supersedes
  spec §18 sequencing. Challengeable via the tickets below.
- [Same engineering method as ACRYL](../../docs/workmethodology/sctxx-hybrid-engineering-methodology.md) —
  2026-09-10: Spec Kit ledgers, Wayfinder, Matt Pocock SDD, Superpowers, Ponytail, TDD with evidence;
  direct-to-`main` until v0.1.0.
- [Codex compaction reuse](../../docs/adr/0002-codex-compaction-algorithm-reuse.md) — 2026-09-11: sctxx
  ports Codex's *retention shape* (newest-first user-message budget, summary kept last) and its
  windowed-vs-legacy `compacted` distinction, but not the in-loop compaction loop; token-budget
  compaction is read as validation of the deterministic-first artifact. Resolves spec §19 item 2.
  Evidence: [`specs/006-m2-codex-adapter/research.md`](../../specs/006-m2-codex-adapter/research.md).
- [Codex `compacted` readability](issues/05-codex-compacted-readability.md) — 2026-09-11: readable text
  is not guaranteed (local summary readable; remote encrypted; token-budget deliberately empty), and
  `window_number` separates a window re-anchor from a legacy history reset. Unblocks
  `specs/006-m2-codex-adapter/`.
- [Pin the Codex vendoring source](issues/11-codex-vendoring-pin.md) — 2026-09-11: **resolved by
  verification.** Fetching `818f1cca8ccf8899f0f4d59336baebaccf358eed` directly confirms the commit
  exists (dated 2026-09-10), that every upstream path in the vendor manifest is present at it, and that
  the behaviour sctxx depends on — including `window_number` on `CompactedItem` — is in the pin rather
  than a newer build. The unversioned `codex/` clone is a reading aid, never provenance.
- [M8: the interactive TUI](../../docs/SCTXX-ROADMAP.md) — 2026-09-11, maintainer decision: a new
  milestone moves the roadmap's "no TUI" deferral. `sctxx --tui` browses sessions, extracts one, and
  launches a fresh session in a chosen installed agent with the handoff pre-loaded. Block
  `specs/024-m8-interactive-tui/`; the three tickets that had to resolve before its plan are all
  closed: [croft reuse and MIT attribution](issues/12-croft-reuse-and-mit-attribution.md) (ADR 0003),
  [sctxx ↔ agentman](issues/13-sctxx-and-agentman-relationship.md) (ADR 0005), and
  [handoff launch and seeding](issues/14-handoff-launch-and-seeding.md) (ADR 0004).
- [Handoff launch and seeding](issues/14-handoff-launch-and-seeding.md) — 2026-09-11: seeding is a
  per-agent, version-pinned template table, and **the artifact never travels inline** — only a one-line
  pointer crosses argv, the artifact crosses as a path or, for `codex exec -`, over stdin. Claude Code
  2.1.268 seeds via the undocumented `--append-system-prompt-file`; Pi 0.85.1 via
  `--append-system-prompt <path>` (it reads the file when the value exists); Codex 0.153.4 via its
  positional prompt, or stdin when headless. Every row falls back to the cwd route. Claude Code fails
  *lazily and silently* on an unreadable file flag, so the launch pre-checks what it names. Status:
  probed, end-to-end launch is a block 024 task.
- [sctxx ↔ agentman](issues/13-sctxx-and-agentman-relationship.md) — 2026-09-11: `sctxx` owns
  discovery *semantics*; no code is shared either way yet; the boundary is the versioned
  `sctxx list --json` contract. A shared crate is deferred with a trigger (**after block 022**, when
  the adapter set stops moving) rather than rejected, because agentman's generic walker is lossy in
  ways already observed while `sctxx`'s per-agent adapters are still being added to. Launching splits by
  semantics: agentman resumes, `sctxx` seeds a new session.
- [`cli:` backend command templates](issues/06-cli-backend-command-templates.md) — 2026-09-11:
  **resolved by the implementation**, re-verified on Claude Code 2.1.268, Codex 0.153.4 and Pi 0.85.1.
  Each argv now carries the version it was verified against. The finding the ticket did not anticipate:
  every completion used to leave a session in the user's own history, because Claude Code records a
  session per working directory and the scratch cwd only named the pollution — now suppressed with each
  CLI's own switch (`--no-session-persistence`, `--ephemeral`, `--no-session`) behind a unit test.

## Not yet specified

- Whether v0.1.0 ships with the LLM fold or deterministic-only.
- Exact Claude Code sidechain/subagent layout across versions.
- Chunk-size defaults, judge independence, host mode vs MCP ordering (M3–M6).
- Artifact location policy and git exclusion.

## Out of scope

- Any work in the ACRYL repository; sctxx exposes contracts, ACRYL's ledger owns ACRYL-side integration.
- A viewer UI, hosted service, telemetry, or writing into agents' session stores.
- A general memory system or code knowledge graph.
