@AGENTS.md

## Claude Code

<!-- Maintainers: keep project-wide rules in AGENTS.md so Codex, Pi, and other agents see them too.
     Only Claude Code–specific behavior belongs below. Path-scoped detail lives in .claude/rules/. -->

- Use plan mode before changing IR types (`src/ir/`), the ops/state/handoff schemas, or anything that spans
  more than one pipeline stage.
- Session fixtures can be many MB. Don't Read whole `.jsonl` files into context. Inspect them with
  `cargo run -- show <file> --view ir --range A..B`, `head -c 4000`, or `jq -c 'select(.type=="…")' <file> | head`,
  or delegate the survey to a subagent and bring back only the findings.
- Never open files under `~/.claude/projects/`, `~/.codex/sessions/`, or `~/.pi/agent/sessions/` on your own
  initiative — they are real conversations, including this one. Only when the user explicitly asks to build a
  fixture from one: pipe it through `cargo run -- redact <file> --strict` first and read the redacted output,
  never the original.
- Don't delete or accept insta snapshots in bulk; show the user the diff summary for snapshot changes in
  adapters and rendering.
- When the user asks to resume earlier work on this repo, check for `.sctxx/handoff.md` first; if it's missing
  and the binary builds, generate it with `cargo run -- extract last --out .sctxx/`.
