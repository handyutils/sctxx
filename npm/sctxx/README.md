# sctxx

**Extract any coding-agent session, hand it to the next agent.**

You hit the context limit in Claude Code at 2am. Or yesterday's work is in Codex and today you are in
Pi. Or you are back on a repo after a week and cannot remember where you were.

The transcript is on disk — often hundreds of MB of tool output — and pasting it into a new session is
neither possible nor useful. `sctxx` reads that file and writes a **compact, verified,
provenance-linked handoff** the next agent loads instead of starting blind.

```sh
npm i -g sctxx
sctxx skill install
```

Then, from inside any agent:

> use sctxx to extract session 7c1e8f82 from claude code and continue here

```text
302 MB transcript, 141,409 events, 288 user turns  ──►  7.9 KB handoff, 3.2k tokens   (5.0 s, no model)
```

That is a real measurement, not a slogan — command, corpus, and host are in the
[performance evidence](https://github.com/handyutils/sctxx/blob/main/specs/004-m1-deterministic-handoff-skeleton/evidence/perf-synthetic-2026-09-11.md).

**Docs:** [handyutils.github.io/sctxx](https://handyutils.github.io/sctxx/) ·
[Quick start](https://handyutils.github.io/sctxx/#quickstart) ·
[Prompts for your agent](https://handyutils.github.io/sctxx/#prompts) ·
[Command reference](https://handyutils.github.io/sctxx/#commands) ·
[Worked examples](https://handyutils.github.io/sctxx/#workflows) ·
[Troubleshooting](https://handyutils.github.io/sctxx/#troubleshooting)

---

## Built on algorithms that already survive daily use

Most "AI context" tooling is a prompt and a hope. `sctxx`'s core is deterministic Rust, and the parts
that are not obvious — how to compress a transcript, what to keep, what to throw away, how to survive
a rollback — were not invented here. They are **ported from OpenAI's Codex CLI**, the Rust coding agent
that runs on real repositories every day ([github.com/openai/codex](https://github.com/openai/codex),
Apache-2.0, pinned at commit `818f1cca8ccf8899f0f4d59336baebaccf358eed`).

We read that codebase and took the five things that matter for context extraction:

| What we ported | Where it comes from upstream | Why it matters to you |
|---|---|---|
| **Evidence tiering** | `codex-rs/memories/write/src/rollout_input.rs` (`serialize_tiered_input`) | Codex's priority order — human turns first, then finals, agent messages, commentary, context, tool output — with tool output capped and rows byte-capped. `sctxx` uses that ordering to classify rows, and cuts the recency tail at an **episode boundary** rather than by tiered fill. We ported the tiered *selector* too and it is not wired in yet ([audit](https://github.com/handyutils/sctxx/blob/main/docs/CODEX-PROVENANCE-AUDIT.md), [ticket 15](https://github.com/handyutils/sctxx/blob/main/specs/000-wayfinding/issues/15-tiered-selection-is-unreachable.md)); we would rather say so than let a table imply otherwise. |
| **Rollback-aware replay** | `codex-rs/core/src/session/rollout_reconstruction.rs` | `ThreadRolledBack { num_turns }` means the newest N user turns were undone. Replaying that correctly is the difference between summarising work that still exists and summarising work the developer deleted. On one real session this put 6,753 of 14,151 events on the live branch. |
| **UTF-8-safe truncation + the 4-bytes-per-token estimate** | `codex-rs/utils/string/src/truncate.rs` | Every budget, cap, and truncation in `sctxx` uses one shared, byte-safe primitive. No tokenizer dependency, no panics on a multi-byte boundary, no two components disagreeing about how big something is. |
| **Secret redaction** | `codex-rs/secrets/src/sanitizer.rs` | Their pattern set (`sk-…`, `AKIA…`, `Bearer …`, `key=value`), extended with Anthropic, GitHub, Slack, Stripe, Google keys, JWTs, PEM blocks, and connection strings. Runs before any model sees the transcript, again on the model's answer, again on the artifact. |
| **`apply_patch` grammar** | `codex-rs/apply-patch/src/parser.rs` | `*** Add File:`, `*** Update File:`, `*** Delete File:`, `*** Move to:`. Lets the file ledger read a Codex edit exactly instead of guessing from prose. |

Two of our prompts are derived the same way: `fold_system.md` from Codex's memory-extraction prompt
`memories/write/templates/memories/stage_one_system.md`, and `handoff_preamble.md` from the compaction
handoff prefix `prompts/templates/compact/summary_prefix.md`.

**We also learned from their compaction design — and we do not claim to have extracted it.**
Codex has no single local compaction algorithm to extract: there is a dispatcher over four strategies,
and one of them runs on OpenAI's servers. What we took from it is the *shape* of retention and the
meaning of a `compacted` marker. Codex has three compaction paths — local
summarisation, server-side v2, and a token-budget path that skips summarisation entirely and rebuilds
the window from canonical context plus retained evidence. Reading them gave us two things `sctxx`
would otherwise have got wrong: that a provider's `compacted` marker means *either* "history was
reset" *or* "the window was re-anchored, the transcript is intact", depending on `window_number`; and
that the newest user messages, not the assistant's prose, are what is worth keeping under pressure.
Both are implemented; the analysis is in
[ADR 0002](https://github.com/handyutils/sctxx/blob/main/docs/adr/0002-codex-compaction-algorithm-reuse.md)
and the [Codex compaction research](https://github.com/handyutils/sctxx/blob/main/specs/006-m2-codex-adapter/research.md).

**How we use it, and how we do not.** `sctxx` *ports* these pieces into
[`src/vendor/codex/`](https://github.com/handyutils/sctxx/tree/main/src/vendor/codex) with an
Apache-2.0 attribution header on every file; it does **not** depend on any `codex-*` crate, so the
binary stays small and the crate stays publishable. Each ported file is listed in the vendor manifest
and in [spec Appendix A](https://github.com/handyutils/sctxx/blob/main/docs/SCTXX-SPEC.md). `sctxx` is
an independent project: not affiliated with, sponsored by, or endorsed by OpenAI.

The result is a tool whose foundations are battle-tested in a production coding agent, wrapped in a
pipeline built for this one job — so you are trusting engineering that already runs on real repos, not
a prompt someone vibed into a YAML file.

---

## Why the output is trustworthy

- **Deterministic first.** Rust computes branch resolution, the file/command/error ledgers, masking,
  budgets, validation, rendering. A model is used only for semantic judgement, and `--llm none`
  produces a complete, useful artifact with **zero** model calls and zero network.
- **Provenance or it did not happen.** Every item cites the event range that justifies it.
  `sctxx expand <ref> 4122..4381` prints those events back. Nothing is a claim you cannot check.
- **Quotes are checked, not paraphrased.** A rule attributed to you must carry your verbatim words,
  matched against the transcript. An invented quote is rejected before it reaches the artifact, and the
  rejection is recorded.
- **The repository wins.** After extraction, `sctxx` reconciles the artifact against your working tree
  with read-only git commands and marks anything `stale` or `contradicted`.
- **Transcripts are data, never instructions.** Nothing from a session is ever executed, and every
  prompt fences transcript text and states it must not be followed.

## Quick start

```sh
npm i -g sctxx                 # this package: prebuilt binary for your OS
sctxx skill install            # teach your agents when to reach for it
sctxx doctor                   # what stores/backends were detected
cd ~/code/your-project
sctxx extract last --out .sctxx/ --llm none
less .sctxx/handoff.md
```

Prefer no Node? `cargo install sctxx`, `cargo binstall sctxx`, or a binary from
[Releases](https://github.com/handyutils/sctxx/releases).

## Use cases

**1. A session ran out of context — keep going.** Your agent compacted itself into a paragraph and lost
the error you were chasing. The original file still has everything.

```sh
sctxx extract claude:7c1e8f82 --out .sctxx/          # then: "read .sctxx/handoff.md and continue"
```

**2. Switch agents mid-task.** Start in Claude Code, finish in Codex or Pi. The artifact is
provider-independent by construction.

```sh
sctxx extract claude:last --focus "finish the exporter" --out .sctxx/
```

**3. Come back after a week.** Goal, current step, the failing command, and the approaches already
abandoned — as a brief, not a wall of transcript.

```sh
sctxx list --limit 5
sctxx find "auth migration"
```

**4. Hand off to a colleague's agent.** `.sctxx/handoff.md` is a file. Commit it, attach it, paste the
L0 brief into a chat. Every claim carries a pointer back to the raw events.

**5. Air-gapped or subscription-only.** `--llm none` never calls a model. `--llm auto` uses an agent
CLI you are already logged into, run in an empty temp directory with tools disabled.

## What you get

| Layer | Content | Budget |
|---|---|---|
| **L0 Brief** | goal, last request, current step, next actions, hard constraints with verbatim quotes, dead ends, verify-first commands, repo drift since the session | ≤ 1,200 tokens |
| **L1 Items** | every item with id, confidence, verification, `[evt a–b]` pointers; files touched; last known test/build status; unresolved errors; plan; git | rest of `--budget` |
| **L2 Recency tail** | the end of the session, near-verbatim | `--tail`, 12k default |
| **L3 Retrieval** | source block and ready-to-run `sctxx expand` commands | ≤ 150 tokens |

Plus `handoff.json`, `state.json`, `ledgers.json`, `report.json` beside it, so a reviewer can audit
anything the brief asserts.

## Works with

**Reads sessions from:** Claude Code (`~/.claude/projects/`), Codex CLI (`~/.codex/sessions/`,
`archived_sessions/`, `.jsonl.zst`), Pi (`~/.pi/agent/sessions/`, format v1–v3).

**Installs its skill into:** Claude Code, Codex CLI, Pi — `sctxx skill install`, user or project scope.
It refuses to overwrite a copy you edited.

## Commands

`sctxx list` · `find` · `show` · `extract` · `expand` · `verify` · `redact` · `skill` · `schema` ·
`doctor` — full reference at
[handyutils.github.io/sctxx/#commands](https://handyutils.github.io/sctxx/#commands).

Exit codes are a contract: `3` ambiguous reference (candidates on stdout), `4` not found, `5` parse-rate
limit, `6` no LLM backend, `7` `--strict` contradiction, `8` probe score below threshold.

## About this npm package

This is the wrapper. It declares six per-platform packages as `optionalDependencies`, so npm installs
the binary matching your OS and CPU and nothing else. No Rust toolchain, no postinstall download, no
network at install time beyond the registry. On an unsupported platform it tells you so and points at
`cargo install` instead of failing.

| Platform | Package |
|---|---|
| macOS arm64 / x64 | `sctxx-darwin-arm64`, `sctxx-darwin-x64` |
| Linux arm64 / x64 (static musl) | `sctxx-linux-arm64`, `sctxx-linux-x64` |
| Windows x64 / arm64 | `sctxx-win32-x64`, `sctxx-windows-arm64` |

## Links

**Documentation site — everything on one page: <https://handyutils.github.io/sctxx/>**

| Page section | Link |
|---|---|
| Why this exists | <https://handyutils.github.io/sctxx/#why> |
| Quick start | <https://handyutils.github.io/sctxx/#quickstart> |
| Prompts to give your agent | <https://handyutils.github.io/sctxx/#prompts> |
| Pointing at a specific session | <https://handyutils.github.io/sctxx/#session-ids> |
| What the artifact contains | <https://handyutils.github.io/sctxx/#artifact> |
| Why you can trust it | <https://handyutils.github.io/sctxx/#trust> |
| LLM backends | <https://handyutils.github.io/sctxx/#backends> |
| Command reference | <https://handyutils.github.io/sctxx/#commands> |
| Worked examples | <https://handyutils.github.io/sctxx/#workflows> |
| Troubleshooting and exit codes | <https://handyutils.github.io/sctxx/#troubleshooting> |

**Source, releases, and the provenance behind the claims**

| Resource | Link |
|---|---|
| Repository (original source) | <https://github.com/handyutils/sctxx> |
| Issues and questions | <https://github.com/handyutils/sctxx/issues> |
| Releases — binaries for six targets, with checksums | <https://github.com/handyutils/sctxx/releases> |
| Changelog | <https://github.com/handyutils/sctxx/blob/main/CHANGELOG.md> |
| Architecture spec | <https://github.com/handyutils/sctxx/blob/main/docs/SCTXX-SPEC.md> |
| Roadmap | <https://github.com/handyutils/sctxx/blob/main/docs/SCTXX-ROADMAP.md> |
| Milestone ledger (`specs/`) | <https://github.com/handyutils/sctxx/tree/main/specs> |
| Development log | <https://github.com/handyutils/sctxx/blob/main/docs/DEVELOPMENT-LOG.md> |
| **Vendored Codex manifest, file by file** | <https://github.com/handyutils/sctxx/blob/main/src/vendor/codex/README.md> |
| Codex compaction research | <https://github.com/handyutils/sctxx/blob/main/specs/006-m2-codex-adapter/research.md> |
| ADR: what is reused from Codex, and why | <https://github.com/handyutils/sctxx/blob/main/docs/adr/0002-codex-compaction-algorithm-reuse.md> |
| Agent Skill source | <https://github.com/handyutils/sctxx/blob/main/skill/SKILL.md> |
| Contributing | <https://github.com/handyutils/sctxx/blob/main/CONTRIBUTING.md> |
| Security policy | <https://github.com/handyutils/sctxx/blob/main/SECURITY.md> |

**Install from**

| Channel | Link |
|---|---|
| npm | <https://www.npmjs.com/package/sctxx> |
| crates.io | <https://crates.io/crates/sctxx> |
| GitHub Releases | <https://github.com/handyutils/sctxx/releases> |
| The upstream we port from — OpenAI Codex CLI | <https://github.com/openai/codex> |

## Licence

Apache-2.0. Includes code derived from [openai/codex](https://github.com/openai/codex) (Apache-2.0) at
commit `818f1cca8ccf8899f0f4d59336baebaccf358eed`; every derived file carries an attribution header.
Not affiliated with or endorsed by OpenAI or Anthropic.
