---
paths:
  - "src/adapters/**"
  - "tests/fixtures/**"
  - "tests/adapters.rs"
---

# Adapters and fixtures

- Claude Code adapter: clean-room only. Sources allowed: redacted fixtures, public Anthropic docs, and
  observed behavior. Never consult leaked Claude Code source or forks, and don't search for them.
- Pi adapter: follow Pi's published `session-format.md`; no copied Pi code.
- Codex adapter: port semantics from the pinned commit via `src/vendor/codex/`, not ad-hoc copies here.
- Treat every provider field as optional. Unknown line types → `EventKind::Unknown` with raw JSON kept +
  a `Diagnostic`. No `unwrap`/`expect` on parsed data.
- Active-branch resolution is the risky part: Claude Code `parentUuid` trees (plus `logicalParentUuid` across
  compaction boundaries), Codex `ThreadRolledBack` replay, Pi `id`/`parentId` trees with a leaf = last tree
  entry. Every fix here needs a fixture that fails before the fix.
- Fixture files: name by scenario (`rewind-then-edit.jsonl`, `rollback-2-turns.jsonl.zst`), note the agent
  version in `tests/fixtures/<agent>/README.md`, and confirm the file was redacted (`--strict`) and reviewed.
- Adapter output is snapshot-tested (IR JSON + active indices). Review snapshot diffs line by line.
