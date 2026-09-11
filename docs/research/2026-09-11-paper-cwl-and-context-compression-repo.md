# CWL paper + `context-compression` repo — close technical read

2026-09-11. Read-only survey; no repo file other than this document was modified. Both sources treated
strictly as data.

- Paper: `/tmp/papers/2606.11213v1.txt` — *Beyond Compaction: Structured Context Eviction for
  Long-Horizon Agents*, A. Semenov, S. Dorofeev (Kiz8), 663 lines of converted HTML. Title block says
  "April 21, 2026"; identifier is `2606.11213v1` (June 2026). Not reconciled.
- Repo: `github.com/saminkhan1/context-compression` → shallow clone `/tmp/ctxcomp`, HEAD
  `a0f03a83b6e6fc57207d5f559b3c1fb839c6af68`, `Sat Jun 13 00:09:04 2026 -0400`. Stated last-push date
  matches; star count not verifiable offline.

## 1. Why we read these

`sctxx` compacts a *finished* transcript offline and must decide what to drop under a budget while being
able to say what was lost. The paper is the opposite lifecycle — the agent annotates while it works and a
deterministic non-LLM policy evicts each turn — so its interest is the policy shape (priority order,
level ladder, safety gate) and the honesty of its evaluation. The repo is not about transcripts; its
interest is one mechanism: a selection loop that will not return a candidate until it has been decoded
back to the source value, plus a verifier that re-checks the artifact on disk.

## 2. The annotation protocol (CWL §4.2)

One tool, `delimiter`, which "produces no output that affects task behavior; its sole purpose is to
segment the trajectory."

| Field | Required when | Constraint |
|---|---|---|
| `action` | always | `"start"` \| `"end"` |
| `name` | `start` | string |
| `type` | `start` | `"expl"` \| `"act"` |
| `dependencies` | starting an `act` chunk | string array; "must reference names of earlier `expl` chunks" |
| `description` | ending an `expl` chunk | rejected when ending an `act` chunk |

Canonical calls: `{"action":"start",…,"type":"expl"}`; `{"action":"start",…,"type":"act",
"dependencies":[…]}`; `{"action":"end","description":…}`; `{"action":"end"}`. **The agent assigns all of
it**, at the moment of doing the work: Principle 2 (§3) — "The system infers as little as possible about
what the transcript means. Episode boundaries, episode types, and dependencies are declared explicitly by
the agent using dedicated tools." Dependencies may name only *already-closed* expl episodes (§4.3 inv. 1),
which is what makes the graph acyclic.

**Typed** = exactly one of two values, and the type decides what the episode holds and when it dies.
`expl` gathers information (search results, directory listings, orienting file reads); its raw content "is
typically not needed once that inference is made"; on close the agent writes a `description`, and §4.2
says "this description is the only content retained after full eviction." `act` takes action (writes,
edits, tool calls); "Their effects are persisted in the environment … making them the first candidates for
eviction." **Dependency-linked** = an `act` declares, by name, the closed `expl` episodes whose information
it is consuming; §4.2: "This encodes the fact that the action episode's correctness relies on the
exploratory one having happened." Expl episodes never declare dependencies ("exploration is the frontier
and has nothing behind it"). The protocol carries no per-tool-call identity and no declaration for content
outside an episode (U7).

## 3. The episode graph (CWL §4.3)

`G = (V, E)`; vertices are episodes; "an edge `(u,v) ∈ E` indicates that action episode `v` declared a
dependency on exploratory episode `u`." A dependency edge therefore asserts: *v's correctness presupposes
that u happened, and u's information is what v consumed.* The graph is "append-only during normal
operation; compression removes nodes but never alters edges." Invariants "maintained at all times":
**(1) Acyclicity** — dependencies reference only closed episodes. **(2) Typed edges** — "All edges go from
exploratory to action episodes. Action-to-action and exploratory-to-exploratory dependencies are not
expressible in the protocol." Unstated consequence: `act` vertices have no outgoing edges, so the gate
"all dependents fully evicted" is **vacuously true for every `act`** — it can only ever gate an `expl`, so
the graph constrains exactly one decision. **(3) Prologue protection** — "Content predating the first
delimiter `start` call — the system prompt, tool definitions, and any initial user turns — is treated as a
protected prologue and is never part of the graph. It is not eligible for eviction under any
circumstances." Fourth state: active episodes (started, not ended) "are never eligible for eviction."

## 4. The eviction policy (CWL §4.4 + Algorithm 1)

Trigger: "When token accounting reports that the current context exceeds the configured threshold, the
eviction policy runs"; in the implementation (§6.1) it "runs the eviction policy on every turn after token
accounting."

```
while countTokens(episodeGraph) > tokenBudget:
  candidates = closed, non-prologue, non-active episodes whose dependents are all fully evicted
  if candidates is empty: break                 # nothing safe left; budget cannot be met
  target = oldest ACT in candidates, else oldest EXPL
  for level in [STRIP_REASONING, STRIP_BULK_OUTPUT, STRIP_INTERMEDIATE, REMOVE_EPISODE]:
    if level is applicable to target.type:
      strip target at level
      if countTokens(episodeGraph) <= tokenBudget: return     # budget satisfied
```

Levels, verbatim (§4.4): **1 reasoning trace stripping** — "exploratory episodes only"; extended CoT
removed first; "often an insignificant fraction of an exploratory episode's token footprint." **2
bulk-output stripping** — "Large, enumerable tool outputs — search results, directory listings (like grep
and glob) — are removed entirely." **3 intermediate artifact stripping** — "Smaller tool interactions —
file reads, bash commands and their outputs — are removed entirely." **4 full episode removal** — "If
stripping is exhausted and the budget is still exceeded, the episode is removed in its entirety."

Always preserved: user turns ("preserved exactly", Principle 3), the prologue, active episodes; if the
budget cannot be met without touching user turns, "the system surfaces the condition rather than silently
degrading." Evicted first: the **oldest eligible `act`** (levels 2→3→4), and only when no `act` exists the
oldest eligible `expl` (levels 1→2→3→4) — justified by recoverability from the environment. Safety gate
(§4.4): "An exploratory episode `u` is only eligible for eviction if every action episode `v` with
`(u,v) ∈ E` has itself already been fully evicted. This prevents the situation in which an exploratory
episode is dropped while an action episode that depended on it is still in context." Exit on budget
satisfied or `candidates.isEmpty()`.

**Underspecified steps — each blocks a faithful implementation:**

| # | Gap |
|---|---|
| U1 | **No classifier maps content to level.** Levels 2–3 are given only by example ("grep and glob" vs "file reads, bash commands"); no rule assigns a tool call or message to one or the other, and assistant text / tool arguments are unaddressed. |
| U2 | **Applicability of levels 2–4 by type is unstated.** Only level 1 is "exploratory episodes only", yet line 18 gates *every* level on `applicable to target.type`. |
| U3 | **Non-termination under the stated predicate.** No `not episode.isFullyEvicted` clause exists. An `expl` reduced to its retained `description` is still closed, non-prologue, non-active and (if its dependents are gone) eligible; no level removes a description, so `countTokens` is unchanged and the loop spins. §4.4 promises an exit when "no further evictions are possible", which the pseudocode cannot express. |
| U4 | **The retained `description` is never accounted for**: counted in `countTokens`? evictable? what happens when only descriptions remain and the budget is still exceeded? It is the terminal state of every `expl`. |
| U5 | **`countTokens` is undefined**: tokenizer, prompt framing, and whether the whole graph is re-tokenized every iteration (implied O(n²) per pass). §5 concedes only that traversal is "non-negligible." |
| U6 | **Budget scope undefined**: full window or input-minus-output? Does `countTokens(episodeGraph)` include prologue and user turns? Principle 3 implies a floor, never stated. |
| U7 | **Content outside every episode is unhandled.** §4.3 starts the graph at the first `start` call; §5 says "unannotated content is treated as exploratory-with-unknown-dependencies, preserving it until action episodes are exhausted" — but such content is not a vertex, so it is never in `candidates` and never evictable. The two statements do not compose. |
| U8 | **"Oldest" is undefined** (episode start vs close). |
| U9 | **Name uniqueness / referential integrity unstated**: duplicate names, unbounded names, dependencies on already-evicted episodes. |
| U10 | **Threshold vs budget and check timing** are described only for the implementation, not defined by the policy. |

## 5. Empirical results and their limits (CWL §6, §5, §7)

Setup (§6.1): a fork of `pi.dev` (`github.com/Kiz8-Team/pi-cwl`) adding the `delimiter` tool, an
in-process graph and per-turn eviction. All experiments use **GPT-5.4**. Benchmarks (§6.2): Terminal
Bench 2.0 (89 tasks), SWE Bench Lite (random 50 of 300), Recovery Bench, LongCLI Bench. The protocol is
**deliberately asymmetric**: baseline = per-task isolated session; CWL = *all tasks sequentially in one
session* under a fixed **80,000-token budget**. The paper calls this "the central point of evaluation" and
says comparisons to leaderboards "are therefore not apples-to-apples." Mean accuracy (§6.3), `n=5` for
Terminal Bench 2.0 and `n=3` otherwise:

| Benchmark | CWL (one session, 80k) | Baseline (isolated) |
|---|---:|---:|
| Terminal Bench 2.0 | 68.25% | 68.40% |
| SWE Bench Lite (50-task sample) | 43.00% | 40.00% |
| Recovery Bench | 66.80% | 69.75% |
| LongCLI Bench | 20.00% | 20.00% |

"Across all four benchmarks, the CWL and baseline conditions differ by at most 3 percentage points in
either direction. These margins are within run-to-run variance and should not be interpreted as
directional signal." No variance, no confidence interval, no per-task breakdown: non-inferiority without
inferential backing. Tokens/cost: shorter suites showed "little to no difference"; Terminal Bench 2.0's
89-task run "processed over 80 million tokens across the full sequence at a total inference cost of
approximately **$55 per complete run**" — 80M is *total tokens processed across the session*, not context
size. Cost reduction **20–70%** vs uncapped sessions (§5, §6.3), attributed to task structure (iterative
code editing benefits most, variable tool output least).

Budget sensitivity (§6.4): τ > 120,000 → "sharp increase in inference cost with no corresponding accuracy
improvement"; τ ≈ 50,000 → "up to 3×" cheaper than 120k+ with no measurable degradation, but wall-clock
per task "increased by up to 2×", the trace showing eviction of still-relevant exploratory content
followed by agent re-exploration; therefore τ ∈ [80,000, 120,000] is "near a Pareto frontier." This is in
tension with §5, which argues cost and quality *both* favour a lower τ ("a tighter budget is
simultaneously cheaper and qualitatively better … argues for erring toward a lower value") — the 80k floor
exists only because of the 50k latency penalty.

Stated limitations (§7, plus §5): **dependency granularity** (whole episodes, not tool calls — "a
deliberate simplification"); **non-linear trajectories** (the design "assumes a single linear stream of
episodes"); **effect on model reasoning behaviour** — "a range of behavioral changes that we were unable
to conclusively attribute to CWL": rushed exploration, over-exploration, looping, tentatively attributed
to "mild confusion introduced by the annotation protocol." Plus §5: annotation burden at every boundary;
dependence on annotation quality (a mis-typed episode causes premature eviction of live context; an
omitted dependency "may [cause] a needed exploratory episode [to be] evicted while the action episode that
relied on it remains"); graph traversal overhead; and **KV-cache invalidation** — "Every eviction
therefore invalidates the cached KV state for the affected prefix and all content that follows it", worst
case net-negative versus compaction, which produces one stable prefix per pass.

## 6. The repo: `saminkhan1/context-compression`

**6.1 What it actually does.** A **format selector for structured data files**, not a transcript
compactor. For one file it parses the value, generates a fixed set of re-encodings, keeps only those that
decode back to that value, and returns the fewest-token one, where the token count **includes the decoder
instruction line the agent must read**. It never rewrites the source; it writes a sidecar under
`.codex/context-cache/` and hands back the sidecar path. Extensions `.json`/`.jsonl`/`.csv`/`.tsv`
(`hook.py:32`). Tiers (`hook.py:33-37`): `safe` = {raw, compact-json, column-json, csv, tsv}; `advanced`
adds {codebook-json, typed-csv, typed-tsv}. Runtime default `safe`; the benchmark used `advanced`.

**6.2 The selection algorithm.** `choose_best` (`hook.py:455`) is the contract:

```python
ranked = sorted(((candidate_token_metrics(c, model_profile), c) for c in candidates),
                key=lambda item: candidate_rank_key(item[1], item[0]))
for metrics, candidate in ranked:
    if candidate_matches_source(source, candidate):     # decode(candidate) == source.value
        best = candidate_with_roundtrip_note(candidate); break
else:
    raise ValueError("no reversible candidates generated")
```

Rank key (`hook.py:527`) is `(total_tokens, payload_tokens, len(text), name)` — a total order, so
selection is deterministic; `stable_headers` (`hook.py:817`) preserves first-seen key order via
`dict.fromkeys`. `total_tokens = count_tokens(instructions + "\n" + text)` (`candidate_blob`,
`hook.py:791`). The round-trip predicate (`hook.py:784`):

```python
def candidate_matches_source(source, candidate) -> bool:
    try:
        return decode_candidate_value(candidate.name, candidate.text, source.kind) == source.value
    except Exception:
        return False
```

**Lossless and reversible, and how:** losslessness is *equality of the parsed Python value*, not byte
equality, re-derived by decoding. Every encoding has a decoder (`decode_candidate_value`, `hook.py:961`):
`compact-json` → `json.loads`; `column-json` → `[headers, rows]`; `codebook-json` → `[columns, dicts,
rows]` with `dicts = [[column_index, values], …]` and integer cell codes; `csv`/`tsv` → each cell a JSON
literal (`cell_json`, `hook.py:957`); `typed-*` → a `t:<types>` header plus typed cells (`i`/`n`/`b`/`s`,
`?` nullable, `~` null). Codebook dictionaries are emitted only when they pay (`hook.py:888`). Because
equality is on the parsed value, the check cannot see loss the parser already did: JSON duplicate keys are
collapsed by `json.loads` (`raw` is itself a candidate decoding through the same parser), a BOM is stripped
by `read_text(encoding="utf-8-sig")` (`hook.py:437`), and key order/whitespace/number formatting are
normalized. So "lossless" = *the agent can reconstruct the same data*, not *the original bytes are
recoverable*. The repo is honest about this: `EVIDENCE.md` states the gate is "round-trip back to the
parsed source value", and `README.md:67` says "the repo currently proves deterministic token savings and
round-trip safety. Full answer-parity evidence across model families is still an optional eval path, not a
completed production claim."

**6.3 Invocation surfaces.** CLI (`selector.py` → a `context-selector/v1` report, `hook.py:39`); a Codex
hook (`hook.py` via `run-hook.sh`) whose `PreToolUse` rewrites exactly a whole-file `cat`
(`plain_cat_paths`, `hook.py:261`; rewrite at `hook.py:332`) and whose `UserPromptSubmit` is a no-op unless
`CONTEXT_OPTIMIZER_VISIBLE_PROMPT_INJECTION=1` (`hook.py:122`); an MCP stdio server
(`adapters/mcp/context_selector_server.py`); and thin adapters for Claude Code, Pi (TS), Hermes, OpenClaw
(TS) and a generic contract — `adapters/CONTRACT.md` requires all to trust only the verified `read_path`.
The hook aborts mid-run past `CONTEXT_OPTIMIZER_MAX_HOOK_LATENCY_MS` (default 500, `hook.py:313`) and skips
rewrites whose local cost is not repaid by provider latency (`hook.py:579`).

**6.4 Tokenizer-aware part.** `resolve_model_profile` (`hook.py:1114`) resolves the slug from the hook
payload, Codex `config.toml`, a model catalog, or the bundled `model-catalog.snapshot.json`.
`resolve_token_counter` (`hook.py:1208`) picks, in order: `tokenizers-json` (a Hugging Face
`tokenizer.json` via the `tokenizers` package) → `tiktoken` (OpenAI slugs) → `deterministic-fallback`;
`preferred_tiktoken_encoding` (`hook.py:1272`) uses `o200k_base` for `gpt-5*`/`gpt-4o`/`o1`/`o3`/`o4`, else
`cl100k_base`. The fallback (`hook.py:1300`) splits on `[A-Za-z0-9_]+` or a single punctuation char and
charges `max(1, (len(word)+3)//4)` per alphanumeric atom. Under the fallback the candidate set is
restricted to `SAFE_CANDIDATES` and counts are labelled `"estimated"` (`hook.py:195`, `:624-628`) — the
one idea directly portable to sctxx, which by `AGENTS.md` must not add tokenizer crates.

**6.5 Test suite and whether losslessness is tested.** 67 test functions across 7 files. Yes, at three
levels: (1) `tests/test_hook.py:251` `test_generated_candidates_round_trip_on_real_fixtures` asserts
`hook.candidate_matches_source` for **every** validated candidate of `sample-data.json`,
`sample-repetitive.json` and `tests/fixtures/hf-julien-c-titanic-survival.json`; (2)
`tests/test_hook.py:265`, `:274` targeted `column-json` and `codebook-json` round-trips (the latter with
nested dict values and a header containing `|`); (3) end-to-end on disk —
`verify_selector_report.validate_round_trip` (`verify_selector_report.py:247`) re-loads the source, reads
the **sidecar file**, strips the instruction line (`hook.py:797`), decodes, and compares against
`source.value`, also checking source `sha256` and sidecar `output_sha256` (`:234-242`); and
`tests/test_selector.py:194` mutates a sidecar and asserts rejection. Boundary: 3 fixtures plus the 28-file
downloaded (git-ignored) corpus; no fuzzing or property test.

**6.6 Size, dependencies, measured behaviour.** 59 files, ~7,300 lines of Python excluding an 8,872-line
JSON fixture. Core is stdlib-only (`json`, `csv`, `hashlib`, `tomllib`, `shlex`, `re`, `io`); runtime deps
`tiktoken==0.13.0` and `tokenizers==0.23.1` (optional at import, `module_available`, `hook.py:1224`);
`requirements-eval.txt` adds `inspect-ai==0.3.224`, `openai==2.37.0`. MIT. Checked-in benchmark
(`reports/benchmark-report.md`, 2026-05-22): 28 files, gpt-5.4-mini via tiktoken, advanced tier —
17,496,442 raw → 15,171,483 optimized = 2,324,959 saved (**13.3%**). Per-family winners: SQuAD 72.7%,
Titanic 50.5%, LogHub 41.5%, GitHub repo metadata 29.1%, all `codebook-json`; `hf-code-doc` only 5.2%.
The ablation is the honest part: `codebook-json` wins 20/28, `raw` wins 2, and `compact-json`/`csv`/`tsv`
are **net-negative on average** (-2.5%/-3.9%/-4.0%). Local processing cost 48,604 ms for the corpus,
giving a break-even provider input throughput of **47,834 tokens/s** — at realistic provider speeds the
local Python pass costs more latency than the token saving, which is why the runtime hook defaults to
`safe`, requires 128 saved tokens, and has the latency gate.

**6.7 Honest assessment.** It does what its title says, carefully: deterministic total ordering, decoder
cost charged to the budget, verification by decoding rather than by trusting the encoder, hashes in the
report, a verifier that re-checks artifacts on disk, and an ablation that includes its own losing
candidates. What the description does not prepare you for is that **it has nothing to do with agent
transcripts, episode structure, or budgeted eviction** — `grep -rni` over the clone returns 0 hits each
for `transcript`, `episode`, `evict`, `compaction`. It is a lossless re-encoder for tabular/text records
whose corpus-wide "compression" is 13.3%, dominated by one dictionary codebook, and net-negative for three
of its own candidate formats. The transferable asset is the verification discipline, not the algorithm.

## 7. What sctxx could adopt, ranked and concrete

`sctxx` already segments episodes/chunks (`src/pipeline/segment.rs:12`, `:25`), computes the ledgers, masks
log noise, keeps a near-verbatim tail, and has typed state including `Constraint` (`SCTXX-SPEC.md` §8.1).
Only deltas below.

**A1 — Value-level re-decode gate + hashes in the artifact (highest value).** Adopt the rule that a
"nothing was lost" claim is *re-derived by decoding*, not asserted by the component that dropped content:
after S7 render, re-derive each rendered L0/L1 line and each L2 masked row from `state.json` + IR and
require equality against the source event text/ranges; write `source_sha256`, `artifact_sha256` and
`verified: true|false` into `handoff.json`, mirroring `sha256`/`output_sha256` (`hook.py:415`, `:425`).
Where: `src/pipeline/` render + `handoff.json`. Cost: low, pure deterministic Rust, no new deps. Risk:
schema bump and snapshot churn — expose it as `sctxx verify` or a `--verify` flag if hash churn in existing
snapshots is unacceptable.

**A2 — Charge the pointer to the budget, and rank renderings instead of only truncating.** The repo charges
the decoder instruction to the candidate (`hook.py:791`). §12.2 renders "by priority order, stop adding
items when the layer budget is reached" and on overflow prints omitted item ids plus an `sctxx expand`
hint — and that hint is not charged. Where: the L0/L1 budget loop in `src/pipeline/`. Cost: low, pure Rust.
Risk: changes every budget-sensitive snapshot; needs an ADR.

**A3 — Machine-checkable evidence gate.** Adopt the pattern of a script that refuses to let a savings claim
be published without a persisted report and a passing round-trip (`scripts/verify_evidence.py`,
`benchmark.py verify-corpus`, `EVIDENCE.md`). sctxx has `sctxx eval` (§15.2) and probes (§10.3) but no
refusal contract tying a token-savings number to its evidence. Delta: `handoff.json` carries
`tokens_before`, `tokens_after`, `report_path`, `source_sha256`, `artifact_sha256`, and `sctxx eval` fails
if the artifact moved without the report. Where: `handoff.json` + `sctxx eval`. Cost: low. Risk: none
substantive.

**A4 — Label estimated budgets explicitly.** From `token_counter_label` (`selector.py:195`): when a number
that drove a decision is approximate, say so. sctxx uses the 4-bytes/token estimate by policy; the delta is
a `tokens_estimated: true` marker and the estimator's name in `handoff.json`'s header. Where: `handoff.json`
header. Cost: trivial. Risk: schema bump.

**A5 — Codebook dictionary encoding for ledger tables (observe, do not build).** The dictionary idea is the
repo's only real winner, but its own ablation shows the surrounding candidates are net-negative and the
decoder instruction is paid in the agent's context. sctxx's ledgers are small and already structured; a
codebook adds decode burden to a reader that is an agent, not a program. Defer until A3 can measure it.

## 8. What does not transfer

- **The `delimiter` annotation protocol.** sctxx reads a *finished* session file; there is no live agent to
  call `delimiter`, and `AGENTS.md` §2.3/§2.4 forbid touching the host agent's tool set. sctxx infers
  episodes deterministically at `UserMessage` boundaries plus hard boundaries (compaction summary, model
  change, successful commit, >30 min gap) — §7.3. Principle 2 says inference is exactly what to avoid, so
  the protocol is inapplicable by construction, not merely inconvenient.
- **The typed episode graph and dependency edges.** Deriving `expl`/`act` and `(u,v)` post-hoc is the
  "inferring structure post-hoc" §5 explicitly rejects, and the paper supplies no classifier (U1). Porting
  the graph requires inventing the one component the paper never gives; the `act`-vs-`expl` priority and
  dependents gate would also need a confidence model sctxx does not have.
- **The eviction loop as a budget mechanism.** Algorithm 1 is online, per-turn, in-place; sctxx's budget is
  applied once at render time over materialized rows. Its genuinely new part relative to sctxx is
  "graduated levels per unit", and sctxx already has a graduated ladder (masked row → placeholder
  `[read src/auth.ts: 340 lines]` → error head/tail cap → tail exclusion). Delta ≈ 0.
- **"Safe to evict" as a concept.** The paper's gate protects against dropping an antecedent of a live
  decision. In sctxx every rendered line carries `[evt a–b]` and `sctxx expand` recovers the original
  span, so nothing is irrecoverable; the real question is "is the pointer cheaper than the content?" — A2,
  a different question with a different answer.
- **The paper's numbers as evidence.** The baseline never faced context pressure by design, the protocol is
  asymmetric and the paper says so, the deltas are ≤3pp at n=3–5 with no variance or confidence intervals,
  and the paper itself says they "should not be interpreted as directional signal." None of it may be cited
  as evidence that a budgeted eviction policy is accuracy-neutral.
- **The repo's candidate formats** (columnar JSON, codebook JSON, typed CSV/TSV) encode *tables of records*;
  sctxx's payload is an ordered event stream with per-row provenance, which they do not represent. The
  **Codex PreToolUse rewrite hook** lands in a different lifecycle (pre-read, live agent) and in the host
  composition, not in a preset or in sctxx's pipeline.

## 9. Verdict per source

**Paper (CWL) — adopt-partially.** Adopt the design principles as a rubric (user content inviolable;
causal antecedents outrank recency; smallest-increment compression; no model in the loop) and the habit of
stating what was preserved and lost. Adopt the τ dial framing: cost and quality both improve with a tighter
budget until look-back breaks — sctxx's `--tail`/`--budget` are that dial and its defaults (12,000/8,000)
sit far below τ, which is consistent. Reject the policy as an implementable component: **U1 (no
content→level classifier) and U3 (the stated candidate predicate admits non-termination, and the retained
`description` has no eviction rule) make it unimplementable as written.** The empirical section is honest
about its own asymmetry and self-limits, so it is usable as motivation, not evidence.

**Repo (`context-compression`) — adopt-partially.** Adopt the verification discipline (A1–A4: all low cost,
all deterministic Rust). Reject the algorithm: it re-encodes structured data files, its own ablation shows
most candidates net-negative, its 13.3% corpus saving is dominated by one dictionary encoding, and its
measured local cost (break-even ≈47.8k input tok/s) is a latency liability at real provider speeds. It
contains nothing about transcripts, episodes, or eviction, so it must not be read as prior art for sctxx's
core problem.

**Could not verify:** the repo's star count (offline); whether `pi-cwl` resembles the paper's pseudocode —
I did not clone it, and Algorithm 1 should be treated as a description, not running code; and the 20–70%
cost range and 23% case-study figure, for which no supporting table or appendix appears in the converted
text.
