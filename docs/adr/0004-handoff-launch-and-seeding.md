# ADR 0004 — Handoff seeding: a path per agent, a one-line pointer as fallback

- **Status**: accepted
- **Date**: 2026-09-11
- **Affects**: `specs/024-m8-interactive-tui/` (FR-017 to FR-022, US1), the agent catalogue and launch
  layer of M8
- **Resolves**: [`14-handoff-launch-and-seeding`](../../specs/000-wayfinding/issues/14-handoff-launch-and-seeding.md)

## Context

M8's last mile is "the new agent is already working on it": choosing a target agent must launch a
**new** session of that agent whose first turn already carries the extracted handoff — the goal, the
constraints, and the next action. Without that, `sctxx` produces a file and leaves the developer
typing the same sentence into a fresh session, which is the problem block 024 exists to remove.

There was no mechanism to reuse. The maintainer's launcher
(`~/.local/bin/aiagents/launch-coding-agent`) builds a `LaunchSpec { argv, env, cwd, preview,
unset_env, claude_provider_mode }` and executes `os.execvpe`; its only user input is a free-text "extra
CLI args" field. **There is no `--prompt`, no file channel, and no stdin channel.** The only context
pre-loading that exists today is the cwd, which lets the agent find `AGENTS.md`/`CLAUDE.md` on its own.

So the mechanism had to be established per agent, against the versions actually installed. This ADR is
that investigation and its decision.

## Evidence (2026-09-11, Apple M1 Max, `aarch64-apple-darwin`)

| Agent | Version | Binary |
|---|---|---|
| Claude Code | `2.1.268` | `/Users/musichen/.local/bin/claude` |
| Codex CLI | `codex-cli 0.153.4` | `/opt/homebrew/bin/codex` |
| Pi | `0.85.1` | `/Users/musichen/.local/bin/pi` (`@earendil-works/pi-coding-agent`) |

### Claude Code 2.1.268 — a documented prompt, an undocumented file flag

`--help` gives `Usage: claude [options] [command] [prompt]`, so a positional prompt is the first turn,
and `--append-system-prompt <prompt>` is listed normally.

`--append-system-prompt-file <file>` and `--system-prompt-file <file>` **are not in `--help`**, but
they exist: they appear as prose inside the `--bare` documentation ("Explicitly provide context via:
`--system-prompt[-file]`, `--append-system-prompt[-file]`, …"). Argument parsing confirms them:

```text
$ claude --append-system-prompt-file
error: option '--append-system-prompt-file <file>' argument missing
$ claude --totally-not-a-flag
error: unknown option '--totally-not-a-flag'
```

**A `--version` probe cannot be used to test flag existence**: `claude --totally-not-a-flag --version`
exits 0, because version printing short-circuits before validation. This is recorded because it is an
easy way to produce a false confirmation.

The read is **lazy and unvalidated**. `claude --append-system-prompt-file <missing-file> doctor`
succeeds silently, exactly like the run with a real file. A path that is wrong, unreadable, or later
deleted therefore fails at session start, not at launch — nothing warns the developer at the moment
they could still fix it.

### Codex CLI 0.153.4 — a positional prompt, and instructions over stdin

`Usage: codex [OPTIONS] [PROMPT]`, and "If no subcommand is specified, options will be forwarded to the
interactive CLI" — so `codex "<prompt>"` opens the interactive TUI with that first turn. `-C/--cd <DIR>`
sets the working root.

`codex exec` is the non-interactive form, and its `[PROMPT]` documentation is the important part:

> Initial instructions for the agent. If not provided as an argument (or if `-` is used), instructions
> are read from **stdin**.

Codex has no system-prompt-file flag. `codex exec -` is nonetheless the strongest channel of the three
for the never-interpolate rule, because the handoff crosses on a file descriptor and never touches a
command line or the argument vector.

### Pi 0.85.1 — `--append-system-prompt` reads a file

`Usage: pi [options] [--] [@files...] [messages...]`, and:

> `--append-system-prompt <text>` Append text **or file contents** to the system prompt (can be used
> multiple times)

"Or file contents" is disambiguated by existence, in
`dist/core/resource-loader.js:17-31`:

```js
function resolvePromptInput(input, description) {
    if (!input) return undefined;
    if (existsSync(input)) {
        try { return stripBom(readFileSync(input, "utf-8")); } catch (error) { … return input; }
    }
    return input;
}
```

So `--append-system-prompt /abs/path/to/handoff.md` appends the file's contents, and the same function
serves `--system-prompt`. Pi also records the source (`appendSystemPromptSourcePaths`) so the loaded
file is visible in its UI, and it discovers `<cwd>/.pi/APPEND_SYSTEM.md` on its own. Note the
implication in the other direction: a *literal* string that happens to name an existing file is read as
a file, so sctxx must pass either a path it controls or text that cannot be one.

## Decision

**Seeding is a per-agent, version-pinned template table, and the handoff travels as a path — never as
interpolated content.**

Per agent, in preference order:

| Agent | Preferred channel | Command shape |
|---|---|---|
| Claude Code | file | `claude --append-system-prompt-file <abs artifact path> "<one-line pointer>"` |
| Pi | file | `pi --append-system-prompt <abs artifact path> "<one-line pointer>"` |
| Codex CLI (interactive) | argv pointer | `codex "<one-line pointer naming .sctxx/handoff.md>"` |
| Codex CLI (headless) | stdin | `codex exec -`, handoff piped to stdin |

**The universal fallback is the cwd route**, and it is always available: extract to
`<session cwd>/.sctxx/`, launch with cwd set to the session's cwd when it still exists, and pass a
one-line prompt naming the artifact. It needs no flag from any agent, which is what makes it a real
fallback rather than a hope — and it is the only channel that also works through a wrapper script that
swallows unknown flags.

Rules that follow, and that the implementation must hold to:

1. **The whole artifact never travels inline.** Only a one-line pointer does, over argv. The artifact
   itself moves as a path, or over stdin on the one channel that offers it. This keeps the launch clear
   of `ARG_MAX` questions entirely — a full L3 artifact is kilobytes to megabytes, and the limit would
   otherwise be a silent cliff — and it keeps the handoff on disk where `expand` can still reach it.
2. **The launch pre-checks anything it names.** Because Claude Code fails lazily and silently on an
   unreadable `--append-system-prompt-file`, sctxx verifies the path exists and is readable *before*
   spawning, and reports it in the pane. sctxx validates what the agent will not.
3. **No shell, ever.** The child is spawned directly with an argument vector; transcript text never
   reaches a shell string (FR-020, constitution I, `AGENTS.md` rule 5). The one-line pointer is
   sctxx-generated prose, and the artifact path is validated, not interpolated.
4. **Version-pinned, with a recorded fallback.** Each row carries the version it was verified on. On an
   unverified version the launch uses the cwd fallback and says so; it never guesses at flags whose
   behaviour it has not confirmed (the §9.3 discipline, applied to launches).
5. **Interactive by default.** US1 needs the developer to keep working, so the Terminal pane opens the
   interactive form. `codex exec -` is offered as an explicit headless choice, not as the default.
6. **Extraction and launch stay separate confirmations** (FR-021b), and the handoff is re-redacted
   immediately before egress (FR-021a). sctxx never writes into an agent's session store.

## Consequences

- The launch layer is data plus a spawn, not a set of per-agent code paths: a template table with a
  verification status per row, in the spirit of the command templates §9.3 already keeps in config.
- `codex`'s interactive and headless forms are different products, and the block says which is which
  rather than picking one and losing the other use.
- Nothing here depends on an undocumented flag being *load-bearing*: the undocumented
  `--append-system-prompt-file` is an optimisation over the cwd fallback, so if upstream removes it the
  feature degrades instead of breaking.
- Verification ran on 2026-09-11 against a real 58 KB artifact, and the three content-delivery rows
  **moved from "probed" to "verified"**: Claude Code's `--append-system-prompt-file`, Pi's
  `--append-system-prompt <path>`, and Codex's pointer route each caused the receiving agent to answer
  with the source session's id, which appears only inside the artifact. Evidence:
  [`specs/024-m8-interactive-tui/evidence/T2419.md`](../../specs/024-m8-interactive-tui/evidence/T2419.md).
- **What the verification found, and it is the important part:** neither interactive launch reached a
  first turn. Both stopped at the agent's **own trust prompt** for a directory it had not seen before
  ("Is this a project you created or one you trust?" / "Do you trust the contents of this directory?").
  That is correct behaviour, and it is exactly the prompt `--dangerously-bypass-…` would skip — which is
  why sctxx never passes that flag. A real handoff starts in the project the session was about, which
  the developer has already trusted, so this is an edge case rather than the normal path; the confirming
  pane now says it will happen so that it is not a surprise.
- Still unverified, deliberately: first-turn delivery in the *interactive* form past that prompt (it
  would mean accepting trust on the developer's behalf, or launching into a live project and letting the
  agent start working), and resumability (the runs exited before writing a session, and relocating the
  agent's home for the test also relocates its credentials).

**Residual risk.** All three CLIs change flags without notice, and Pi's existence-based disambiguation
means a path-shaped string is always a file — a behaviour to keep in mind if a future channel passes
free text. The mitigation for both is the same: pin the version, verify the launch, and fall back to
the cwd route, which cannot break because it asks nothing of the agent.
