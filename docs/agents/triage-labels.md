# Triage Labels

The skills speak in terms of five canonical triage roles. This file maps those roles to the strings used
in this repo's tracker (`Status:` line on markdown issues, or GitHub labels if a ticket is promoted).

| Label in mattpocock/skills | Label in our tracker | Meaning |
| --- | --- | --- |
| `needs-triage` | `needs-triage` | Maintainer needs to evaluate this issue |
| `needs-info` | `needs-info` | Waiting on reporter for more information |
| `ready-for-agent` | `ready-for-agent` | Fully specified, ready for an AFK agent |
| `ready-for-human` | `ready-for-human` | Requires human implementation or a human approval gate |
| `wontfix` | `wontfix` | Will not be actioned |

Always `ready-for-human` in sctxx, regardless of specification quality: publishing to crates.io or npm,
changes to redaction or LLM egress, intake of fixtures from real sessions, and license/NOTICE or vendoring
boundary changes (high-risk class in the methodology §4).

Wayfinder tickets use a separate `Status:` vocabulary (`open` / `claimed` / `resolved`) and a `Type:` of
`research` / `prototype` / `grilling` / `task`. That is claim state, not triage.
