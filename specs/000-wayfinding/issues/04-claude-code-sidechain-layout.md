# Map Claude Code subagent and sidechain storage across versions

Type: research
Status: open

## Question

How do current Claude Code versions store subagent (sidechain) transcripts: inline entries with
`isSidechain: true`, separate files next to the session, or both? Which fields link a spawn to its result,
and how do compaction boundaries (`logicalParentUuid`) interact with them?

Work only from session files on the maintainer's own machine (redacted before being committed) and public
Anthropic documentation. Never consult leaked Claude Code source or its forks. Record agent versions,
fixture paths, and observed behavior as verified facts; everything else is labeled hypothesis.

Unblocks: `specs/003-m1-ir-and-claude-code-adapter/`. Spec reference: `docs/SCTXX-SPEC.md` §6.3, §19 item 1.
