# Verify non-interactive command templates for agent CLIs used as LLM backends

Type: research
Status: resolved (2026-09-11) — the templates shipped in `src/llm/cli.rs`

## Question

For the installed versions of `claude`, `codex`, and `pi` on the M1 Max: what exact non-interactive
invocation returns plain model text or JSON for a prompt on stdin, with tool use and file access disabled,
and where in stdout the text lives? Record versions, commands, and captured outputs (no private content).

Unblocks: `specs/010-m3-llm-backends/`. Spec reference: `docs/SCTXX-SPEC.md` §9.3.

---

## Resolution

**Answered by the implementation, and re-verified on the versions now installed.** The shapes below are
`src/llm/cli.rs`'s `TEMPLATES` table; each row now carries the version it was verified against and the
version is reported in a backend failure, so a CLI that moves a flag is diagnosable from the error.

| Agent | Version | argv | Text lives at |
|---|---|---|---|
| Claude Code | 2.1.268 | `claude -p --output-format json --allowed-tools "" --no-session-persistence` | stdout JSON, key `result` |
| Codex CLI | 0.153.4 | `codex exec --skip-git-repo-check --sandbox read-only --ephemeral -` | stdout |
| Pi | 0.85.1 | `pi -p --no-session` | stdout |

Prompt on **stdin** for all three. Tool use and file access are disabled by combining an empty tool
allowlist (`claude`), a read-only sandbox and no-repo requirement (`codex`), and print mode (`pi`), with
the call running in an empty temporary cwd so an agent acting as a completion engine cannot see the
user's repository.

### The finding this ticket did not ask for

**Every completion used to leave a session in the user's own history.** The scratch cwd does not prevent
it: Claude Code records a session *per working directory*, so `cli:claude` wrote
`~/.claude/projects/<scratch-cwd>/<uuid>.jsonl` on every call, whose transcript was sctxx's own fold
prompt — and `sctxx list` then reported sctxx's private scratch calls as sessions. Five such sessions
were present on this machine, dated when the backend was being built.

Each CLI has a supported switch for exactly this, and all three are now in the table:
`--no-session-persistence` (Claude), `--ephemeral` (`codex exec`), `--no-session` (Pi). Because the
failure mode is silent — nothing errors, the junk simply appears in a list the user reads as their own
history — a unit test asserts every template still passes its switch and that the template count matches
the expected set, so a new backend cannot be added without one.

Verified today: the flags are documented in each CLI's `--help`, and `--no-session-persistence` is
accepted by 2.1.268 (`claude --no-session-persistence doctor` runs, `claude --invented-flag-x doctor`
does not). **Not** verified end to end: a live completion per backend, which spends the user's quota.
The first real `sctxx extract --llm cli:claude` on a session is that confirmation, and the
`~/.claude/projects` listing before and after is the evidence.

### Why it is resolved rather than open

The ticket existed to unblock `specs/010-m3-llm-backends/`, which has shipped; the argv, the stdin
contract, the tool/file lockdown, and the output location are all in code with versions attached. The
one item the ticket asked for that is not recorded here is a captured real completion per agent — the
contract is covered by the mock suite instead, and the live capture is the user's own acceptance test.
