# Decide whether the sidecar directories beside a Claude Code session are evidence

Type: research
Status: open

## Question

A Claude Code session is not one file. Beside `<session-id>.jsonl` there are up to three directories
that Claude Code writes itself, and **sctxx reads exactly one of them, by request**:

| Directory | What it holds | sctxx today |
|---|---|---|
| `<session-id>/subagents/` | the subagent transcripts, as full `.jsonl` sessions plus `.meta.json` | read only with `--include-sidechains`, off by default |
| `<session-id>/tool-results/` | every tool output **untruncated**, as `<id>.txt` | not read at all |
| `<session-id>/custom-title.json` | the user's own title for the session | not read |
| `~/.claude/file-history/<session-id>/` | `<hash>@vN` — the **exact contents** of every file version the session produced | not read at all |
| `~/.claude/session-env/<session-id>/` | (empty on the session measured) | – |

Measured on the 103k-event session (`1367d688-…`, 225 MB):

```text
subagents/       4 entries      (2 transcripts + 2 meta)
tool-results/   21 files, 6.2 MB
file-history/  747 files,  16 MB
```

## Why this matters

**`tool-results/` answers the truncation question.** sctxx caps every tool row at 2,000 tokens
(`mask.rs`, from Codex's `TOOL_OUTPUT_TOKENS`) because a transcript cannot carry a 100 KB compiler
dump. That cap is right — and it is a *lossy* decision made against a file that is sitting on disk,
untruncated, beside the session. The same is true after the DeepSeek Harness survey raised pruning:
their pruner throws the middle away, and Claude Code has already kept it. A pointer to the real file
is strictly better than a marker saying something was removed.

**`file-history/` answers "what does this file actually look like".** The file ledger infers what
happened from `apply_patch` headers and tool arguments — that is what sctxx does today, and it is why
the artifact says things like "45 files the session worked on no longer exist". `file-history` has the
content itself, versioned, per session. For the Active Workset (T2424) and for reconciliation, exact
beats inferred.

**`subagents/` is session context that is currently invisible.** `--include-sidechains` exists and is
default-off, which is the right default for cost — but a handoff whose work was largely delegated to
subagents would describe the main thread only. Whether that matters is a measurement, not an opinion.

## Decide

1. **`tool-results/` pointers.** Should a truncated tool row carry the path of its untruncated
   original, so `sctxx expand <artifact> <evt>` (or a new flag) can print the whole thing? This is
   cheap, local, provenance-preserving, and removes the only place sctxx knowingly destroys evidence.
2. **`file-history/`.** Is it a source for the file ledger and the Active Workset, or a second
   reconciliation input? It is a private Claude Code layout, so it needs the same clean-room care as
   the adapter itself (AGENTS.md rule 2 — public docs and on-disk format only).
3. **`subagents/`.** Does the default need to change, or does the artefact need to *say* that sidechains
   were excluded? Silence is the worst option: a reader cannot tell "no subagents" from "not read".

## Evidence to produce

For the session above: how many of the 2,000-token-capped rows have an untruncated original, how much
of the ledger's file story `file-history` can confirm or contradict, and how much of the work happened
in subagents. Then a decision per directory, not one for all three.

## Clean-room note

Layouts and file names are observable facts about files on this machine, which is what the adapter is
allowed to use. Nothing here is read from leaked source, and none of it may be.
