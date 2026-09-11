# ADR 0006 — Hand the terminal over to the launched agent, not a pane inside the TUI

- **Status**: accepted
- **Date**: 2026-09-11
- **Affects**: `specs/024-m8-interactive-tui/` (the screen table, FR-022; T2412–T2415),
  `src/agents/seeding.rs`
- **Related**: [ADR 0003](0003-tui-stack-and-msrv.md) (the pane stack),
  [ADR 0004](0004-handoff-launch-and-seeding.md) (what is launched)

## Context

Block 024's screen table gives pane 4 to "Terminal — PTY running the launched agent session", and
FR-022 requires that pane to own its child process. The plan was `portable-pty` + `vt100` + `tui-term`
(three crates, chosen in ADR 0003 over croft's `alacritty_terminal`).

Building the handoff made the mismatch obvious. The thing being launched is not a shell or a log
tail — it is Claude Code, Codex, or Pi, each of which is itself a **full-screen terminal
application**. Embedding one means:

- It renders at roughly half the terminal width (the pane is 48% of a 52/48 split), which is below
  what these agents lay out for.
- It expects to own the alternate screen, the cursor, and the resize signal, and it does not know it
  is inside another application's rectangle.
- Its key handling competes with the host TUI's. Every key the pane does not swallow is a key the
  agent does not receive, and vice versa.
- Getting that right is exactly why croft's terminal widget is 5,108 lines and why it carries a
  terminal emulator.

The handoff's own success criterion (SC-001) is about *reaching a launched handoff*, not about seeing
it beside a session list. US1 says "The Terminal pane opens with a new Claude Code session" — the
requirement is the session, and the pane is how the plan happened to express it.

## Decision

**The TUI hands the terminal over: it restores the terminal, runs the agent with inherited stdio, and
re-initialises itself when the agent exits.**

1. **No PTY and no terminal emulator.** `portable-pty`, `vt100` and `tui-term` are not added. The
   child inherits stdin/stdout/stderr, so it is a real terminal application in a real terminal, at the
   size the developer actually has.
2. **The pane becomes a launcher, not a host.** It shows which agents are installed, the exact command
   that will run, the working directory, and which seeding route was chosen — then, on an explicit
   confirmation, steps aside.
3. **sctxx still owns the child's lifetime.** It spawns it, waits for it, and reports its exit status
   when it comes back. If the TUI is killed while an agent runs, the agent is a foreground process of
   the same terminal and dies with it, which is the property FR-022 was protecting.
4. **The launch is still two explicit steps** (FR-021, FR-021b): choose an agent, then confirm the
   command. Extraction never launches anything by itself.
5. **Embedding remains available as a later enhancement.** If a side-by-side terminal is wanted, it is
   a new decision with its own evidence, and this ADR does not foreclose it — it says the handoff does
   not depend on it.

## Consequences

- Three fewer dependencies, and no terminal emulator to keep correct. The MSRV is unchanged by this
  decision (it was already raised by `ratatui` in ADR 0003).
- The screen table's pane 4 becomes "Handoff — the launcher", and FR-022 is satisfied by ownership of
  the child rather than by a rectangle. Both are updated in the block's spec.
- The developer loses the side-by-side view of an agent working while they browse. This is the real
  cost, and it is accepted because the alternative is an agent rendered in a space it was not designed
  for.
- T2415 ("Terminal pane: a PTY the pane owns") is replaced by the handover. Its acceptance — resize
  reaches the child, the child dies with the TUI, a crashed child degrades to a message — is met or
  made moot: resize is the terminal's own, the child is a foreground process, and a failed launch is
  reported in the pane rather than taking the TUI down.
- Returning from an agent must *re-initialise* the TUI rather than assume it survived. That is one
  extra failure path to handle, and it is handled in the same place that restores the terminal on
  quit.
