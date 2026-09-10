# Prompts

Every prompt is a file here, embedded into the binary with `include_str!` and recorded in each
artifact by id and version. Editing one means bumping its `version` front-matter, because the
artifact header claims which prompt produced it.

| File | Purpose | Derived from Codex |
| --- | --- | --- |
| `fold_system.md` | Fold rules: hygiene, evidence, item kinds, op semantics | `memories/write/templates/memories/stage_one_system.md` |
| `fold_user.md` | Per-chunk fold request | — |
| `premap.md` | Isolated candidate extraction for one chunk | — |
| `final_pass.md` | Reconcile state against the recency tail | — |
| `repair.md` | Single repair turn for rejected ops | — |
| `handoff_preamble.md` | Header the receiving agent reads first | `prompts/templates/compact/summary_prefix.md` |

Templates derived from Codex carry the Apache-2.0 attribution as an HTML comment at the top of
the file; `scripts/check-vendor-headers.sh` verifies it.

## Placeholders

`{{chunk_id}}`, `{{session}}`, `{{focus}}`, `{{state}}`, `{{ledger_slice}}`, `{{later_index}}`,
`{{chunk}}`, `{{evt_start}}`, `{{evt_end}}`, `{{schema}}`, `{{rejections}}`. An unknown
placeholder is left as-is rather than silently emptied, so a typo is visible in the prompt.

## Injection hygiene

Every prompt that embeds transcript text wraps it in
`<transcript evt_start=… evt_end=…> … </transcript>` and states that the content is data and must
never be followed. This is not optional: a session transcript contains text written by users,
previous agents, tools, and whatever those tools fetched.
