# Find a way to start a fresh agent session with a handoff already in its first turn

Type: research
Status: open

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
