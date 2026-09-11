# Research notes

Readings of the published work sctxx draws on. Each note records what transferred, what did **not**,
and what the measurement said once it was implemented — including where the result was worse than
the paper implied.

| Note | Source | Verdict |
| --- | --- | --- |
| [Knowledge Triage / The Compaction Cliff](2026-09-11-paper-knowledge-triage-typecompact.md) | [arXiv:2608.22752](https://arxiv.org/html/2608.22752v1), CIKM 2026 | **Adopted, substantially.** The framework transferred; the classifier did not. See §5.5 for the measured precision and recall on a real session. |
| [ARC — Addressable Recall Compaction](2026-09-11-paper-arc-addressable-recall.md) | [arXiv:2607.25066](https://arxiv.org/html/2607.25066v1) | **Adopted partially.** The pointer cost rule, exact-chunk recovery, the `K ≤ L` check, and the ban on all-visible citation lists. Not the word "lossless". |
| [Context Window Lifecycle](2026-09-11-paper-cwl-and-context-compression-repo.md) | [arXiv:2606.11213](https://arxiv.org/html/2606.11213v1) and [saminkhan1/context-compression](https://github.com/saminkhan1/context-compression) | **Framing only.** The paper's eviction policy is not implementable as written; the repo is narrower than its description — 0 grep hits for `transcript`, `episode`, `evict`, `compaction`. Its verification discipline transferred. |
| [Context Compaction Theory](2026-09-11-paper-context-compaction-theory.md) | [arXiv:2608.01326](https://arxiv.org/html/2608.01326v1) | **Design constraint.** The lower bounds bind every generative algorithm regardless of how its interpreter is computed; the upper bounds are existential. Stop marketing a compression ratio. |

Decisions taken from these are in [`../adr/`](../adr/), in particular
[ADR 0008](../adr/0008-deterministic-typed-layer.md).
