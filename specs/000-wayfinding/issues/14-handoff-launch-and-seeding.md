# Find a way to start a fresh agent session with a handoff already in its first turn

Type: research
Status: resolved (2026-09-11) — see [ADR 0004](../../../docs/adr/0004-handoff-launch-and-seeding.md)

## Question

Block `specs/024-m8-interactive-tui/` (FR-018, FR-019) needs the last mile: launch a **new** session of
Claude Code, Codex CLI, or Pi with the extracted handoff already loaded, so the receiving agent's first
turn knows the goal, the constraints, and the next action. What is the actual mechanism, per agent, on
the versions installed here?

## What is already known

The maintainer's launcher (`~/.local/bin/aiagents/launch-coding-agent`, 4,704 lines of uv/Python/
Textual) launches agents across a provider matrix but **has no seeding channel**:

- Its `LaunchSpec` is `argv, env, cwd, preview, unset_env, claude_provider_mode`; execution is
  `os.chdir(spec.cwd); os.execvpe(argv[0], argv, env)`.
- The only user input path is a free-text "extra CLI args" field, `shlex.split` into `argv`. There is
  no `--prompt`, no file, no stdin.
- Per-agent construction is thin: `claude` → `("claude", *extra)`; `codex` → `build_direct("codex", [])`;
  `pi` → `build_direct("pi", ["--provider", …, "--model", …])`; others through wrapper scripts.
- **The only context pre-loading that exists today is the cwd** — point it at the project and the agent
  picks up `AGENTS.md`/`CLAUDE.md` on its own. `--nc` on Pi only *disables* that.

So "create a fresh session with the extracted context pre-loaded" is not a thing that can be reused.
It has to be designed, per agent, and verified — which is why this is a ticket and not a task.

## Candidate mechanisms to test

1. **Positional prompt.** `claude "<handoff>"`, `pi "<handoff>"` — does the CLI accept a first-turn
   prompt argument, and is there a length limit that a 8 KB handoff would exceed?
2. **Non-interactive exec form.** `codex exec "<handoff>"` — starts a session that runs the prompt. Does
   it leave a resumable session behind, and does it behave when the handoff is large?
3. **Artefacts in cwd, prompt names them.** `sctxx extract --out .sctxx/` then launch with a one-line
   prompt (`read .sctxx/handoff.md and continue`) or, where available, an append-system-prompt flag.
   This avoids argument-length limits and keeps the handoff on disk where `expand` can reach it.
4. **Anything version-specific** a CLI offers for injecting context (system prompt, context file,
   `--append-*`).

## Constraints that bound the answer

- The handoff must be passed as **argv or a path**, never as interpolated shell text: transcript
  content is data and must not become part of a command line (constitution I, `AGENTS.md` rule 5).
- sctxx must not write into an agent's session store — launching is the agent's own job.
- A fallback must exist for every agent: the cwd route works even when nothing else does, so the
  feature degrades rather than fails.

## Evidence to produce

For each of Claude Code, Codex CLI and Pi, on the versions installed on the M1 Max (`sctxx doctor`
prints them): the exact command that starts a new session with a given prompt, the maximum practical
prompt size, whether the session is left resumable, and the fallback. Record the version next to each
result — these CLIs change flags without notice, which is why §9.3 already keeps command templates in
config rather than in code. Unblocks the `plan` of block 024.

---

## Resolution

**Answered for all three agents, on the versions installed on this machine.** Full evidence, the
per-agent table, the fallback rule, and the constraints that follow:
**[ADR 0004 — Handoff seeding](../../../docs/adr/0004-handoff-launch-and-seeding.md)**.

The short version, with versions probed 2026-09-11:

| Agent | Version | Channel | Handoff travels as |
|---|---|---|---|
| Claude Code | 2.1.268 | `--append-system-prompt-file <path>` (exists, undocumented in `--help`) + positional prompt | path |
| Codex CLI | 0.153.4 | `codex "<pointer>"` interactive; `codex exec -` reads instructions from **stdin** | argv pointer / stdin |
| Pi | 0.85.1 | `--append-system-prompt <path>` (reads the file when the value exists on disk) | path |

Two findings from the ticket's candidate list that change how the block is built:

1. **Candidate 3 (artefacts in cwd, prompt names them) is not the fallback — it is the shape.** The
   whole artifact never travels inline on any channel: only a one-line pointer crosses argv, the
   artifact crosses as a path or over stdin. That removes `ARG_MAX` from the design entirely and keeps
   the handoff on disk where `expand` can reach it.
2. **Claude Code fails lazily and silently on an unreadable file flag** — a `doctor` run with a
   nonexistent `--append-system-prompt-file` succeeds exactly like one with a real file. The launch must
   therefore pre-check every path it names; sctxx validates what the agent will not.

Two candidate mechanisms were eliminated as *primary* channels: the interactive Codex form has no
file or stdin channel, and the maintainer's launcher has no seeding channel at all, so nothing there
was reusable. Every row degrades to the cwd route, which asks nothing of the agent and therefore cannot
break.

**Verification status: probed, not yet verified end-to-end.** Flag existence, usage strings, and Pi's
file-vs-text rule are confirmed by argument-parse probes and by Pi's source. A real launch of each
agent with a real artifact is a task in block 024, because it starts sessions and spends tokens.
