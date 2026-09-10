# Decide whether host mode ships as a CLI protocol or MCP first

Type: grilling
Status: open

## Question

For environments without an API key or usable agent CLI, sctxx can hand LLM steps to the calling agent.
Should that ship first as the stepwise CLI protocol (`fold init/next/apply/finish`) or first as
`sctxx mcp` tools?

Speak for yourself. Do not let the agent answer this ticket.

Spec reference: `docs/SCTXX-SPEC.md` §9.5, §19 item 5. Unblocks: `specs/018-m6-host-mode-and-mcp/`.
