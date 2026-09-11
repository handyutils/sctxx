# sctxx

**Session ConTeXt eXtractor.** Turn a finished coding-agent session into a warm start for any
other coding agent.

[![crates.io](https://img.shields.io/crates/v/sctxx.svg)](https://crates.io/crates/sctxx)
[![docs](https://img.shields.io/badge/docs-handyutils.github.io%2Fsctxx-blue)](https://handyutils.github.io/sctxx)
[![license](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)

You hit a context limit in Claude Code, or you want to continue yesterday's Codex work in Pi.
The transcript is on disk — often hundreds of megabytes of tool output — and pasting it into a
new session is neither possible nor useful.

`sctxx` reads that transcript and writes a compact, verified, provenance-linked **handoff
artifact** the next agent can load instead.

```sh
cargo install sctxx
sctxx skill install          # teach your agents when to use it
```

Then, from inside any agent:

> use sctxx to extract session 7c1e8f82 from claude code and continue here

or directly:

```sh
sctxx extract claude:7c1e8f82 --out .sctxx/
```

```text
302 MB transcript, 141,409 events, 288 user turns  ──►  7.9 KB handoff, 3.2k tokens   (5.0 s, no model)
```

## Install

```sh
npm i -g sctxx                 # prebuilt binary for your OS, no toolchain needed
cargo install sctxx            # builds from crates.io
sctxx update                   # update the way you installed it (npm or cargo, detected)
cargo binstall sctxx           # prebuilt binary via cargo
```

## Supported agents

| Agent | Store | Notes |
| --- | --- | --- |
| Claude Code | `~/.claude/projects/` | rewinds, compaction boundaries, subagent transcripts |
| Codex CLI | `~/.codex/sessions/`, `archived_sessions/` | `.jsonl.zst`, rollback replay, `apply_patch` |
| Pi | `~/.pi/agent/sessions/` | session format v1–v3, branch summaries |

Adding a provider means writing one adapter to the canonical IR.

## Common commands

```sh
sctxx list                                   # what sessions exist here
sctxx find "auth migration"                  # find one by topic
sctxx extract claude:last --out .sctxx/      # the most recent session in this directory
sctxx extract codex:6f1a2b3c --llm none      # deterministic, no model, no network
sctxx extract pi:last --focus "finish the exporter"
sctxx expand claude:7c1e8f82 4122..4381 --context 3
sctxx verify .sctxx/ --strict                # is this handoff still true?
sctxx doctor                                 # what did sctxx detect on this machine?
```

`sctxx extract --dry-run` prints the plan and the estimated token cost before spending anything.

## LLM backends

The fold is optional and works with whatever you already have.

| `--llm` | Uses |
| --- | --- |
| `none` | nothing. Deterministic artifact. |
| `auto` *(default)* | an API key if present, else an installed agent CLI, else `none` |
| `cli:claude`, `cli:codex`, `cli:pi` | your existing subscription login, in an empty temp directory with tools disabled |
| `api:anthropic`, `api:openai` | `ANTHROPIC_API_KEY` / `OPENAI_API_KEY` |
| `api:compat/<model>` | any OpenAI-compatible endpoint via `SCTXX_BASE_URL` (OpenRouter, DeepSeek, Ollama, vLLM, LM Studio) |



### The main line

Get one agent's session into another agent:

```sh
sctxx handoff last --to claude            # extract the context and print the command
sctxx handoff last --to claude --run      # ...and start the agent with it loaded
sctxx handoff last --json                 # who could continue this session?
```



Measured on an Apple M1 Max (release build) against a synthetic 302 MB session with tool-output-heavy
turns; see [`specs/004-m1-deterministic-handoff-skeleton/evidence/perf-synthetic-2026-09-11.md`](specs/004-m1-deterministic-handoff-skeleton/evidence/perf-synthetic-2026-09-11.md)
for the command, the raw numbers, and the memory characteristic.

## What makes the output trustworthy

Most compaction is a model reading a transcript and writing a paragraph. sctxx is built the
other way round.

- **Deterministic first.** Rust computes branch resolution, the file/command/error ledgers,
  masking, budgets, validation, and rendering. A model is used only for semantic judgment.
  `--llm none` produces a complete, useful artifact and calls nothing.
- **Provenance or it did not happen.** Every item cites the event range that justifies it, and
  `sctxx expand <ref> 4122..4381` prints those events back. Nothing in the artifact is a claim
  you cannot check.
- **Constraints are quoted, not paraphrased.** A rule attributed to you must carry your verbatim
  words, checked against the transcript. An invented quote is rejected before it can reach the
  artifact — and the rejection is recorded in `state.json`. The layer that finds them runs without a
  model, so the section exists even when `--llm none` does, and a deterministic verifier restores any
  constraint the semantic pass drops. The artifact reports `constraints / preserved / restored /
  missing` rather than asserting that nothing was lost — and states, in the section itself, what a
  pattern cannot see: a rule you wrote declaratively is invisible to it, so a missing rule is unknown,
  not permission.
- **The prompt is checked against the reader's window before it is sent.**
  `--model-context 200000 --max-completion 8000` makes sctxx verify `K ≤ L` on every call and refuse
  once, immediately, naming each term (`K = system + state + ledgers + premap + chunk + scaffold`)
  instead of failing forty times over two hours.
- **The repository wins.** After extraction, sctxx reconciles the artifact against your working
  tree with read-only git commands and marks anything `stale` or `contradicted`.
- **Secrets are redacted** before any model call, again on the model's output, and again on the
  rendered artifact.
- **Nothing from a transcript is ever executed**, and every prompt that embeds transcript text
  fences it as data.

## Built on published research

sctxx does not invent its compaction policy. Every mechanism below is taken from a peer-reviewed
paper or a public artifact, with a note in `docs/research/` recording what transferred, what did
not, and what the measurement said when it was implemented.

| Source | What sctxx takes from it |
| --- | --- |
| **[The Compaction Cliff in Long-Running AI Agent Memory](https://arxiv.org/html/2608.22752v1)**<br>Zerhoudi, Mitrović, Granitzer — CIKM 2026 | The finding that uniform summarisation loses safety rules at the same rate as everything else — best-of **0.53** of an agent's constraints survive a 50 % budget, **0.10** after five rounds — and its remedy: type an item before you compact it, keep the safety-critical class verbatim, and **verify afterwards**. sctxx's deterministic constraint layer is the no-LLM classifier that paper recommends, plus its post-compaction verifier. Also its warning, which sctxx reproduces in the artifact: a run without the verifier reported apparent 1.00 recall while silently dropping a mean 57 % of the constraints it should have kept. |
| **[Addressable Recall Compaction](https://arxiv.org/html/2607.25066v1)**<br>Dang, Ichikawa, Fatima, Shirahata — Fujitsu Research / RIKEN AIP | That a citation is only worth its cost if it is cheaper than what it replaces, so the pointer is **charged to the budget**; that recovery must be paged in exact non-overlapping chunks, because a head/tail window "would irreversibly discard interior tokens"; and Theorem 15's `K = B + M + R + P + Q + η ≤ L` prompt check, which sctxx enforces before every model call — named per term, so a refusal says *which* knob to turn. |
| **[Beyond Compaction: Structured Context Eviction](https://arxiv.org/html/2606.11213v1)**<br>Semenov, Dorofeev — Kiz8 | The episode-graph and typed-eviction framing, and a cautionary measurement: its eviction algorithm is not implementable as written (the candidate predicate omits "not fully evicted", so the loop cannot terminate). sctxx took the framing, not the policy. |
| **[Context Compaction Theory](https://arxiv.org/html/2608.01326v1)**<br>Tirmazi, Markelon, Bishop, Mitzenmacher | Why the deterministic layers are the floor: the paper's lower bounds "bound every GEN algorithm regardless of how its interpreter is computed", while its upper bounds are merely existential. It is also why sctxx does **not** market a compression ratio — the same paper shows required budget can be Ω(Nm) bits, no better than storing the dependencies uncompressed. |
| **[saminkhan1/context-compression](https://github.com/saminkhan1/context-compression)** | Its verification discipline: re-derive the result from the stored form and compare hashes, rather than asserting that the render was lossless. |

Where a source's setting differs, sctxx says so rather than borrowing the authority. That paper's
corpus is *authored rules files*, where `make sure …` opens a rule; in a *transcript* it opens a
task. Measured on a real 274-turn session, the difference is the whole ballgame: matching on markers
anywhere in a sentence returns 40 "constraints" of which none of the first six inspected is an
instruction; anchoring the marker at the head of the clause returns 1, which is real. The full
measurement, including the recall this layer does **not** have, is in
[`docs/research/`](docs/research/) and [ADR 0008](docs/adr/0008-deterministic-typed-layer.md).

## The artifact

Four layers, cheapest first, so an agent can stop reading as soon as it knows enough.

| Layer | Content |
| --- | --- |
| **L0 Brief** | goal, last user request, current step, next actions, hard constraints with quotes, dead ends, verify-first commands, what changed in the repo since |
| **L1 Items** | every item with id, confidence, verification, and `[evt a–b]` pointers; files touched; last known command status; unresolved errors; the plan; git activity |
| **L2 Recency tail** | the end of the session, near-verbatim |
| **L3 Retrieval** | the source and ready-to-run `sctxx expand` commands |

An excerpt from a real run:

```markdown
**Current step** (S2): Making TrustTier 3 actually sandboxed in ModuleHost.spawn.
`pnpm vitest run packages/ext-engine` still fails: TypeError: Cannot read properties
of undefined (reading 'capabilities') at module-host.ts:41:22. [evt 12–15]

**Hard constraints**
- (C1) "Never auto-install extensions from the registry without asking me." [evt 0]

**Don't retry**
- (X1) Wiring the raw manifest into ModuleHost.spawn — spawn reads
  manifest.capabilities, which is undefined. [evt 7–11]
```


## sctxx Handoff Architecture - What Comes from Codex vs What Is Original

<img width="1672" height="941" alt="image" src="https://github.com/user-attachments/assets/7fd349db-5903-4f57-a97a-9ca394fc370b" />




It is **deterministic**: no model, no tokens, seconds. The artifact carries every `[evt a-b]` pointer,
every ledger, and the recency tail, and the receiving agent follows pointers with `sctxx expand`.
`--llm cli:claude` asks for the model-written fold, which is a choice: on a 100k-event session that is
41 fold calls plus 40 premap calls.

The same thing with two keystrokes: `sctxx --tui`, pick a session, press `h`.

The npm package is a shim: it depends on a per-platform package containing the binary, so `npm i -g
sctxx` needs no Rust toolchain. On an unsupported platform it says so and points at `cargo install`
rather than failing the install. See [`npm/README.md`](npm/README.md).

Or download a binary from [Releases](https://github.com/handyutils/sctxx/releases).

## Documentation

**Everything on one page: <https://handyutils.github.io/sctxx/>**

| Section | Link |
|---|---|
| Quick start | <https://handyutils.github.io/sctxx/#quickstart> |
| Prompts to give your agent | <https://handyutils.github.io/sctxx/#prompts> |
| Pointing at a specific session | <https://handyutils.github.io/sctxx/#session-ids> |
| What the artifact contains | <https://handyutils.github.io/sctxx/#artifact> |
| Why you can trust it | <https://handyutils.github.io/sctxx/#trust> |
| LLM backends | <https://handyutils.github.io/sctxx/#backends> |
| Command reference | <https://handyutils.github.io/sctxx/#commands> |
| Worked examples | <https://handyutils.github.io/sctxx/#workflows> |
| Troubleshooting and exit codes | <https://handyutils.github.io/sctxx/#troubleshooting> |

**In this repository**

| Resource | Path |
|---|---|
| Architecture spec | [`docs/SCTXX-SPEC.md`](docs/SCTXX-SPEC.md) |
| Roadmap | [`docs/SCTXX-ROADMAP.md`](docs/SCTXX-ROADMAP.md) |
| Milestone ledger | [`specs/`](specs/) |
| Development log | [`docs/DEVELOPMENT-LOG.md`](docs/DEVELOPMENT-LOG.md) |
| **Vendored Codex manifest, file by file** | [`src/vendor/codex/README.md`](src/vendor/codex/README.md) |
| Codex compaction research | [`specs/006-m2-codex-adapter/research.md`](specs/006-m2-codex-adapter/research.md) |
| ADR: what is reused from Codex | [`docs/adr/0002-codex-compaction-algorithm-reuse.md`](docs/adr/0002-codex-compaction-algorithm-reuse.md) |
| Agent Skill source | [`skill/SKILL.md`](skill/SKILL.md) |

**Elsewhere:** [npm](https://www.npmjs.com/package/sctxx) ·
[crates.io](https://crates.io/crates/sctxx) ·
[Releases](https://github.com/handyutils/sctxx/releases) ·
[Issues](https://github.com/handyutils/sctxx/issues) ·
[the upstream we port from](https://github.com/openai/codex)

## Status

v0.3.0. The CLI, exit codes, and the `sctxx.handoff/v1`, `ops.v1`, and `state.v1` schemas are
contracts. The Rust library surface is public but unstable before 1.0.

Not yet built, and tracked in the roadmap: the probe loop and `sctxx eval` (M5), cache and
resume, incremental updates, host mode, and an MCP server (M6).

## Licence and provenance

Apache-2.0. Includes code derived from [OpenAI Codex](https://github.com/openai/codex)
(Apache-2.0) at commit `818f1cc`: UTF-8-safe truncation, secret redaction, evidence tiering
budgeting, rollback-aware replay, and the `apply_patch` header grammar. Each ported file carries
its attribution header; [`src/vendor/codex/README.md`](src/vendor/codex/README.md) is the
manifest. sctxx is not affiliated with or endorsed by OpenAI or Anthropic.

The Claude Code adapter is clean-room: written from on-disk session files, public documentation,
and contributed fixtures only.
