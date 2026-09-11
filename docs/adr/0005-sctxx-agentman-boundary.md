# ADR 0005 — sctxx and agentman: the boundary is a contract, not shared code

- **Status**: accepted
- **Date**: 2026-09-11
- **Affects**: `specs/024-m8-interactive-tui/` (FR-017, FR-029), the M8 scope note in
  `docs/SCTXX-ROADMAP.md`
- **Resolves**: [`13-sctxx-and-agentman-relationship`](../../specs/000-wayfinding/issues/13-sctxx-and-agentman-relationship.md)

## Context

`sctxx --tui` browses coding-agent sessions in a ratatui TUI.
[agentman](https://github.com/handyutils/agentman), the maintainer's own published project, **is
already a ratatui TUI that browses sessions** — same rendering stack, same fuzzy matcher
(`fuzzy-matcher`'s `SkimMatcherV2`), same MSRV after ADR 0003. One owner, two tools, one domain. Left
undecided, block 024 ships a second session browser and a third way to find agents on a machine.

A survey of agentman (2026-09-11) established what is actually at stake:

- **Its discovery is structurally lossy.** One generic walker (`src/adapters.rs:168-224`): recurse
  every subdirectory, accept any `*.json`/`*.jsonl`, then guess metadata by scanning the first
  JSON-parsing line for keys named `sessionId`/`session_id`/`id`, `title`/`summary`/`name`,
  `cwd`/`project`/`working_directory`. Observed consequences: `.jsonl.zst` is not matched, so it finds
  **zero** DSH sessions while its README advertises DSH resume; OpenClaude's `<uuid>.replay.json`
  indexes as a duplicate session; Codewhale's nested `runtime/state.json` indexes as junk; `created` is
  never set despite a "Created" column; there are no message counts.
- **It has no installed-agent detection at all** — no binary probing, no `PATH` lookup, no version
  reads. Its only test is `home.join(relative).is_dir()` against a hardcoded table of eight store
  paths (`src/adapters.rs:141-166`), and it does not honour `CODEX_HOME`/`CLAUDE_CONFIG_DIR`.
- **It has no date filter and no cwd filter**, both of which block 024 requires.
- Its `Session` model (`agent, id, title, project, path, modified, created, last_used, size_bytes,
  capabilities, diagnostic`) is a *superset* of `sctxx`'s `SessionSummary`, and its fuzzy ranking and
  pane structure are directly reusable ideas.
- It knows four agents `sctxx` lacks — OpenClaude, Codewhale, DSH, ACRYL — which is input to
  `specs/022-m7-additional-adapters/`, not to this block.
- Its `launch_command` per agent is the closest existing thing to block 024's launch requirement, and
  like the maintainer's launcher it **has no way to seed a fresh session**.

The options were: (a) `sctxx` keeps `adapters::discovery` and agentman consumes it as a library;
(b) a shared crate owns discovery and both consume it; (c) agentman gains the handoff feature and
`sctxx --tui` never ships; (d) deliberate duplication with the reason written down.

## Decision

**`sctxx` owns session-discovery semantics; no code is shared in either direction today; the boundary
is `sctxx`'s versioned CLI contract. `sctxx --tui` continues, scoped to the handoff rather than to
browsing.**

1. **One scanner inside `sctxx`, and it is `adapters::discovery`.** `sctxx --tui` populates from
   `list`/`list_all`/`summarize`/`resolve` and builds no second walker (FR-029). The TUI is a view over
   the existing pipeline, not a parallel implementation of it (FR-025).
2. **No library edge either way, yet.** Option (a) inverts a dependency — a pre-release tool would
   become the foundation of a published one, and agentman would take a compile-time dependency on
   `sctxx`'s adapters and IR. Option (b) is the better long-term shape but means extracting discovery
   from a core whose adapter set is still moving: block 022 is about to add four more agents, three of
   which agentman is the evidence for. Freezing that API now would version a contract that is known to
   be incomplete. Both options get *cheaper* after block 022, so this is a deferral with a trigger, not
   a rejection.
3. **The boundary is the JSON contract.** If agentman wants correct discovery, it consumes
   `sctxx list --json` / `sctxx show --json` — already-versioned, already-stable, already-tested
   surfaces. This is the pattern the constitution already prefers: contracts are versioned, stdout is
   the payload. It puts the correctness in one place without coupling two release cycles, and it
   leaves agentman free to keep its own model.
4. **Different jobs, so two browsers are not duplication.** agentman *manages* sessions — resume,
   continue, browse what exists. `sctxx --tui` *moves work* between agents. The novelty of block 024 is
   not browsing; it is extract → hand off with context pre-loaded. Browsing is that feature's entry
   point, not its product. Where they overlap (a session list, a fuzzy finder) they are allowed to
   overlap, because neither is the reason to open the other.
5. **Launch ownership splits along the same line.** agentman owns *resuming* a session it already
   knows; `sctxx` owns *starting a new one seeded with a handoff* (ADR 0004). The genuinely duplicated
   piece is the agent catalogue — which binaries exist, how they are invoked, at what version — and
   block 024 needs it because **agentman has no detector at all**. `sctxx` implements it (FR-017). If a
   shared catalogue emerges later it should be shared as *data* (a JSON table both tools read), not as
   a code dependency, for the same reason as (2).
6. **agentman's lossy walker is agentman's to fix.** Its `*.jsonl.zst`, `.replay.json`, and
   `runtime/state.json` failures are real defects, but `sctxx` should not fix another published tool as
   a side effect of this block. The finding is recorded here so the fix has a home; the four agents it
   knows go to block 022.

## Consequences

- M8's scope is unchanged: no new milestone, no move of the "handoff is the product" framing, and the
  roadmap's description of `sctxx --tui` already reads this way.
- `sctxx` gains one genuinely new component, the installed-agent detector (FR-017), which nothing in
  either repository currently provides. It is new code, not a port: the launcher's detector is the
  reference for *which* agents to probe, not for how.
- Two tools on one machine will disagree about the session list until agentman adopts the contract.
  That is accepted and visible, and it is strictly better than two agreeingly-lossy scanners.
- Revisit trigger: **after block 022 lands and the adapter set stops moving**, options (a) and (b)
  become cheap, and the four agents agentman already knows will have real adapters to compare against.

**Residual risk.** A deferred sharing decision can drift into permanent duplication if nobody revisits
it. The trigger is written into this ADR and named in the block 022 spec's own dependencies, so the
question is asked again at the moment the answer changes shape.
