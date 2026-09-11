# sctxx CLI reference

## Session references

```text
<ref> := [<agent> ":"] <selector>
<agent> := claude | codex | pi
<selector> := <uuid> | <id-prefix, >= 6 chars> | last[:N] | <path to .jsonl or .jsonl.zst>
```

- A filesystem path is used directly and the agent is detected from the file's content.
- With an agent prefix, only that store is searched; without one, all of them are.
- `last` means the most recent session recorded in the current directory; `last:3` the third most
  recent. Add `--any-project` to ignore the directory.
- A prefix that matches several sessions exits **3** and prints the candidates as JSON on stdout.

## Commands

### `sctxx extract <ref>` — the main command

| Flag | Default | What it does |
| --- | --- | --- |
| `--mode fast\|standard\|full` | `standard` | `fast` skips the premap pass; `full` is reserved for the probe loop (roadmap M5) and currently behaves as `standard` |
| `--llm none\|auto\|cli:<agent>\|api:<provider>[/<model>]\|mock` | `auto` | `none` produces the deterministic artifact and calls no model |
| `--budget <TOKENS>` | `8000` | Artifact budget, excluding the recency tail |
| `--tail <TOKENS>` | `12000` | Recency tail budget |
| `--chunk-tokens <TOKENS>` | `24000` | Tokens per fold chunk |
| `--focus "<TEXT>"` | — | What the next agent wants to do; biases extraction |
| `--repo <PATH>` | session cwd, else `.` | Repository to reconcile against |
| `--no-verify` | off | Skip reconciliation |
| `--strict` | off | Exit 7 if the repository contradicts the artifact |
| `--out <PATH>` | stdout | A directory writes all five files; `.md` or `.json` writes one |
| `--format md\|json\|both` | `md` | Format when writing to stdout |
| `--layers L0,L1,L2,L3` | all | Which layers to render |
| `--include-sidechains` | off | Include subagent transcripts |
| `--since-compact` | off | Start from the newest provider compaction boundary, keeping its summary as a low-trust seed. Reports the boundary on stderr; a session that never compacted is a notice, not an error |
| `--max-bad-lines <RATE>` | `0.02` | Fraction of lines allowed to fail parsing. Raise it for a session written by a provider version newer than sctxx; unknown lines are kept as events either way |
| `--keep-reasoning` | off | Keep readable model reasoning in masked rows |
| `--keep-system` | off | Keep system and unrecognized events |
| `--redact default\|strict\|off` | `default` | `off` applies only with `--llm none` |
| `--concurrency <N>` | `4` | Parallel premap calls |
| `--progress text\|json` | `text` | Progress format on stderr |
| `--dry-run` | off | Print the plan and estimated tokens, then exit |

### Other commands

```sh
sctxx list   [--agent A] [--any-project] [--limit N]
sctxx find   <query> [--agent A] [--any-project] [--limit N]
sctxx show   <ref> [--view raw|masked|ir] [--range A..B] [--active-branch-only]
sctxx expand <ref|artifact-path> <A..B>... [--context N]
sctxx verify <artifact> [--repo PATH] [--strict]
sctxx redact <path|-> [--strict] [--out PATH] [--check]
sctxx skill  install|uninstall|print [--target A]... [--scope user|project] [--force]
sctxx schema handoff|state|ops|ir
sctxx doctor
sctxx update [--check]
```

`sctxx --tui` is a top-level flag rather than a subcommand: it takes no subcommand of its own and is a
complete invocation. It needs a real terminal, and fails with exit 2 if stdout or stdin is piped.

`sctxx update` updates an installed copy the way it was installed — npm if the executable is inside a
`node_modules` directory, `cargo install --force` if it is in cargo's bin directory. It prints what it
detected and the exact command before running it; `--check` stops there. An install it did not make (a
distribution package, a container, a checkout build) is refused, with both installer commands named.

Every command accepts `--json`, `--quiet`, `--claude-root`, `--codex-root`, and `--pi-root`.

## Exit codes

| Code | Meaning | What to do |
| --- | --- | --- |
| 0 | success | — |
| 1 | unexpected error | read stderr |
| 2 | usage error | fix the arguments |
| 3 | ambiguous session reference | candidates are on stdout as JSON; show them to the user |
| 4 | session not found | run `sctxx list` |
| 5 | parse failure rate above the threshold | the file may not be a session file |
| 6 | no usable LLM backend | run with `--llm none`, or `sctxx doctor` to see why |
| 7 | the repository contradicts the artifact and `--strict` was set | re-extract |

## Conventions

- **stdout is the payload**: the artifact, the JSON, the event text, or the path that was written.
  Progress and diagnostics go to stderr, so `sctxx extract ... > handoff.md` is always safe.
- No interactive prompts. sctxx never blocks an agent waiting for input.
- Timestamps are RFC 3339 UTC; paths in output are POSIX-normalized.

## Environment

| Variable | Effect |
| --- | --- |
| `CLAUDE_CONFIG_DIR` | Claude Code store root becomes `$CLAUDE_CONFIG_DIR/projects` |
| `CODEX_HOME` | Codex store root |
| `SCTXX_LLM` | Default `--llm` value |
| `ANTHROPIC_API_KEY`, `OPENAI_API_KEY` | Enable `api:anthropic` / `api:openai` |
| `SCTXX_BASE_URL`, `SCTXX_API_KEY`, `SCTXX_MODEL` | Configure `api:compat` for any OpenAI-compatible endpoint |
