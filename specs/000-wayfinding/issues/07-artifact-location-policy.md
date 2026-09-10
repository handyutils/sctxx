# Choose where handoff artifacts live and whether sctxx hides them from git

Type: grilling
Status: open

## Question

Artifacts default to `.sctxx/` in the working repository. Should sctxx add `.sctxx/` to `.git/info/exclude`
automatically on first write, only print a hint, or default to a cache location outside the repo?

Options:

1. Write `.sctxx/` and add it to `.git/info/exclude` automatically.
2. Write `.sctxx/` and print a one-line hint.
3. Default to the user cache directory; `--out` opts into the repo.

Speak for yourself. Do not let the agent answer this ticket.

Spec reference: `docs/SPEC.md` §12.1, §19 item 6.
