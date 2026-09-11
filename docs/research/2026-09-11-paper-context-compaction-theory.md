# Context Compaction Theory — close read, and what it licenses for sctxx

**Paper.** Hayder Tirmazi (AllSpice Inc. and Boston University), Sam Markelon (Proof Trading), Allison Bishop (Proof Trading and CCNY), Michael Mitzenmacher (Harvard). *Context Compaction Theory.* URL `https://arxiv.org/html/2608.01326v1`. Read in full from the converted text at `/tmp/papers/2608.01326v1.txt` (1379 lines); §2.2, §2.4, §3 (all), §4 (all), §5, and Appendix A read closely.

**Why we read it.** sctxx's product claim is a split: a deterministic pipeline that keeps evidence, plus an optional LLM fold that writes typed items — the constitution's "deterministic first, LLM last". That split had no theory behind it. This paper is the first formal treatment of context compaction we can cite, and it answers two questions we have been asserting: what class of algorithm each half of sctxx is, and what a compaction artifact can and cannot guarantee. Its answer to the second is sharper than our README and cuts against one metric we imply: compression ratio.

**Reading discipline.** Everything below is quoted or paraphrased from the paper. Where the paper is silent — retrieval, hierarchical summarization, multi-round handoff, hybrid budget allocation — I say so rather than invent a result. Where a theorem's statement is narrower than the paper's own prose about it, I flag the gap.

---

## 2. The two games, formally

Both games model **one invocation** of compaction on the agent's internal state `I`, with the compacted output committed before the query arrives. §2.2: "Since the context compaction is committed before the query arrives, the context compaction algorithm cannot depend on the query." Context is modeled "as a set of discrete items instead of a sequence of tokens" — "atomic units of information such as variable names, error messages, design decisions, or instructions" — and "Our framework is agnostic to the choice of granularity."

### Definition 1 — Context Selection Game (§3.1)

An instance is `(X, s, B, Q, query regime)`: the **item universe** `X = {x_1,…,x_N}`; a **size function** `s(x_i) > 0` ("the amount of context window space it occupies", additive over the retained set, and "each retained item may be losslessly compressed on its own"); a **budget** `B > 0` ("the maximum total size of items that can be retained within the context window of an LLM"); a **query space** `Q` where each `q` has a value function `v_q : 2^X → [0,1]`; and a **query regime** (Definition 2).

**Players.** "a selector, i.e., the context compaction algorithm, and an adversary, which represents the future information needs of an agent." **Three stages.** (1) The adversary fixes `X` and each item's size. (2) The selector observes `X` and chooses `S ⊆ X` with `Σ_{x_i∈S} s(x_i) ≤ B`. (3) A query `q` is chosen per the regime. **Payoff.** `val(S,q) := v_q(S)`; the selector "incurs error `err(S,q) := 1 − val(S,q)`, which it seeks to minimize". `B`, `Q`, and the regime are fixed before the game starts and known to both players.

**Class.** "A context compaction algorithm is a **selection algorithm** if its output identifies an unordered subset `S ⊆ X`. Its cost is the additive `Σ_{x_i∈S} s(x_i)` … We denote the class of selection algorithms `SELECT`."

### Definition 2 — Query regimes (§3.1)

- **Stochastic.** "the item universe and the query are drawn together from a known distribution `μ` over pairs `(X,q)`. This draw replaces the adversarial choice of the item universe in the first stage. The selector observes `X` but not `q`. Its error is the expected error under `μ`."
- **Oblivious adversary.** "the adversary fixes the item universe as in the first stage and then chooses the query `q` with no access to the subset `S` chosen by the selector. Its error is the worst case of its error over all inputs `(X,q)`."

The ordering note matters: "The minimum budget in the oblivious adversary regime, at any target error, is therefore at least the minimum budget in the stochastic regime at the same target error, for every distribution `μ`." And: "a context compaction algorithm designed for a distribution `μ` may achieve a low error in expectation and still have a high error on an input that `μ` draws rarely. An oblivious adversary can choose that particular bad input."

### Definition 3 — Context Generation Game (§3.2)

An instance is `(X, B, Σ, Q, query regime)`, with `v_q : Out → [0,1]` scoring a candidate output. **Players.** "a generator, i.e., the context compaction algorithm together with the LLM that reads the compacted context, and an adversary, as in Definition 1." **Three stages.** (1) The adversary fixes `X`. (2) The generator applies a **condenser** `Cond` mapping `X` to a message `Cond(X) ∈ Σ^{≤B}`, and "commits an interpreter `Int : Σ^{≤B} × Q → Out`". (3) `q` is chosen per the regime; the score is `v_q(â)` where `â = Int(Cond(X), q)`.

### Where they diverge

`SELECT` is "the special case of the Context Generation Game with the following parameters. The output space is `Out = 2^X`. The condenser–interpreter pair lies in `SELECT`, i.e., `Cond`'s output identifies a subset `S ⊆ X`, possibly after lossless decoding, and `Int` returns `S`." Generation drops **three restrictions** (§3.2): it (1) "may emit new tokens that appear in no item of `X`, such as the tokens of an LLM-generated summary", (2) "may reorder any retained items to convey information", and (3) "may encode the whole retained set at a size below `Σ s(x_i)`" — e.g. "one bit per item of `X`" to record membership of `S`, "far smaller than `Σ s(x_i)`" when `S` contains most of `X`.

So `SELECT ⊆ GEN` by construction, and the objective differs in kind: `SELECT` optimizes **which items to keep**, priced by content kept; `GEN` optimizes **what bits to send**, priced by message length. §4.1 switches units deliberately: "in this section we measure the budget for the Context Generation Game in bits and identify the token alphabet `Σ = {0,1}`. For `|Σ| > 1`, `B` symbols are equivalent to `B log|Σ|` bits." **Do not mix units when quoting a bound at sctxx:** in Definition 1 `B` is token-like content size; in §4 it is bits.

---

## 3. §3.3 classification of real mechanisms (Table 1, reproduced)

| System | Class | Granularity (of the SELECT layer) |
|---|---|---|
| Codex [34] | `GEN` | — |
| Gemini CLI [19] | `GEN` | — |
| Claude Code [4] | `SELECT + GEN` | Tool result |
| OpenCode [42] | `SELECT + GEN` | Tool result |
| OpenAI Assistants truncation [33] | `SELECT` | Message |
| LangChain `trim_messages` [27] | `SELECT` | Message |
| Aider repo map [18] | `SELECT` | Code symbol |
| LLMLingua [22] | `SELECT` | Token |
| Selective Context [29] | `SELECT` | Token |

Table 1 caption: "Hybrid entries (`SELECT + GEN`) stack a `SELECT` layer with a `GEN` summarization fallback when the `SELECT` budget is exhausted; the listed granularity is that of the `SELECT` layer." §3.3: "Every algorithm in our survey lies in `SELECT ∪ GEN`, and the `SELECT` entries span granularities from individual tokens up to entire messages."

§2.4 supplies the mechanism behind each row: **Codex** is "LLM-based summarization" at a per-model token threshold, and the summary "replaces the conversation history for future iterations" (Codex warns that "long threads and multiple compactions can cause the model to be less accurate"); **Gemini CLI** does the same but "instructs the LLM to produce a structured XML snapshot"; **Claude Code** uses "a tiered eviction strategy that tries cheap alternatives such as inline compression of large tool outputs before invoking LLM-based summarization"; **OpenCode** "can additionally prune the outputs of old tool calls"; **OpenAI Assistants** offers `last_messages` ("keeps the N most recent messages") and `auto` ("drops middle messages to fit a token budget"); **LangChain** `trim_messages` drops messages by configurable strategy, "e.g., dropping the oldest messages"; **Aider** "ranks code symbols by a PageRank-style importance score and selects the top-`k` that fit a budget"; **LLMLingua** and **Selective Context** "delete low-information tokens, producing a compressed prompt that is a sub-sequence of the original tokens".

**Two omissions to note.** Table 1 contains **no retrieval entry** and **no hierarchical/tiered-summarization entry**; the nearest is Claude Code's tiered eviction, classified as a hybrid at tool-result granularity. Retrieval-augmented answering is structurally outside Definition 3 — `Int` receives only `(m, q)`, never `X` — even though §2.2 acknowledges the raw history is kept in storage. **Do not claim the paper covers retrieval-style compaction.**

---

## 4. The communication reduction and the lower bounds

### Definition 4 — the induced one-way problem (§4.1)

"Alice receives the item universe `X` and Bob receives a query `q ∈ Q`. Alice then sends a single message `m ∈ Σ^{≤B}` to Bob. Bob then outputs an answer `â ∈ Out` for the query `q`." Protocol error on `(X,q)` is `1 − v_q(â)`. Two measures: `R^→_{μ,ε}(Π)`, "the `μ`-distributional one-way **public-coin randomized** communication complexity of `Π` at error `ε` … the minimum message length in bits of a protocol whose **expected** error under `μ` is at most `ε`"; and `R^→_ε(Π)`, "the minimum message length of a public-coin randomized protocol whose **expected** error, over its public coins, is at most `ε` **on every input** `(X,q)`".

### Theorem 1 (Generation ≡ one-way communication)

Let `G` be a Context Generation Game and `Π_G` its induced one-way problem.

- **Property 1 (stochastic).** "For any input distribution `μ` over pairs `(X,q)`, the minimum budget at which some (possibly randomized) `GEN` algorithm achieves expected error `≤ ε` with the input drawn from `μ` equals `R^→_{μ,ε}(Π_G)`." Expectation over both the algorithm's internal randomness and `(X,q) ~ μ`, "with the conversation and the query drawn jointly from `μ`".
- **Property 2 (oblivious).** "In the oblivious adversary query regime, the minimum budget at which some (possibly randomized) `GEN` algorithm achieves expected error `≤ ε` on every input `(X,q)` equals `R^→_ε(Π_G)`." Expectation over the algorithm's randomness; "the bound is required on every input, so the controlled quantity is the worst case over inputs of the expected error."

**Hypotheses to carry with every citation.** (i) The generator is a condenser–interpreter pair chosen before the query; (ii) the class is `GEN` — an arbitrary message of at most `B` bits, **not** `SELECT`; (iii) randomization is **public-coin**; (iv) error is **expected** error, either under `μ` or, in Property 2, per-input worst case over inputs; (v) the regime is stochastic or oblivious — **not** adaptive.

**Proof mechanism — why the direction of a bound matters.** "a `GEN` algorithm `G` … and a one-way communication protocol `π` … are described by the same pair of functions" (condenser, interpreter), and "The answer `Int(Cond(X), q)` is identical in both." So an upper bound is *existential*, while a lower bound for `Π_G` "bounds every `GEN` algorithm, regardless of how its interpreter is computed" (§4.3). That asymmetry is the most usable fact in the paper for sctxx (§8.1 A1).

### Corollary 2 — selection is a restricted protocol class

"`SELECT` algorithms correspond exactly to one-way protocols in which Alice's message identifies a subset `S ⊆ [N]` such that `Σ_{i∈S} s(x_i) ≤ B`, and Bob's output is a function only of `S`, the set `{x_i : i ∈ S}`, `q`, and the public coins. In particular, every gap between `SELECT` and `GEN` on a set of queries `Q` is a gap between subset-encoding protocols and unrestricted one-way protocols." Hypotheses are Theorem 1's, inherited by applying it to the subclass; the added restriction is that Bob gets the retained *items*, not a free-form message about them.

### Theorem 3 (Selection ⊊ Generation) — the only selection-specific lower bound

> "Let `n = 2^k` for an integer `k ≥ 2`. There is a set `Y` of `n` items and a single query `q` on `Y` with the following property: some `GEN` algorithm answers `q` with zero error using a budget of `n` bits, while every `SELECT` algorithm that answers `q` with zero error requires a budget of at least `nk = n log_2 n > n` bits."

Proof: each item has size `s(x) = log_2 n` bits; the adversary fixes `X` to be "an arbitrary, possibly empty, subset of `Y`"; `q` asks for `X` ("an answer is correct if and only if it equals `X`"). Upper bound: send the `n`-bit indicator vector. Lower bound: a zero-error `SELECT` algorithm has `f(S(X)) = X` for every `X ⊆ Y`, so `X ↦ S(X)` is an injection of the power set into itself, hence a bijection; `S(X*) = Y` for some `X*`; since `S(X*) ⊆ X*`, `X* = Y`; on input `Y` it "retains all `n` items at a cost of `n log_2 n` bits". Randomized `SELECT` is covered because "the error must be zero for every value of its public random coins".

**Two overclaims to avoid.** (a) The theorem is stated at **zero error** only, while the introduction and §4.2 describe "a factor of `Θ(log n)` more budget than generation **to reach the same error**" — broader than the stated result; cite it at `ε = 0`. (b) It is witnessed by **one** adversarial family (`q` asks for the whole universe); "Characterizing which sets of queries admit a gap … remains open" (§5).

### The §5 lower bound that also applies to generation (set disjointness)

The worked example: an agent records the set `S` of dependencies and must answer "Do any of these dependencies appear in this list of packages with known CVEs?", i.e. `S ∩ T = ∅` for arbitrary `T`. Because the compacted context is fixed before the query, "to answer every such query correctly, the compacted context must determine `S` exactly." With `m`-bit package identifiers (`|U| = 2^m`) and `N = |S|`, "Representing an `N`-element subset of `U` requires `log_2 C(|U|, N) ≥ N log_2(|U|/N)` bits … so this is `Ω(Nm)` bits. By Theorem 1, every strategy for the Context Generation Game that answers this query correctly on all inputs must use a context compaction budget of `Ω(Nm)` bits. This is no better than storing the dependencies uncompressed." The Bloom-filter route fails too: a filter with false-positive rate `ε` uses about `1.44 N log_2(1/ε)` bits, but disjointness errs unless all `|T|` membership queries are negative, giving error `1 − (1−ε)^{|T|}`; holding error below `δ` forces `ε = O(δ/|T|)` and hence `Ω(N log_2(|T|/δ))` bits — and since `|T|` can be `|U| = 2^m`, again `Ω(Nm)`. "This holds even when the target error `δ` is a constant."

**Realism for a coding-agent transcript.** These hypotheses are demanding: one fixed message, and correctness on *every* input against an adversary who knows the query family but not the message. For a handoff, that is the right worst-case question for "did we lose the dependency list?", and the answer is that for arbitrary-list disjointness no budget below storing the list works. It is *not* the right question for "will the next agent generally succeed", which is distributional over a `μ` nobody has characterized.

---

## 5. §4.3 Computation Caveats — the authors' own scope limits

1. **Information, not computation.** "The equivalence of Theorem 1 is information-theoretic. Our bounds are on the context compaction budget … but do not address the computation a context compaction algorithm must perform to attain it." "An upper bound on the context compaction budget exhibits some `GEN` algorithm that meets it, but does not guarantee that a particular context compaction algorithm, such as a summarizer whose interpreter is an LLM call, can compute it. A **lower bound** on the context compaction budget does not have this caveat … a lower bound for `Π_G` bounds every `GEN` algorithm, regardless of how its interpreter is computed."
2. **Regime restriction.** "The equivalence of Theorem 1 is also restricted to the stochastic and oblivious query regimes of Definition 2. Under an **adaptive adversary**, which observes the condenser's output before choosing `q`, Theorem 1 does not apply." Left open (§5).

Two further limits sit outside §4.3 and must travel with the theorem: the model is a **single invocation** — §2.2 calls it "the most favorable case for the agent. If a single context compaction cannot preserve an answer, neither can a session of many context compactions" — and Appendix A is explicitly "a snapshot of one deployed endpoint … on one workload … at the time of writing".

---

## 6. §5 open problems (condensed, in the paper's terms)

1. **Adaptive adversaries.** "An adaptive adversary observes the condenser's output before choosing its query. Theorem 1 does not apply in this regime. We leave finding a communication model whose complexity equals the minimum context compaction budget under an adaptive adversary as an open problem."
2. **Separating selection from generation.** Any gap is "a gap between subset-encoding protocols and unrestricted one-way protocols" (Corollary 2), and Theorem 3 exhibits "one such gap, of size `Θ(log n)`". "Characterizing which sets of queries admit a gap, and how large the gap can grow in general, remains open. Such a characterization would tell agent designers when generation-based context compaction can outperform selection-based context compaction."
3. **Repeated context compaction.** Extending the single-invocation model to a sequence, where "a later context compaction acts on a state that contains the summaries produced by earlier context compactions"; the goal is "how the error on a fixed set of queries grows with the number of context compactions". Codex's own warning is the motivating evidence.
4. **Computationally efficient context compaction.** "A budget that is attainable in principle may not be attainable by a realistic context compaction algorithm such as an LLM-based summarizer. Determining which optimal budgets remain attainable when the condenser and the interpreter must run in polynomial time, or when the interpreter is an LLM call, is an open problem." Concrete first question: "whether an LLM can reliably simulate the decoding procedure of a sketch placed in its context."

Also asserted (not an open problem): the theorem "helps an agent designer calculate the minimum context compaction budget required for a given set of possible future queries or a distribution over future queries", and prior work has computed one-way complexity for **set membership** and **equality** queries [26].

---

## 7. Appendix A — "Context Compaction in the Wild"

**What they inspected.** Anthropic's Claude API context compaction endpoint [6] with **Opus 4.8**, on **set membership** queries over the **Malicious URLs dataset** [41]. Method: sample **15,000 URLs** uniformly; set the endpoint's compaction threshold to **50,000 tokens** (the URLs take about **500,000 tokens**, so compaction triggers); run one compaction; ask **200 membership queries**, half on members (probing false negatives) and half on non-members from the dataset (probing false positives), each in a separate request containing only the compacted context, answered as structured JSON yes/no; three seeds. The compaction prompt **tells the endpoint its workload**: "Compact the conversation so as to minimize the number of membership queries you answer incorrectly. Use whatever representation best achieves this." "The context compaction budget of a run is the size in bits of the natural language summary the endpoint returns." Code: `github.com/jadidbourbaki/context-compaction-experiments`.

**Reference points.** "any approximate membership tester with no false negatives and false positive rate at most `ε` requires at least `N log_2(1/ε)` bits in the worst case", and "A Bloom filter attains this bound within a factor of `log_2 e ≈ 1.44`". Figure 2 plots Bloom error `½ e^{−(ln 2)² B/N}` and the information-theoretic lower bound `½ 2^{−B/N}` against `B`, with `N = 15,000`.

**Findings.** The endpoint produces "a summary of about 14 kilobits" and answers "with error rates `0.505`, `0.535`, and `0.555` across the three seeds". Because the error counts false positives and false negatives together, "a context compaction algorithm that forgets every item and always answers no has an error rate of `0.5`. Every run lands on the random guess line." A Bloom filter of the same size "errs on about a third of the queries". Table 2: Seed 42, 14.3 Kbits, FPR 0.04, FNR 0.97; Seed 43, 13.6 Kbits, 0.28, 0.79; Seed 44, 14.8 Kbits, 0.48, 0.63; **No context compaction**, 7280 Kbits, 0.00, 0.04 (total error 0.02). The FP/FN split "varies from seed to seed", "consistent with a context compaction algorithm that retains no membership information". The **control** establishes causation: "Information lost during context compaction therefore causes the error in the main experiment." The reproduced summaries show the model stating it cannot store the set and falling back to "a description of the set's general character".

**The authors' own limits on the claim.** "This experiment does not demonstrate that context compaction algorithms for set membership query workloads necessarily perform significantly worse than a Bloom filter in real-world use cases. This experiment merely captures a snapshot of one deployed endpoint … at the time of writing." Other endpoints, future versions, a different LLM, or a different prompt "may perform differently". Follow-on questions they raise: closing the gap "may require context compaction that produces sketch-like representations. One path is a tool that maintains a sketch outside the LLM's context. Another path is placing a sketch's raw state in the context together with decoding instructions. Whether an LLM can execute such decoding reliably is an open question."

---

## 8. What this licenses for sctxx

### 8.0 Which game is sctxx playing?

The paper's axis is **output shape**, not who computes it, so be precise. **The deterministic pipeline (`--llm none`)** performs a Definition 1 move — masking, tier budgeting, tail selection, top-`k` ledger rows and `[evt a–b]` citations are a subset choice with additive size under a budget — but the **rendered artifact** is not "an unordered subset `S ⊆ X`": L0/L1 aggregate (`×4`, `340 lines`, "FAILED (2) at evt 4381"), emitting tokens that appear in no item, and L3 emits `expand` commands. Taken as a whole the deterministic artifact is a **deterministic `GEN` algorithm** whose condenser is Rust and whose interpreter is the receiving agent plus an LLM. **The fold** is also `GEN`: its condenser is (LLM fold → typed ops → deterministic apply/render) and its interpreter is the receiving agent plus an LLM; §8.4's validation gates (text ≤ 60 words, verbatim quotes, in-range sources) sharply restrict the reachable output space, but it is not `SELECT`.

Consequence: `SELECT ⊆ GEN` means the deterministic path is **not a weaker class** than the fold. Determinism is orthogonal to the taxonomy; "deterministic first" is a cost, reproducibility, and privacy argument that the paper neither supports nor refutes. **Do not dress ADR 0007 in this paper's language.**

### 8.1 Actionable, ranked (each tied to a section or theorem)

**A1 — Treat the deterministic layers as the bit floor, and never let the fold delete them.** *(§4.3 caveat 1; Theorem 1 proof.)* The paper states outright that a **lower** bound "bounds every `GEN` algorithm, regardless of how its interpreter is computed", while an **upper** bound only "exhibits some `GEN` algorithm". Bits absent from the artifact are unrecoverable by any interpreter; bits present can always be ignored by a bad one. Verbatim quotes, exact error strings, exact commands, paths and `[evt a–b]` pointers are the content whose presence is necessary under any regime. This is the strongest theoretical support the current architecture gets, and it is an argument for keeping L0/L1 evidence deterministic and non-droppable once the fold runs.

**A2 — Do not expect the fold to capture `GEN`'s advantage; the advantage is encoding, and the fold cannot encode.** *(§3.2 restriction 3; Theorem 3 proof.)* The `Θ(log n)` gap is won by sending an `n`-bit **indicator vector** instead of retaining `n` items of `log_2 n` bits each. sctxx's fold emits prose items of ≤ 60 words with event ranges — it is structurally incapable of emitting a sketch. So sctxx sits on the `SELECT` side of Theorem 3's separation in **both** modes. If such a workload ever matters, the fix is a **new deterministic condenser** (typed index, sketch, compact table), not a better prompt. This is the sharpest design consequence in the paper.

**A3 — Replace "compression ratio" with per-query-family error against a stated optimum, plus a no-compaction control.** *(§5 opening; Appendix A method and Table 2.)* The paper's operational definition of quality is that "the minimum context compaction budget for answering these queries within a target error is equal to the one-way communication complexity of the induced communication problem at the same error", and Appendix A turns that into a procedure: declare the workload, compare against a same-size optimal-per-family baseline (Bloom) **and** the information-theoretic lower bound, and include a **no-compaction control** to separate "compaction lost it" from "the model could not answer anyway". §15.2's metrics should gain a per-query-family table with a control row. A raw compression ratio is unlicensed as a quality claim: §5's disjointness example shows the required budget can be `Ω(Nm)` bits — "no better than storing the dependencies uncompressed".

**A4 — The artifact's pointers and S5 reconciliation are the escapement the paper does not model; that is where the leverage is.** *(Definition 3: `Int : Σ^{≤B} × Q → Out`; Table 1 has no retrieval row.)* sctxx's handoff is not a one-way message: L3 ships `sctxx expand` invocations, the preamble instructs the receiver to run verify-first commands, and S5 lets the repository override the artifact. The receiver therefore obtains information that is not in the message, so **the one-way lower bounds do not bound sctxx end-to-end**. The paper analyzes neither retrieval nor multi-round protocols, so this is a **design inference, not a theorem**: investment in exact pointers, `expand`, and repository reconciliation buys recall that no L0–L3 budget increase can buy.

**A5 — Report the regime you are actually in; stop implying the probe score is a guarantee.** *(Definition 2 and its ordering note; §4.3 caveat 2; §5 open problem 1.)* The probe loop tunes the fold against probes generated from the transcript and re-folds failed probes (§10.3) — optimization against an empirical `μ`, i.e. the **stochastic** regime. Definition 2 is explicit that a compactor designed for `μ` can still be bad on a rare input, and "an oblivious adversary can choose that particular bad input". The deployed consumer, which reads the handoff before deciding what it needs, is closer to the **adaptive** adversary that §4.3 excludes and §5 leaves open. Report `probe_score` as a stochastic-regime estimate, never as a bound on worst-case handoff fidelity.

**A6 — Convert units before quoting any bound.** *(§4.1.)* `B` symbols over an alphabet with `|Σ| > 1` are "equivalent to `B log|Σ|` bits". sctxx's budgets are 4-bytes-per-token estimates and Definition 1's `B` is token-like item size, not bits; Appendix A's bound is `B/N` bits per recorded item. Any sentence of the form "our 8,000-token budget exceeds the bound" must name the unit and the `N` it is per.

**A7 — Do not promise that a bigger artifact helps.** *(Theorem 1, both properties; §5.)* The equivalence is per query family and per target `ε`; there is no monotone "more tokens ⇒ better" result. The paper's recommendation is to *calculate* the minimum budget for a *given* set of future queries — which requires naming the query family, the step sctxx's docs currently skip.

### 8.2 What this does NOT license

- **No rule for splitting budget between the deterministic layers and the fold.** Table 1's caption is the paper's only statement about hybrids — "stack a `SELECT` layer with a `GEN` summarization fallback when the `SELECT` budget is exhausted" — and says nothing about optimal allocation, thresholds, or when the fallback fires. The 1,200-token L0 / remainder L1 / 150-token L3 split remains an engineering choice with no theoretical warrant.
- **No proof the fold beats the deterministic artifact.** `SELECT ⊆ GEN` is a class inclusion under a *different cost model*; it does not say an LLM summarizer beats a subset on any real distribution, and Appendix A's `GEN` endpoint scored no better than a random guess on the one workload measured, with the authors' own snapshot caveat attached.
- **No bound on sctxx's deployed error.** Theorem 1 needs a fixed `μ` (or an oblivious adversary) and a one-shot message. Neither is established for "what the next agent needs", and the deployed setting is plausibly adaptive, where Theorem 1 "does not apply".
- **No claim that Theorem 3's gap applies to coding-agent queries.** Which query sets admit a gap, and how large it can grow, "remains open"; Theorem 3 is one artificial family at zero error.
- **No claim that retrieval or pointers are optimal.** The paper never models them (A4). Outside the model is not proven better.
- **Nothing about repeated compaction, multi-agent handoff, or verification.** Repeat-compaction error growth is open problem 3; provenance and repository reconciliation are not in the model at all — its items are atomic units of information with no notion of being checkable against an external environment.
- **No support for citing this paper as endorsement of "deterministic first".** Per §8.0, the axis is output shape, not who computes it.

---

## 9. Verdict

Worth the read, and it changes three things. **First**, it supplies the right vocabulary and prevents a category error: determinism is not `SELECT`, an LLM is not `GEN`, and sctxx is `GEN` in both modes because its artifact is a rendering, not a subset. **Second**, it identifies the one property worth building on — lower bounds are caveat-free, upper bounds are not (§4.3) — a clean theoretical statement of "keep the evidence; the model may ignore it, but absence is fatal". **Third**, it says `GEN`'s real advantage is compact *encoding* (§3.2, Theorem 3), which sctxx's prose-shaped fold cannot reach, while Appendix A shows a deployed `GEN` endpoint performing indistinguishably from random guessing on membership. The honest posture: keep the deterministic artifact as the floor and the default; keep the fold as an unproven overlay; stop marketing compression ratio; measure per-query-family error against a stated optimum with a no-compaction control; and recognize that our strongest answer to the lower bounds is the part of the design the paper does not model — exact pointers and repository reconciliation. The theory does not prove the current architecture. It does say precisely which parts of it are necessary and which are taste.
