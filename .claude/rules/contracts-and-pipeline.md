---
paths:
  - "schemas/**"
  - "src/cli/**"
  - "src/pipeline/**"
  - "src/llm/**"
  - "skill/**"
---

# Contracts and pipeline invariants

- stdout carries only the command payload; everything else goes to stderr. Exit codes follow spec §3.1
  exactly — don't add or renumber codes without a spec change.
- Schema-affecting changes (`handoff.v1`, `ops.v1`, `state.v1`, `ir.v1`): bump the version when the change
  isn't backward compatible, run `cargo xtask gen-schemas`, update snapshots, add a CHANGELOG entry.
- Flag changes: run `cargo xtask gen-skill` so `skill/references/` matches the binary.
- Fold ops are validated before apply (spec §8.4): known active ids, sources inside the current chunk range,
  constraint quotes matching a human message verbatim (NFKC + whitespace collapse + case fold), length caps,
  max 1 active `current_step` and 3 active `next_action`. One repair turn, then discard and log.
- Constraints are never dropped, only superseded by a later quoted user statement.
- Budgets are enforced in Rust at render time, not by asking the model to be brief.
- Redaction runs before every LLM request and on every LLM response; `--redact off` never applies to `api:`
  backends.
- `cli:` LLM backends run in an empty temp cwd; command templates live in config, not hardcoded strings.
- Any code path that embeds transcript text in a prompt uses the shared `<transcript>` wrapper.
