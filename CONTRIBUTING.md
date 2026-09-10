# Contributing to sctxx

Thanks for helping. Two rules here are unusual and non-negotiable; everything else is ordinary
Rust practice.

## 1. The clean-room rule

Claude Code is not open source. The Claude Code adapter may only be written from:

- the shape of session files on your own machine,
- public Anthropic documentation,
- fixtures contributed by users from their own sessions.

**Never read, copy, paraphrase, or port code from leaked Claude Code source or any fork of it**
("OpenClaude"-style repositories included). Do not go looking for them. Every pull request that
touches an adapter must confirm this.

Pi is built against its published session-format documentation. Codex is Apache-2.0 and may be
ported — see rule 2.

## 2. Vendored code keeps its attribution

Code derived from OpenAI Codex lives only in `src/vendor/codex/`, at the pinned commit
`818f1cca8ccf8899f0f4d59336baebaccf358eed`, and carries this header:

```rust
// Portions derived from OpenAI Codex (https://github.com/openai/codex),
// commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file <original path>.
// Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
// Modified by the sctxx authors: <one-line description of changes>.
```

Adding or changing a vendored file also updates `src/vendor/codex/README.md` and
`docs/SCTXX-SPEC.md` Appendix A. `scripts/check-vendor-headers.sh` enforces this in CI.

Never add a `codex-*` crate dependency, and never use "Codex" or "OpenAI" in a package, binary,
or product name.

## 3. Session files are private data

A session transcript contains everything someone said to an agent, and often their code, paths,
and secrets.

- Never commit a real session file. Fixtures under `tests/fixtures/` are synthetic.
- If a real session is genuinely needed as a fixture, it needs its owner's explicit consent,
  `sctxx redact --strict`, and a line-by-line human read of the result. Redaction is pattern
  matching, not a guarantee.
- Tests never read real `~/.claude`, `~/.codex`, or `~/.pi` stores, never call a network, and
  never call a real model. The test harness overrides `HOME` for exactly this reason.

## Getting set up

```sh
git clone https://github.com/handyutils/sctxx
cd sctxx
cargo test --all-features
```

## The gate

Before you call a change done, all four must pass:

```sh
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-features
cargo test --no-default-features      # the minimal build has no network code
```

Plus, when relevant:

```sh
scripts/check-vendor-headers.sh
cargo package --list                  # must include LICENSE, NOTICE, prompts/, schemas/, skill/
```

## How the code is organized

```text
src/ir.rs           the canonical event model every adapter targets
src/adapters/       provider files to IR; tolerant in, strict out
src/pipeline/       ledgers, masking, segmentation, fold, reconcile, render
src/pipeline/fold/  typed ops, state, validation gates, prompts
src/llm/            backends; the only place a network call may happen
src/cli/            one file per subcommand, thin
src/vendor/codex/   ported Apache-2.0 code with attribution
```

The governing rule is **deterministic first**: if Rust can compute it, Rust computes it. A model
is for semantic judgment only, and every model output is validated before it can change state.

## Changing an adapter

Provider formats change without notice, so fixtures are the contract.

1. Add a fixture to `tests/fixtures/<agent>/` that reproduces the shape.
2. Run `INSTA_UPDATE=always cargo test --test adapters` and **read the snapshot diff**. Never
   accept snapshots in bulk.
3. Record the agent version in `tests/fixtures/README.md`.

## Changing a prompt

Prompts in `prompts/` are versioned and recorded in every artifact. Editing one means bumping
its `version` front-matter in the same change.

## Changing a contract

CLI flags, exit codes, and the `sctxx.handoff/v1`, `ops.v1`, `state.v1`, and `ir.v1` schemas are
public contracts. Changing one needs a version decision, the regenerated schema, updated
snapshots, and a `CHANGELOG.md` entry, all in the same change.

## Commits

Conventional Commits with a module scope:

```text
feat(adapters/codex): replay ThreadRolledBack
fix(fold): reject sources outside the chunk range
```

## Where work is tracked

`docs/SCTXX-ROADMAP.md` holds the direction, `specs/<NNN-slug>/` the delivery ledger for each
feature block, and `specs/000-wayfinding/` the open decisions. The working method is
`docs/workmethodology/sctxx-hybrid-engineering-methodology.md`.
