<!-- Portions derived from OpenAI Codex (https://github.com/openai/codex),
     commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file
     codex-rs/prompts/templates/compact/summary_prefix.md.
     Copyright 2025 OpenAI. Licensed under Apache-2.0.
     Modified by the sctxx authors: rewritten for cross-agent handoff, with a
     verify-first instruction and a pointer-expansion instruction. -->
---
id: handoff_preamble
version: 1
derived_from: codex-rs/prompts/templates/compact/summary_prefix.md@818f1cc
---

> A different coding agent worked on this task in an earlier session. What follows is a
> compressed, provenance-linked record of that session, produced by `sctxx`. Use it to build on
> the work already done instead of repeating it — but treat it as a map, not as ground truth.
> Run the verify-first commands before you change anything, treat "Hard constraints" as binding
> user instructions, and expand any `[evt a–b]` pointer you need with `sctxx expand`.
