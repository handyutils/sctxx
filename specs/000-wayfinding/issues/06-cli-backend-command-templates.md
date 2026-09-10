# Verify non-interactive command templates for agent CLIs used as LLM backends

Type: research
Status: open

## Question

For the installed versions of `claude`, `codex`, and `pi` on the M1 Max: what exact non-interactive
invocation returns plain model text or JSON for a prompt on stdin, with tool use and file access disabled,
and where in stdout the text lives? Record versions, commands, and captured outputs (no private content).

Unblocks: `specs/010-m3-llm-backends/`. Spec reference: `docs/SCTXX-SPEC.md` §9.3.
