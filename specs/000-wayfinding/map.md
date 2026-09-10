# Wayfinder map: sctxx

Labels: `wayfinder:map`

## Destination

Reach roadmap **M1**: from a real Claude Code session on the M1 Max,
`sctxx extract claude:<id> --llm none --out .sctxx/` produces a handoff that lets a Codex session state the
goal, the last failing command, and the modified files correctly, with every pointer resolvable through
`sctxx expand`. Everything before that (M0) makes the repository safe to build in public.

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

## Not yet specified

- Whether v0.1.0 ships with the LLM fold or deterministic-only.
- Exact Claude Code sidechain/subagent layout across versions.
- Readability of Codex local `compacted` lines.
- `cli:` backend command templates per agent CLI version.
- Chunk-size defaults, judge independence, host mode vs MCP ordering (M3–M6).
- Artifact location policy and git exclusion.

## Out of scope

- Any work in the ACRYL repository; sctxx exposes contracts, ACRYL's ledger owns ACRYL-side integration.
- A viewer UI, hosted service, telemetry, or writing into agents' session stores.
- A general memory system or code knowledge graph.
