# 0008 — The deterministic typed layer

Status: accepted (2026-09-11)

## Context

`ItemKind::Constraint` is the first entry in `ItemKind::PRIORITY`, the artifact renders it as
**Hard constraints**, and the artifact's own preamble instructs its reader to *"treat 'Hard
constraints' as binding user instructions"*. Two things were true of it at 0.3.0:

1. **The only producer of a `Constraint` item was the fold.** ADR 0007 made the fold opt-in, so with
   the default `--llm none` the section was empty by construction. Measured on the real 103,757-event
   session: `state.json` was `{"items": [], ...}` and the artifact told its reader to obey a section
   that did not exist.
2. **When the section was not empty, it could still vanish.** `render_brief` renders each block
   through a `push` helper that returns early — dropping the block whole — when it does not fit the
   remaining L0 budget. With 40 constraints in state, the block exceeded the brief budget and was
   discarded silently, in one piece, with no marker. A run that produced the constraints and a run
   that found none were indistinguishable to the reader. (Re-measured after the first fix: 40
   constraints seeded, 40 dropped, `Hard constraints` absent from the artifact.)

The external evidence for taking this seriously is
[arXiv:2608.22752](https://arxiv.org/html/2608.22752v1) (Zerhoudi, Mitrović, Granitzer, CIKM 2026),
read in `docs/research/2026-09-11-paper-knowledge-triage-typecompact.md`. Its measurement: eight
type-blind compaction strategies, including four frontier LLM compactors invoked with the production
prompt *"compress to N tokens, keep every safety rule and procedural command verbatim"*, retain a
best-of **0.53** of an agent's constraints at a 50 % budget, **0.39** at 25 %, **0.24** at 10 %, and
Claude Code's `/compact` decays from 0.53 to **0.10** over five rounds. The reason is not a prompt
bug: *a type-blind compactor has no signal for which sentences are safety rules.* The paper's own
recommendation for a no-LLM deployment is an explicit classifier.

Its most useful number is not 1.00-versus-0.53. It is that a run **without** the deterministic
post-compaction verifier reported apparent 1.00 constraint recall while silently dropping a mean
**57 %** of the constraints that should have been kept. Both defects above are that failure: a
reported state that does not describe the artifact.

## Decision

**Safety-critical items are extracted, carried, and verified deterministically, and the section that
renders them is never dropped whole.**

1. **A new pipeline stage, S1b, runs before the mask, the segment, and any model call.** It reads the
   session's non-meta user turns and emits `Constraint` items — no schema change, because the item
   kind already exists. `--llm none` now yields a non-empty state.
2. **The classifier is anchored at the head of the clause.** A rule is stated in the imperative, so
   its directive comes first: `never push to main` is a rule, `contracts must not break` is a
   property that contains the same words. The first implementation did not anchor and returned
   `GATES THAT MUST STAY GREEN` and `intercept() and fetch.register() don't touch webServer` as
   binding user instructions. In a transcript the imperative mood is what the user wants done *now*,
   so `make sure` / `ensure` / `be sure` and bare `you must` are deliberately **not** markers — the
   paper's corpus is authored rules files, where they are.
3. **Constraints carry a scope `σ`, and are replicated into the chunks they govern.** Global is the
   default when scope is undecidable, because over-replication costs tokens and under-replication
   loses the rule. This is the paper's TypeDecompose, measured at 0 % locality violations against
   93 % for type-blind partitioning, at a median 0 % replication overhead. sctxx's fold walks 40
   sequential chunks; without this, a rule stated in chunk 2 governs chunk 30 only if a model
   re-emitted it in each of the 28 calls between.
4. **A verifier in S5 checks the state the fold produced**, restores what it can, and reports what it
   could not. Restore-and-recheck, not restore-and-assume.
5. **The mandatory blocks spend the budget first and are never refused** — the notice that no model
   ran, the contradictions, and the user's own instructions. `--budget` bounds the optional content;
   an artifact that fits its budget by deleting those is not smaller, it is wrong. This also fixed a
   pre-existing violation: L0 ignored `--budget` entirely, emitting a 1,200-token brief under
   `--budget 400`.
6. **The artifact states what the deterministic layer cannot see**, in the section itself, whenever
   no model ran.

### Authority: what this layer is not

It is a **floor**, not a solution, and the artifact says so rather than implying coverage. Measured
on the real session in `docs/research/…-knowledge-triage-typecompact.md` §5.5: 274 user turns, 1
constraint found, precision verified by hand at 1/1. On the small fixtures the same classifier finds
`Do not touch the parser` and `Never auto-install extensions from the registry without asking me` and
correctly finds nothing in the Codex fixture. The paper predicts the ceiling: regex recall on
*declarative* phrasing is **0**, and declarative phrasing is 49.8 % of real safety text in the
domains it measured. A rule stated as "the schema is frozen until the migration lands" is invisible
here, and the artifact says so in the section a reader would otherwise over-trust.

## Consequences

- The default artifact is no longer self-contradictory: it no longer instructs its reader to obey an
  absent section.
- Constraint survival is now a reported number (`triage: {constraints, preserved, restored, missing}`
  in the front matter and in `report.json`) instead of an unmeasured claim.
- `--budget` becomes a real bound on the optional content, and the mandatory content is bounded by
  its own three blocks instead. This is a behaviour change for anyone relying on `--budget` as a hard
  ceiling on artifact size.
- A false positive is cheaper than it was — one wrong line in a section, not the whole artifact — but
  the section is presented as *binding*, so the classifier rejects on the conservative side and
  accepts a low recall. The next step, when it is taken, is a higher-recall classifier whose misses
  are measured, not a looser pattern set.
- `docs/SCTXX-SPEC.md` §8 gains S1b; §12.2 gains the mandatory-block rule.

## Alternatives rejected

- **Leaving the section to the fold.** That is the state this ADR changes; a section that is opt-in
  cannot be described as binding.
- **Making the fold the default.** ADR 0007's reasons stand, and the real-session evidence is
  against it: the fold that ran on the 103k-event session made 81 calls, and the earlier 0.2.0
  publication run produced `accepted_ops: 0` from a provider rate limit while reporting an ordinary
  handoff.
- **A regex that also matches mid-sentence deontic verbs.** Tried, measured, reverted: it returned
  40 constraints from 274 turns of which the first six inspected were headings, file comments, and
  descriptions of what code does not do.
- **Porting the paper's encoder stage.** Measured at 387 ms per item with constraint recall 0.27 —
  worse than the pattern classifier on every axis that matters here, and sctxx's constitution keeps
  the model in the fold rather than in indexing.
- **Emitting `Unsafe` as a terminal state**, as the paper does. sctxx has no budget it cannot raise;
  it reports the shortfall instead of refusing.
