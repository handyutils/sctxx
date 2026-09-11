# The Compaction Cliff / Knowledge Triage — what sctxx should take from it

Source: **The Compaction Cliff in Long-Running AI Agent Memory** — Saber Zerhoudi, Jelena Mitrović,
Michael Granitzer (University of Passau; IT:U), accepted to CIKM 2026.
[arXiv:2608.22752](https://arxiv.org/html/2608.22752v1)

Why we read it: it is the only paper in the set that attacks *deterministic, type-aware* compaction
head-on, with algorithms specified precisely enough to implement, and with a safety argument that is
stated as a requirement on the output rather than as an empirical hope. sctxx is a compactor, its
constitution is deterministic-first, and it already has a `Constraint` item type. The paper explains
why that type is empty in practice and what to do about it.

---

## 1. The result that matters: type-blind compaction loses constraints monotonically

The paper's central empirical claim (§4.3) is that **uniform summarisation loses safety rules at the
same rate as surrounding prose, and the loss compounds**:

- 50 stratified configurations, 622 constraints, 2,257 procedures. Eight type-blind strategies,
  including hierarchical truncation, temporal windowing, aggressive pruning, LLMLingua-2, and four
  frontier LLM compactors invoked with the production prompt *"compress to N tokens, keep every
  safety rule and procedural command verbatim"*.
- Best single-round constraint recall across all eight: **0.53 at 50 %**, 0.39 at 25 %, 0.24 at 10 %.
- Claude Code's `/compact` on Sonnet 4.6 run for five rounds at 50 % per round: **0.53 → 0.10**.
- The decay appeared for **every LLM family and every structural baseline** — it is not a prompt bug.
- TypeCompact on the same 50 configurations: **1.00 / 0.95 / 0.80** at 50 / 25 / 10 %, stabilising at
  **0.96 from round two onward**. Table 5: it trades belief and preference fidelity (0.50 / 0.51) for
  full constraint and procedural retention.

The mechanism is trivial once stated: *a type-blind compactor has no signal for which sentences are
safety rules.* A model told to "keep every safety rule" cannot do it reliably, because the
instruction to keep them is not a mechanism for finding them.

**Type distribution in real agent knowledge** (AgentArtifactCorpus, 396,934 artifacts from 54,628
GitHub repos across Claude/Cursor/Copilot/Windsurf/Continue/Aider/Codeium/Universal; Table 4):
Constraint **12.3 %**, Procedural 28.7 %, Belief 31.4 %, Preference 14.2 %, Episodic 13.4 %. The
five-type model covers **97 %** of items; the residual 3 % are meta-instructions with no safety force.

## 2. The model, precisely

A typed knowledge base is a tuple `K = (I, τ, T, π, σ)`:

| symbol | meaning |
| --- | --- |
| `I` | finite set of items |
| `τ : I → {C,P,B,F,E}` | the type assignment |
| `T` | a topic tree |
| `π : I → leaves(T)` | leaf-topic mapping |
| `σ : I_C → 2^{leaves(T)}` | **scope**: the leaf topics each constraint applies to; global constraints have `σ(c) = leaves(T)` |

`σ` is the part most likely to be dropped in a reimplementation, and the paper's own worked example
shows why it cannot be: "never modify production schema without a migration file" sits under
Database but carries project-wide scope. Scope is what makes decomposition and retrieval provable.

Per-type distortion `d_t` (following Liu et al.): binary for constraints (`d_C = 0` if the rule
survives intact, `∞` otherwise), behaviour-preserving rewrites only for procedures, embedding
distance for beliefs and preferences (stricter for beliefs), gist distance for episodic items.

**Authority weighting** (§3.2): weights `w_sys = 1.5`, `w_dev = 1.25`, `w_user = 1.0`,
`w_tool = 0.75`, `w_ret = 0.6`; constraint risk `risk(i) = p(τ(i)=C) · w_{a(i)}`. Measured effect
(§4.4): on a 1,033-item retrieval corpus, risk-weighted TypeRetrieve reaches recall@{5,10,20,50} of
0.40 / 0.58 / 0.93 / 1.00 against full-pin 0.40 / 0.57 / 0.96 / 1.00 and dense retrieval
0.34 / 0.44 / 0.69 / 0.93 — the same recall@50 with a smaller pinned footprint.

## 3. The three operators, and their safety requirement

Each operator carries one requirement that depends on `τ`, and each has a **known failure mode
against type-blind strategies**. These are the three theorems, and they are the load-bearing part.

### 3.1 TypeCompact (Algorithm 1) — constraint preservation

Requirement: `∀ i ∈ I_C, ∃ i' ∈ C(K) : d_C(i, i') = 0`. Every constraint has a **zero-distortion
copy** in the output.

Feasible only above `B_min = Σ_{i : τ(i) ∈ {C,P}} |i|`. Below it: expand the budget, relax safety,
or fall through to decomposition.

```
TypeCompact(K, B, h, v, a, θ_C, θ_P) -> K' with |K'| ≤ B, or Unsafe
  for i in K:
      p_i ← h(i, ctx(i))
      if p_i(C)·w_a(i) ≥ θ_C or abstain(p_i):  H_C ← H_C ∪ {i};  g_i ← guard(i)
      elif p_i(P) ≥ θ_P:                        H_P ← H_P ∪ {i}
      else:                                     route i to soft lane by argmax type
  H ← dedup(H_C ∪ H_P);  if ℓ(H) > B: return Unsafe
  allocate B − ℓ(H) across the soft lane: beliefs/prefs → compressed, episodic → placeholder
  K' ← H ∪ soft
  if v({g_i}, K') = fail: restore failed guards if budget permits, else return Unsafe
  return K'
```

Three lanes: constraints and procedures at **full fidelity**, beliefs and preferences **compressed**,
episodic **placeholdered**. Note the escalation: it returns `Unsafe` rather than a quietly weaker
artifact. That is the design decision worth copying.

**The deterministic verifier `v`** is the piece a reimplementation is most tempted to skip, and the
paper measures what skipping it costs (§4.3 ablations):

- Across the run the verifier recorded **27 restoration events** (mean 0.46 per call, max 1) and never
  escalated to `Unsafe` at the 50 % budget. Without it, the same runs "would silently truncate the
  hard lane".
- On the **1.4 %** of configurations whose constraint-plus-procedural density exceeded 40 %,
  `B_min` approached the budget. TypeCompact escalated to `Unsafe` on 28 % and reached 1.00 on the
  other 72 %. **The same configurations without the verifier reported apparent 1.00 recall while
  silently dropping a mean 57 % of the constraints that should have been kept.**

  That is the whole argument for it: the verifier's absence does not show up as a lower score, it
  shows up as a *higher* score that is a lie.

- Removing any single component (indexing-time labels, verifier, Unsafe escalation) puts a deployed
  agent below the 0.50 mark.

The verifier's mechanism: extract a **canonical form (negation plus object phrase)** from every
constraint in the hard lane, check it appears in the output, restore the original or escalate.

### 3.2 TypeDecompose (Algorithm 2) — constraint locality

Requirement: `∀ c ∈ I_C, ∀ j ∈ [m] : (∃ i ∈ K_j : π(i) ∈ σ(c)) ⇒ c ∈ K_j`. **Every partition that
holds an item covered by a constraint's scope must also hold the constraint.**

```
TypeDecompose(K, B) -> {K_1 … K_m}, each ≤ B
  group K by topic; chunk each group into sub-bases of size ≤ B
  for each constraint c in I_C:
      for each partition K_i:
          if ∃ i' ∈ K_i : π(i') ∈ σ(c) and c ∉ K_i:  K_i ← K_i ∪ {c}   # replicate
  return {K_1 … K_m}
```

Measured: **0 % locality violations versus 93 %** for type-blind partitioning (partitioning by topic
similarity, item frequency, or token count). Cost: replication overhead median **0 %**, worst case
**219 %** — the price of copying each constraint into every partition its scope covers.

This is the operator that maps onto sctxx's fold directly. sctxx packs episodes into ~24k-token
chunks by size and hands each chunk to a separate pass, carrying state forward in the prompt. A
constraint stated in episode 3 survives into chunk 30 only if a model chose to re-emit it, in every
intervening chunk, thirty times. That is exactly the partitioner this section measures at 93 %
locality violations.

### 3.3 TypeRetrieve (Algorithm 3) — constraint priority

Requirement: `∀ c ∈ I_C : inscope(q,c) ⇒ c ∈ R_q`, **regardless of similarity**.
`inscope(q,c) ⇔ topics(q) ∩ σ(c) ≠ ∅`.

```
TypeRetrieve(K, q, k, r, inscope) -> C_q ∪ R_others
  C_q      ← {c ∈ I_C : inscope(q,c)}                       # pin in-scope constraints
  R_others ← topk(K \ C_q, k − |C_q|, by = r(q, ·))          # then spend the residual by relevance
  return C_q ∪ R_others
```

Measured **100 % versus 73 % recall@50**. Any scorer based on similarity alone can rank an in-scope
constraint below the cutoff.

### 3.4 Composition

Compaction first; below `B_min`, where compaction would drop a constraint, fall through to
decomposition, pushing part of the base to external storage for TypeRetrieve to pull back on demand.

## 4. The classifier — and the part that runs without a model

`τ` is the only learned component, and **every per-operator safety claim depends on it**: a missed
constraint falls into the soft lane where it can be paraphrased or dropped. The paper is explicit
that the guarantee rests entirely on `τ`'s recall.

Ten variants on a 200-item test set (Table 9):

| variant | C-recall | C-F1 | macro-F1 | latency |
| --- | --- | --- | --- | --- |
| **regex only** | **0.60** | 0.63 | 0.35 | **< 1 ms** |
| encoder only | 0.27 | 0.32 | 0.33 | 387 ms |
| regex + encoder | 0.70 | 0.63 | 0.40 | 266 ms |
| distilled MiniLM + LR | — | — | — | — |
| single-shot LLMs (nano/mini/Sonnet/Opus) | 0.73–0.93 | — | — | 667 ms |
| SafetyMargin (counterfactual, gpt-5.4-mini) | **0.93** | — | — | 880 ms |

The regex stage's four pattern families, tried in **fixed precedence order, constraints first**:

1. **constraint indicators** — `never`, `must not`, capitalised markers such as `CRITICAL`, and
   `always` / `ensure` / `require` **combined with a modal verb**;
2. **procedural indicators** — commands opening with verbs such as `run`, code blocks, package-manager
   invocations;
3. **temporal markers** for episodic items — `yesterday`, `recently`;
4. **preference indicators** — `prefer`, `ideally`.

Unmatched items fall through to the encoder's prototype-centroid cosine (ten labelled examples per
type); below 0.55 confidence they fall through to the LLM; **below 0.40 they are routed to the hard
lane as constraints** — `abstain`-as-hard, i.e. uncertainty resolves toward safety. In the cascade,
`regex → encoder → LLM → abstain-as-hard`.

Two honest caveats about the cheap path, both from the paper:

- **Declarative phrasing defeats grammatical classifiers.** Rewriting 50 author-written safety rules
  into four grammatical forms (200 items, labels known in advance): LLM variants handle imperative,
  conditional, and passive forms at 1.00 recall but **lose 38–48 absolute points on declarative
  phrasing**. "The patient is allergic to penicillin" is recognised as a constraint by gpt-5.4-mini
  62 % of the time, by the selective cascade 52 %, by **regex 0 %**. Declarative phrasing is 49.8 % of
  openFDA safety text and 61.1 % of LegalBench contract-NLI safety clauses.
- Therefore the paper's own recommendation is a split: `{Regex, MiniLM, mini, nano}` for
  imperative-heavy text, and **regex explicitly for no-LLM deployments**, accepting its ceiling.

Note the two things regex *does* buy: it is free and it is **not stochastic** — the same transcript
always yields the same constraints. For sctxx that is not a compromise, it is the constitution.

## 5. Where this lands on sctxx

### 5.1 The defect it explains

sctxx's state schema has `ItemKind::Constraint`, first in `ItemKind::PRIORITY`, rendered as
**Hard constraints** and repeated in the artifact preamble: *"treat 'Hard constraints' as binding
user instructions"*. That item is produced **only by the fold**. Since ADR 0007 made `--llm` default
to `none`, the default artifact has:

```
$ sctxx extract <session> --llm none --out /tmp/newsctxx
$ cat /tmp/newsctxx/state.json
{ "version": 1, "items": [], "ops_log": [], "rejected": [], "processed": [], "next_seq": {} }
```

On a real 103,757-event, 274-turn session the default artifact instructs its reader to treat a
section as binding, and that section does not exist. Every constraint the human stated over ten days
— and the paper's corpus says roughly one item in eight is a constraint — is absent from the
deterministic artifact by construction. The semantic layer is opt-in; the *safety* layer must not be.

### 5.2 What sctxx already has that the paper assumes

- **A topic tree and a leaf mapping.** `subsystem_of()` already resolves a path like
  `acryl-tui/src/render.rs` to the subsystem `acryl-tui/src`; the ledgers rank subsystems by activity.
  That is `T` and `π` in the paper's tuple, already computed, already deterministic.
- **Deterministic first, LLM only where the spec puts it** (§AGENTS.md hard rule 9). The paper's
  no-LLM recommendation is not a fallback here, it is the target configuration.
- **Provenance on every item.** The paper cannot verify a constraint's survival against its source;
  sctxx can, because every item carries `[evt a–b]`.
- **A verify stage (S5) and a `verify` subcommand.** The post-compaction verifier is a new check in
  an existing stage rather than a new stage.
- **A budgeted, masked, segmented pipeline.** TypeCompact is a routing policy over lanes that already
  exist (hard lane ≈ L0/L1 verbatim items, soft lane ≈ compressed digests, placeholder ≈ the counts
  that replaced the file list in L0).

### 5.3 What is genuinely missing

| missing | paper section | why it matters here |
| --- | --- | --- |
| a deterministic `τ` over the transcript | §3.2, §4.4 | `Constraint` is empty whenever no model runs |
| scope `σ` on constraints | §3.1 | without it, a constraint cannot be routed to the chunks it governs |
| constraint replication across chunks | §3.3.2 | the fold's 40-chunk partitioner is the 93 %-violation case |
| a deterministic post-compaction guard verifier | §3.3.1 | the `--llm none` path has nothing to verify against |
| an `Unsafe` outcome | §3.3.1 | sctxx currently degrades silently; the paper escalates |
| a measured constraint-recall figure | §4.3 | sctxx reports tokens, calls, and rejections, never what survived |

### 5.4 The honest limits of the transfer

- **The paper compacts a live agent's own memory mid-run; sctxx compacts a finished transcript for a
  different agent to read.** `B_min` is therefore not a runtime constraint — sctxx can always emit
  more. What survives the transfer is the *ordering and the verifier*, not the emergency: sctxx should
  never truncate a constraint to fit a budget, because it has no budget it cannot raise.
- **The classifier's recall ceiling is the ceiling of the whole scheme.** Regex-only is 0.60 C-recall
  on the paper's test set, against 0.93 for a frontier counterfactual classifier. Adopting the
  deterministic classifier means accepting that roughly two in five constraints that a frontier model
  would find are invisible to it — and, worse, the miss rate is concentrated in *declarative*
  phrasing, exactly the register of a human stating a rule in a session ("the schema is frozen until
  the migration lands"). A regex-only layer is a floor, not a solution.
- **sctxx's items are not the paper's items.** The paper classifies authored configuration artifacts
  (rules files, prompts, instruction docs). sctxx classifies turns inside a transcript, where a
  constraint arrives as one sentence inside a paragraph of narration. Precision at that granularity is
  unmeasured by the paper and must be measured here.
- **Authority weighting has no clean analogue.** `sys/dev/user/tool/ret` does not map onto a
  two-party transcript; `user` versus `assistant` does, but an assistant's restatement of a rule is
  not evidence of the rule, and the existing rule "under-index on assistant suggestions" already says so.

## 5.5 What happened when it was built and measured

The layer was implemented (ADR 0008) and run against the real session this document's §5.1 defect was
found on. The measurements are worth recording because they are less flattering than the framework.

| run | constraints found | honest assessment |
| --- | --- | --- |
| first classifier, markers anywhere in the sentence | **40** | first six inspected: a pasted heading (`CONTRACTS YOUR HOT-RELOAD / LIFECYCLE WORK MUST NOT BREAK`), a file comment (`(blends index + blends init acryl.demo) - do not hand-edit`), a plan item (`032 follow-up (6f468a8) - do not add a parallel mechanism`), a mid-sentence fragment. **0 of 6 were instructions.** |
| head-anchored, all marker families | 3 | 1 real (`never add claude to the commiter`), 2 tasks (`make sure we don thvave .dsh floating config…`, `also ensure that both work if i save API key…`) |
| head-anchored, transcript-reliable families only | **1** | `never add claude to the commiter`. Precision 1/1, recall unknown and low. |

The middle row is the interesting one, and it is the paper's own caveat arriving as a measurement:
`make sure` / `ensure` open a **rule** in an authored configuration file and a **task** in a
transcript. The words are identical; what separates them is knowledge about the world that the
pattern does not have. So the shipped classifier has no such family, and the artifact states the
consequence.

Two further findings from the same work, both about the *renderer* rather than the classifier:

- With 40 constraints in the state, the **whole block was dropped** from L0 by the budget helper,
  silently and in one piece, while the preamble still said the section was binding. The paper's
  57 %-silently-dropped figure is this failure with better instrumentation.
- L0 did not respect `--budget` at all: `--budget 400` emitted a 1,200-token brief, because the brief
  had its own fixed ceiling and three of its blocks were never charged to any budget.

The honest summary: the *framework* transferred and the *classifier* did not. What sctxx gained that
is unambiguously worth having is the verifier, the budget discipline, the replication rule, and a
reported number where there used to be an unmeasured claim — not the extraction quality. Anyone
reading this as "regex now finds the constraints" has read it wrong, which is why the artifact says
so in the section itself rather than only here.

## 6. Recommended adoption, ranked

1. **Deterministic typed extraction with scope** (new S1b stage; regex + structural features,
   authority-weighted, no model). Emits `Constraint` / `Procedural` / `Belief` / `Preference` /
   `Episodic` items with `σ` derived from the ledgers' subsystems. *This is the fix for the empty
   `Constraint` type and the highest-value change in this document.*
2. **Replicate constraints into every chunk whose scope they cover, before the fold**
   (TypeDecompose). Turns the fold's 93 %-violation partitioner into a 0 %-violation one at median
   0 % overhead, and makes the fold additive rather than load-bearing for safety.
3. **Bound-verbatim constraints in L0, never compressed, never budgeted away** (TypeCompact's hard
   lane, minus the budget emergency: sctxx raises the budget instead of escalating).
4. **A guard verifier in S5** with three outcomes — `Preserved` / `Restored` / `Unsafe` — replacing
   today's silence, and a `constraints: {found, preserved, missed}` line in the front matter. This is
   what makes the artifact's quality claim checkable instead of asserted.
5. **Abstain-as-hard.** Unclassified turns whose text carries deontic force go to the hard lane.
   Uncertainty must resolve toward safety, not toward the soft lane.
6. **TypeRetrieve discipline in `expand` and in the workset**: in-scope constraints are pinned before
   relevance ranking, and pinned items do not count against the relevance budget.

Rejected: porting the encoder stage (387 ms/item, C-recall 0.27 — worse than regex on every axis that
matters here); the LLM classifier (sctxx's constitution puts the model in the fold, not in indexing);
and `Unsafe` as a terminal state (sctxx can always emit a larger artifact — it should report the
shortfall, not refuse).

## 7. Verdict

**Adopt, substantially.** The framework is the right shape for sctxx: it isolates exactly one learned
component (which sctxx can replace with a weaker but deterministic one), it names the failure mode
type-blind compaction has — which is sctxx's failure mode with `--llm none` — and its three operators
are each a small, testable, deterministic routine that drops into a stage sctxx already has. The
measurement discipline is the part to take most seriously: the paper's most useful single number is
not 1.00 vs 0.53, it is **57 % of constraints silently dropped by a system that reported 1.00**.
sctxx currently reports nothing at all about constraint survival, which is strictly worse.
