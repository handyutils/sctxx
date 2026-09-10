# SCTXX Hybrid Engineering Methodology

> **Instruction to a coding agent:** Read this file and follow it for the sctxx repository. It is the
> ACRYL hybrid engineering methodology applied to a Rust CLI. Where `AGENTS.md` or
> `.specify/memory/constitution.md` is stricter, they win.

## 1. Operating model

sctxx is built by human and coding-agent teams (Claude Code, Codex, Pi, and others) with the same toolset
as ACRYL:

- **GitHub Spec Kit** for durable feature ledgers and task generation (`/speckit-*` skills);
- **Wayfinder** (Matt Pocock) for decisions while the route is still unclear;
- **Matt Pocock spec-driven delivery** (`to-spec`, `to-tickets`, `triage`): clear requirements, small tasks,
  review, evidence;
- **Superpowers** process skills: brainstorming, systematic debugging, TDD, verification before completion;
- **ownership discipline** (ACRYL's Cordis lifecycle rules, translated to Rust ownership in §7);
- **Ponytail minimalism**: the simplest correct root-cause change, no speculative infrastructure;
- vertical slices, focused commits, and repository artifacts as durable memory.

```text
docs/SCTXX-ROADMAP.md            gives direction.
specs/<NNN-slug>/                gives task truth.
Tests, fixtures, evidence        give behavioral proof.
Git history + DEVELOPMENT-LOG    give recoverable checkpoints.
Chat and agent transcripts       coordinate work but are not project memory.
```

sctxx itself turns transcripts into handoffs. Those handoffs are a warm start for the next agent, never a
substitute for the ledger, source, tests, evidence, or git history.

## 2. Sources of truth

| Artifact | Role | Changes when |
|---|---|---|
| `.specify/memory/constitution.md` | binding laws | amended by decision (§17) |
| `docs/SCTXX-ROADMAP.md` | direction, milestones, invariants, invalidation conditions | product direction changes |
| `docs/SPEC.md` | architecture reference cited by section (§n) | a decision changes the design |
| `specs/000-wayfinding/` | Wayfinder map and decision tickets | a foggy question is asked or resolved |
| `specs/<NNN-slug>/` | one bounded feature block: spec, research, plan, tasks, evidence | during delivery |
| `docs/adr/` | architecture decision records, created lazily | a significant decision is made |
| `docs/DEVELOPMENT-LOG.md` | important evolutions with full commit SHAs | after an important commit |
| `tests/fixtures/` | provider format contracts | a provider format is observed |

The roadmap is not a task tracker. Never check implementation items there.

### 2.1 Feature ledger layout

```text
specs/<NNN-slug>/
  spec.md             user value, requirements, acceptance            (/speckit-specify)
  research.md         verified facts, decisions, alternatives
  plan.md             boundaries, ownership, test strategy            (/speckit-plan)
  tasks.md            dependency-ordered executable work              (/speckit-tasks)
  checklists/         requirement and acceptance checklists           (/speckit-checklist)
  contracts/          CLI flags, exit codes, JSON schemas touched by this block
  data-model.md       IR/state entities and transitions, when the block changes them
  quickstart.md       runnable acceptance commands
  evidence/T###.md    RED/GREEN/gate output, commit SHA
  invalidated.md      only if the block is deliberately stopped
  issues/NN-slug.md   only if `/to-tickets` slices the block
```

Only create artifacts that help the block. Block folders are numbered sequentially
(`.specify/init-options.json` → `feature_numbering: sequential`). `.specify/feature.json` names the active
block.

### 2.2 Feature lifecycle

```text
new block:      specify -> clarify -> plan -> tasks -> analyze -> implement -> converge
existing block: clarify -> plan -> tasks -> analyze -> implement -> converge
```

Milestone blocks listed in the roadmap exist as stub `spec.md` files marked `Status: Stub`. Before running
`/speckit-specify` for a stub, point `.specify/feature.json` at that folder so Spec Kit fills it instead of
creating a parallel folder. A stub is never approved scope.

Block states: **Stub** (named, not specified) · **Draft** · **Active** · **Completed** (all tasks checked,
evidence present, converge clean) · **Superseded** (successor named) · **Invalidated** (evidence proved the
goal or route wrong) · **Archived**. Put the state on the `**Status**:` line of `spec.md`.

## 3. Readiness before work

Read `AGENTS.md`, the constitution, the active ledger, and the source paths it names. Then write:

```markdown
## Readiness

- Milestone: M# - <name>
- Ledger block: specs/<NNN-slug>/
- Task: T### - <name>
- Rigor class: tiny | normal | architectural | high risk
- Why needed: <user or architectural outcome>
- Spec refs: docs/SPEC.md §<n>
- Scope: <exact paths; paths explicitly out of scope>
- Fixtures: <tests/fixtures/... used or to be added>
- Feedback loop: `<exact cargo test filter or command>`
- Acceptance proof: <test, snapshot, command output, or observation>
- Risks/open questions: <material items only>
```

A task is ready only when: the block is not stubbed or invalidated; `spec.md` is usable; material ambiguity
is resolved or recorded as an assumption; the proof is named; the source boundary and callers are
understood; no other agent or worktree owns the same files.

Stop and return to planning when a task changes the IR, a public schema, exit codes, the LLM egress path,
redaction, or behavior across more than one pipeline stage.

## 4. Scale rigor to the work

| Class | sctxx examples | Required process |
|---|---|---|
| Tiny | typo, doc link, clippy fix, CI cache key | understand the path, run the focused command, verify, commit |
| Normal | one adapter edge case, one ledger rule, one CLI flag, one render detail | ledger task, root-cause trace, RED-GREEN-REFACTOR, spec + standards review, evidence |
| Architectural | IR change, new adapter, new pipeline stage, schema or exit-code change, new LLM backend, cache format | Superpowers brainstorming and approved design, Spec Kit clarify/plan/tasks/analyze, vertical slice, contract updates, acceptance evidence |
| High risk | redaction and secret classes, anything sending data to an LLM, fixture intake from real sessions, vendoring or clean-room boundaries, license/NOTICE, crates.io/npm publishing, deleting cache data | architectural rigor plus explicit threat/failure analysis, rollback plan, and a human approval gate before merge or publish |

When uncertain, choose the higher class. Discoveries can upgrade a task; they never silently downgrade it.
Publishing is irreversible in practice (crates.io versions can only be yanked; npm versions cannot be
reused), so every publish is high risk.

## 5. Specify user value first

`spec.md` states what users of sctxx need and how success is recognized. Users are both humans and
receiving agents. A good spec has: actors and goals; prioritized independent scenarios; testable functional
requirements; measurable success criteria; non-goals; entities and state transitions when the IR or fold
state changes; edge cases (malformed lines, huge sessions, rewinds, missing repo, no LLM backend, secrets in
input); assumptions and dependencies (provider version, upstream commit); acceptance scenarios a reviewer
can run.

Use `/speckit-specify` and run its requirements checklist. Make every requirement observable through a
command, snapshot, exit code, or artifact field; otherwise remove it.

Use `/speckit-clarify` only when the answer changes user-visible behavior, the privacy or egress boundary,
the IR or state model, a public contract, acceptance tests, or task order. Ask one question at a time with a
short set of options. Record accepted answers in `spec.md`; record low-risk defaults as assumptions.

If the question is bigger than one block, open a Wayfinder ticket instead (§15).

## 6. Research separates facts from hypotheses

For every planning-relevant question, record in `research.md`:

```markdown
## Decision: <decision>

- Verified facts: <source, version or commit, date, observed behavior>
- Hypothesis or assumption: <labeled if unverified>
- Rationale: <why it fits requirements and constraints>
- Alternatives considered: <viable alternatives and rejection reason>
- Consequences: <what plan and tasks must honor>
```

Sources, in order of authority for sctxx: an observed session file from the named agent version; the pinned
upstream commit (Codex `818f1cca8ccf8899f0f4d59336baebaccf358eed`); the provider's published format docs
(Pi `session-format.md`); official product docs; executable experiments. A claim about a provider format
without a fixture or a pinned source is a hypothesis. Never use leaked Claude Code source or its forks as a
source.

## 7. Plan boundaries and ownership

`/speckit-plan` names: the module boundary (`src/ir`, `src/adapters`, `src/pipeline/*`, `src/llm`,
`src/cli`, `src/vendor/codex`); data and contract boundaries (IR, schemas, CLI flags, exit codes); existing
code to reuse; paths to create, modify, or leave unchanged; resource ownership and failure behavior; test
strategy and the acceptance command; compatibility with existing artifacts and caches.

### 7.1 Ownership and lifecycle in Rust

Every capability has one owner responsible for acquisition, configuration, state transitions, observable
output, and disposal. In sctxx the resources are: child processes (`cli:` backends), temp directories,
tokio tasks and concurrency permits, HTTP clients, open session files and zstd decoders, cache files and
locks, and stdout/stderr streams.

```text
Provides: stable function, type, or trait the rest of the crate uses.
Consumes: required inputs and optional collaborators.
Owns:     resources and where they are acquired.
Disposes: Drop order, cancellation, flushing, and what "quiescent" means.
Recovers: error mapping, retry, checkpoint, and resume behavior.
```

Rules:

- Subprocesses are spawned with kill-on-drop semantics and never outlive their owning call.
- No detached tasks: concurrent work runs in an owned `JoinSet` (or equivalent) that is awaited or
  cancelled; Ctrl-C cancels in-flight LLM calls and leaves the last fold checkpoint consistent.
- Temp directories are owned values that clean up on drop.
- One source owns each fact: provider files for events, the IR for normalized history, `FoldState` for items,
  the repo for current truth. Rendered markdown, progress output, and caches are projections.
- Depend on stable contracts (IR types, `LlmBackend`, schemas), not on a concrete provider or call order.

## 8. Dependency-ordered tasks

`/speckit-tasks` produces executable tasks. Each names exact paths, dependencies, a RED/GREEN proof, and
whether it can run in parallel:

```markdown
- [ ] T012 [US1] Resolve `logicalParentUuid` across compaction boundaries in `src/adapters/claude_code.rs`
  - Why: rewound or compacted sessions otherwise lose everything before the boundary
  - Depends on: T010
  - RED/GREEN proof: `cargo test --all-features adapters::claude_code::compact_boundary`
  - Acceptance: snapshot of active indices for `tests/fixtures/claude/compact-boundary.jsonl`
```

Order: fixtures and failing tests → foundational types → user stories by priority → contracts and
generated files (`cargo xtask gen-schemas`, `gen-skill`) → docs. Avoid parallel tasks that edit the same file.
A task is complete only after evidence and a focused commit exist.

## 9. Vertical-slice loop

```text
Task -> read the actual path -> RED -> GREEN -> REFACTOR -> gate -> spec + standards review
     -> evidence -> focused commit -> check task in tasks.md -> devlog if important
```

Start every progress turn with:

```markdown
- Milestone: M# - <name>
- Task: T### - <name>
- Why needed: <one sentence>
- Feedback loop: `<command>`
```

A slice crosses only the layers needed for one observable behavior: for example, one provider line type
→ IR → one ledger rule → one rendered line → one CLI snapshot. Do not build all adapters, all ledgers, or
all backends before one path works end to end. M1 is this rule applied to the whole product.

## 10. Test-driven development

For every behavior change:

1. **RED:** write the smallest test that proves the behavior. Prefer a fixture plus an insta snapshot for
   adapters and rendering, a unit test for ledgers and validation, `assert_cmd` for CLI contracts, proptest
   for truncation and budgets.
2. Run it and observe the *expected* failure (a missing behavior, not a compile or fixture-path mistake).
3. **GREEN:** smallest correct change.
4. Run the focused test and observe success.
5. **REFACTOR** while green.
6. Run the gate: `cargo fmt --all -- --check && cargo clippy --all-targets --all-features -- -D warnings &&
   cargo test --all-features && cargo test --no-default-features`.

Use real code and real files. The only mocked boundary is the LLM (`llm::mock` record/replay); tests never
touch the network, real LLMs, or real `~/.claude`, `~/.codex`, `~/.pi` stores.

Required coverage by area:

- **Adapters:** malformed lines, unknown types, rewinds/rollbacks, compaction boundaries, sidechains,
  `.zst`, empty sessions.
- **Fold:** each validation rule rejects its violating op; supersede/resolve/merge semantics; repair turn
  path; budget overflow.
- **Redaction:** every secret class in spec §10.2, applied before egress and on outputs.
- **Lifecycle:** subprocess killed on cancel, temp dir removed, checkpoint consistent after interruption.

Snapshot changes are reviewed line by line and justified in the evidence file; never accept in bulk.

## 11. Systematic debugging

Never patch a symptom first.

1. Read the full error, backtrace (`RUST_BACKTRACE=1`), and diagnostics output.
2. Reproduce with the smallest fixture; if the input is a real session, redact it first.
3. Check recent commits, provider version, feature flags, and toolchain.
4. Trace the bad value backward: provider line → IR event → active branch → ledger → row → op → render.
5. Compare with the pinned upstream behavior or the provider's documented format.
6. State one falsifiable hypothesis; add the smallest failing test that distinguishes it.
7. Fix the shared root cause (usually the adapter or IR), not each downstream symptom.
8. Verify the original reproduction and the full gate.

A third failed independent fix is an architecture signal: stop, reassess the boundary or premise, and
record the decision (Wayfinder ticket or ADR) before continuing.

## 12. Ponytail minimalism

After understanding the flow, climb this ladder and stop at the first rung that satisfies acceptance:

1. The behavior may not need to exist yet. Check the roadmap milestone and skip speculative work.
2. Reuse existing crate code or a vendored Codex helper.
3. Use `std`.
4. Use an OS capability.
5. Use an already-present dependency.
6. Write the smallest direct code.
7. Add a dependency, trait, or abstraction only when the above fail.

For sctxx: a trait needs at least two real implementations (adapters and `LlmBackend` qualify); no plugin
system, no generic config layers, no caches or feature flags beyond the spec until a measured need exists.
Minimalism never removes redaction, input validation, error handling that prevents data loss, the
clean-room boundary, or a rollback path. Record a deliberate shortcut with a `// ponytail:` comment naming
its ceiling and upgrade trigger.

## 13. Verification and review

Never claim done, fixed, or passing without fresh command output.

1. Map each acceptance claim to a command or observation.
2. Run the focused test, then the gate from §10.
3. Read exit status and the whole failure output.
4. Reproduce the original defect or user flow (for handoff work: run `extract` and read the artifact).
5. Review the diff twice:
   - **Spec review:** does it deliver the block's scenarios and acceptance, with no unapproved scope?
   - **Standards review:** constitution, `AGENTS.md` hard rules, ownership, redaction, contracts, tests,
     vendoring headers, generated files.
6. Record evidence (§14).

Use `/speckit-analyze` before implementing when artifacts may be inconsistent. Use `/speckit-converge`
after implementing; if it finds specified-but-unbuilt work, the block is not done.

Performance acceptance measured on the M1 Max records the host in evidence
(`rustc -vV | grep host`, release build, file size). CI proves the other platforms.

## 14. Focused commits and evidence

- Inspect `git status`; stage explicit paths only.
- Keep separate: behavior changes, vendoring updates, dependency upgrades, generated files, and refactors.
- Commit message: Conventional Commits with a module scope (`feat(adapters/codex): replay ThreadRolledBack`).
- Until v0.1.0 is public (roadmap M4), work directly on `main` with focused commits. From M4 on, outside
  contributions arrive as pull requests with the clean-room confirmation.
- Record important product, architecture, workflow, or operational evolutions in `docs/DEVELOPMENT-LOG.md`
  in a separate documentation commit that cites full commit SHAs.

Evidence file `specs/<NNN-slug>/evidence/T###.md`:

```markdown
# Evidence: T### <task name>

- Commit: <full SHA>
- RED: `<command>` -> <expected failure>
- GREEN: `<command>` -> pass
- Gate: `<gate command>` -> pass
- Snapshots: <changed snapshot files and why>
- Acceptance observation: <artifact excerpt, CLI output, or N/A>
- Host (perf only): <host triple, build profile, input size>
- Clean-room (adapter tasks): Claude Code adapter work used only fixtures and public docs
- Residual risk: <none or specific limitation>
```

Never paste unredacted session content into evidence.

## 15. Scope control, discoveries, and Wayfinder

Classify new work before acting:

- **Required correction:** needed for this task's acceptance or safety; add it only if small and coupled.
- **New prerequisite:** add a dependency task to the active `tasks.md` and stop until planned.
- **Adjacent improvement:** record as a candidate block or ticket; do not grow the slice.
- **Open question:** too big for `clarify` → Wayfinder ticket in `specs/000-wayfinding/issues/`.
- **Roadmap change:** affects direction, milestone order, or invariants → update the roadmap only after an
  explicit decision.
- **Invalidation:** contradicts an active block's assumptions → mark it invalidated or superseded before
  building around the contradiction.

Wayfinder tickets decide; they do not build. Claim before working, resolve by appending `## Answer`, and add
a one-line gist to the map's "Decisions so far". Tickets of type `grilling` are answered by the user, never
by the agent.

## 16. Agent startup, coordination, and handoff

An implementation agent receives a readiness packet (§3) plus: constitution and invariants, inspected paths
and callers, working-tree and worktree ownership, commands already run with results, deliverables, and the
verification command. It owns only its scope, does not merge its own work, and never discards unrelated
changes.

At task end or handoff, write a durable result into the ledger (evidence file or `tasks.md` note):

```markdown
## Handoff / Result

- Objective and task ID
- Completed behavior
- Changed files
- Tests and command output summary
- Commit hashes
- Decisions and assumptions
- Open problems and risks
- Exact next recommended task
```

Dogfooding: once `sctxx extract` works, an agent resuming this repo may generate `.sctxx/handoff.md` from
the previous session as a warm start. Then it still reads the ledger, because the ledger is the truth.

## 17. Methodology self-evolution

Change this method only with evidence: name the failure mode or waste, choose the smallest process change,
record rationale/alternative/consequence (ADR or constitution amendment), trial it on one block, and keep it
only if it improves correctness, speed, or clarity. Remove steps that no longer change behavior.

## 18. Anti-patterns

- Treating chat, scrollback, or a sctxx handoff as project memory.
- Coding before the block has a task and a proof; treating a stub or unfilled template as a plan.
- Building every adapter or backend before one slice works end to end.
- Tests written after the implementation without observed RED; accepting snapshots in bulk.
- Claiming completion from reading code instead of running it.
- Fixing downstream symptoms of an adapter or IR bug.
- Detached tasks, orphaned subprocesses, temp files without an owner.
- A second IR, a second config system, or a parallel artifact format beside the existing one.
- Speculative traits, plugin systems, settings, caches, or dependencies.
- Reading or committing real session content without redaction.
- Consulting leaked Claude Code source or its forks.
- Leaving superseded blocks apparently active; massive commits; destructive git commands to make status
  look clean.

## 19. Templates

### 19.1 Stub block (created from the roadmap)

```markdown
# Feature Specification: <milestone> - <name>

**Feature Branch**: `<NNN-slug>`
**Created**: <YYYY-MM-DD>
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M#; docs/SPEC.md §<n>

## Scope seed
- <bullets copied from the roadmap block>

## Unlocked by
- <Wayfinder ticket title and path, or "no open ticket">

Fill with `/speckit-specify` after pointing `.specify/feature.json` at this folder.
```

### 19.2 Research record

```markdown
# Research: <topic>

## Decision
<Chosen direction>

## Verified facts
- <Fact> - Source: <fixture path + agent version | upstream commit | doc URL + date | command output>

## Assumptions
- <Unverified but accepted working assumption>

## Alternatives considered
- <Alternative> - Rejected because <reason>.

## Consequences
- <Constraint later plan/tasks must honor>.
```

### 19.3 Plan item

```markdown
## Design: <capability>

- Module: <src/... path>
- Provides: <types, functions, CLI surface, schema>
- Consumes: <inputs and collaborators>
- State: <durable facts vs projections>
- Owns/disposes: <subprocesses, temp dirs, tasks, files; drop and cancel behavior>
- Failure/recovery: <error mapping, exit code, checkpoint/resume>
- Contracts touched: <flags, exit codes, schemas + version impact>
- Verification: <focused test and acceptance command>
```

### 19.4 Invalidated block

```markdown
# Feature invalidated: <name>

- Status: invalidated
- Date: <YYYY-MM-DD>
- Decision: <why work must not continue>
- Evidence: <fixture, test, measurement, user decision, or architectural finding>
- Successor: <spec path or none>
- Preservation: <archive, remove, or retain for reference>
```

### 19.5 Completion report

```markdown
## Completion

- Milestone: M# - <name>
- Task: T### - <name>
- Delivered: <observable behavior>
- Evidence: specs/<NNN-slug>/evidence/T###.md
- Commit: <full SHA>
- Residual risk: <none or specific limitation>
- Next: <next task, why, feedback loop>
```

## 20. Final execution rule

```text
Understand -> readiness -> RED -> GREEN -> REFACTOR -> gate -> review
-> evidence -> focused commit -> update ledger -> handoff -> next task
```

When the repository is silent, choose the smallest reversible change that preserves authoritative state,
explicit ownership, private data, and a runnable proof.
