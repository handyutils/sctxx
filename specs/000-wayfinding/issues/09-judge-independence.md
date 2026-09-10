# Decide how probes are judged when only one model family is available

Type: grilling
Status: open
Blocked by: [Decide whether v0.1.0 includes the LLM fold](03-v0-1-release-scope.md)

## Question

When the only available backend is one model family (for example only `cli:claude`), is same-family judging
acceptable for probe scores, or should sctxx fall back to deterministic probes only and report that the LLM
probe score is unavailable?

Speak for yourself. Do not let the agent answer this ticket.

Spec reference: `docs/SPEC.md` §10.3, §19 item 4. Unblocks: `specs/015-m5-probe-loop/`.
