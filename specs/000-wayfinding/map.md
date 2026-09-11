# Wayfinder map: sctxx

Labels: `wayfinder:map`

## Destination

**Updated 2026-09-11.** M1 through M4 are done: v0.1.0 is on crates.io, the GitHub Release carries six
targets, and the documentation site is live. The destination is now **M5 — a public, reproducible
quality number**: a probe loop, `sctxx eval` with baselines, and a results table that gates future
prompt and algorithm changes. The road to M1 (Claude Code session → deterministic handoff → any agent,
every pointer resolvable through `expand`) was reached and is kept below as context.

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

## Not yet specified

- Whether v0.1.0 ships with the LLM fold or deterministic-only.
- Exact Claude Code sidechain/subagent layout across versions.
- `cli:` backend command templates per agent CLI version.
- Chunk-size defaults, judge independence, host mode vs MCP ordering (M3–M6).
- Artifact location policy and git exclusion.

## Out of scope

- Any work in the ACRYL repository; sctxx exposes contracts, ACRYL's ledger owns ACRYL-side integration.
- A viewer UI, hosted service, telemetry, or writing into agents' session stores.
- A general memory system or code knowledge graph.
