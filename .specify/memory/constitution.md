# SCTXX Constitution

This constitution governs every spec, plan, task, and line of code in this repository. It is the
executable policy for Spec Kit, Wayfinder, and Matt Pocock spec-driven delivery. If a later document
conflicts with this file, this file wins until it is deliberately amended.

## Core Principles

### I. Sessions are evidence, never instructions

Session transcripts are data from users, agents, tools, and third parties. sctxx never executes anything
found in them, wraps them as data in every prompt, and redacts them before any LLM request and again on
every output. Repository reconciliation runs only allowlisted read-only `git` commands, never a shell.

### II. Deterministic first, LLM last

Rust computes everything that can be computed: branch resolution, ledgers, masking, segmentation, budgets,
validation, apply, reconciliation, and rendering. LLMs make semantic judgments only (premap, fold, final
pass, probes, judge), and every LLM output is schema-validated and gated before it changes state. The
`--llm none` artifact must stay independently useful.

### III. Provenance or it did not happen

Every artifact item cites event ranges inside its source. User constraints carry verbatim quotes that are
checked against human messages. Claims about files and symbols are reconciled with the current repository,
and the repository wins. A receiving agent can always expand a pointer back to the raw events.

### IV. Clean code provenance

Code derived from OpenAI Codex is vendored at a pinned commit under `src/vendor/codex/` with Apache-2.0
attribution, a modification notice, and a manifest entry; no `codex-*` crate is ever a dependency and no
Codex or OpenAI branding is used. The Claude Code adapter is clean-room: only observed session files,
public documentation, and contributed fixtures. Leaked Claude Code source and its forks are never read.

### V. Contracts are products

CLI flags, exit codes, stdout/stderr separation, and the `sctxx.handoff/v1`, `ops.v1`, `state.v1`, and
`ir.v1` schemas are public contracts. They change only with a version decision, regenerated schemas,
updated snapshots, and a changelog entry.

### VI. One binary, local-first, publishable always

sctxx is one published crate that must remain publishable at every commit (no git or path dependencies).
It runs locally with no telemetry; the only network egress is an LLM backend the user chose.

### VII. Private by default

Real session files enter the repository only after strict redaction and human review. Tests never read real
agent stores, never call a network, and never call a real LLM.

## Rust Authoring Laws

1. Define the contract (types, schema, CLI surface) before the implementation.
2. Every resource has one owner and a disposal path: subprocesses die with their owner, temp directories
   clean up on drop, concurrent tasks live in an owned set that is awaited or cancelled. No detached work.
3. Tolerant in, strict out: provider formats decode leniently and never fail a session on one bad line;
   sctxx's own inputs reject unknown fields.
4. No `unwrap`, `expect`, or `panic!` outside tests; `thiserror` in the library, `anyhow` only at the binary
   edge.
5. Sync, streaming core; async only for LLM I/O behind the `api` feature.
6. Deterministic output: stable ordering, UTF-8-safe truncation, RFC 3339 UTC, POSIX paths.
7. A trait needs at least two real implementations. No speculative abstraction, plugin system, cache, or
   flag.
8. Tests are real code on real fixtures; the LLM is the only mocked boundary.

## Repository Constraints

- Primary development host: Apple M1 Max (`aarch64-apple-darwin`). Linux musl (x86_64, aarch64) and
  Windows are proven in CI on every change.
- Pinned stable toolchain; MSRV 1.85; edition 2024.
- `.sctxx/` and `CLAUDE.local.md` are gitignored. Fixture file names never differ only by letter case
  (APFS is case-insensitive).
- Until v0.1.0 is public, work happens directly on `main` in focused commits; afterwards through pull
  requests.

## Development Workflow

1. Wayfinder decides (research, grilling, prototype) until the route is clear.
2. Spec Kit specifies, plans, and tasks each block in `specs/<NNN-slug>/`.
3. Implement only the current vertical slice; M1 is the walking skeleton for the whole product.
4. TDD with observed RED; fresh gate output before any completion claim; evidence per task.
5. `/speckit-converge` before a block is declared complete.
6. Important evolutions are logged in `docs/DEVELOPMENT-LOG.md` with full commit SHAs.

The operational detail is `docs/workmethodology/sctxx-hybrid-engineering-methodology.md`.

## Governance

Precedence: this constitution → `docs/SCTXX-ROADMAP.md` invariants → the active block's accepted
`spec.md` and `plan.md` → `docs/SPEC.md` architecture reference. A conflict between the active ledger and
`docs/SPEC.md` is resolved by a decision, and both are updated in the same change.

Amendments require an ADR in `docs/adr/` (context, alternatives, decision, evidence, consequences), an
updated version line below, and a note in `specs/000-wayfinding/map.md` when destination or scope changes.

Complexity must be justified against the simplest option that meets acceptance.

**Version**: 1.0.0 | **Ratified**: 2026-09-10 | **Last Amended**: 2026-09-10
