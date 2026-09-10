# Fixture corpus

Provider session formats change without notice, so **these files are the contract**. Every adapter
behavior is pinned to a fixture and an `insta` snapshot of the resulting IR, so a format change
shows up as a reviewable diff rather than a silent regression.

## Policy

All fixtures here are **synthetic**. They were written by hand to reproduce the shapes observed in
real sessions, and contain no real conversation content.

A real session may only enter this directory when:

1. Its owner explicitly consented to publishing it.
2. It was run through `sctxx redact --strict`.
3. A human read the redacted result line by line. Redaction is pattern matching, not a guarantee.
4. The provider and agent version it came from are recorded in the table below.

Never commit a session file straight out of `~/.claude`, `~/.codex`, or `~/.pi`.

## What each fixture covers

| Fixture | Shape it pins |
| --- | --- |
| `claude/basic.jsonl` | The ordinary case: goal, read, plan, edit, failing test, repeated failure, summary title |
| `claude/rewind.jsonl` | A rewind: two siblings share a parent, and only the newer path is live |
| `claude/compact-boundary.jsonl` | `isCompactSummary` plus `logicalParentUuid` bridging a missing physical parent |
| `claude/sidechain-and-malformed.jsonl` | A `Task` subagent sidechain, an unknown entry type, and a truncated final line |
| `codex/basic.jsonl` | `session_meta`, harness context as a user message, developer message, reasoning, argv-array shell, `apply_patch`, `update_plan` |
| `codex/rollback.jsonl` | `thread_rolled_back` undoing the newest user turn and its work |
| `codex/ask-and-compaction.jsonl` | `request_user_input` paired into one human answer, a `compacted` line, an unknown item type |
| `pi/basic.jsonl` | A v3 tree, `bashExecution`, `toolCall`/`toolResult`, `modelChange`, a label |
| `pi/branch.jsonl` | An abandoned branch with a `branchSummary`; only the live path is active |
| `pi/v1-linear.jsonl` | A v1 linear session with a `compactionSummary` |

## Agent versions

| Provider | Version the shapes were modelled on |
| --- | --- |
| Claude Code | `2.1.7` (field names recorded in the fixtures themselves) |
| Codex CLI | `0.58.0`, rollout format at upstream commit `818f1cc` |
| Pi | session format v1 and v3, per the published `session-format.md` |
