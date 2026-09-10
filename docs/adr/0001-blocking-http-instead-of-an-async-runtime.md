# ADR 0001 — Blocking HTTP instead of an async runtime

- **Status**: accepted
- **Date**: 2026-09-11
- **Deviates from**: `docs/SCTXX-SPEC.md` §4.3 and §9 (decision D-4)

## Context

The spec proposed `reqwest` with `rustls` plus `tokio` for the `api` feature, and described the
architecture as "sync core, async edges": parsing and the deterministic stages synchronous,
`tokio` and `reqwest` confined to `src/llm/`.

When the pipeline was built, that boundary turned out not to hold. The fold is **sequential by
definition** — chunk *k+1* is prompted with the state produced by chunk *k* — so the LLM calls
sit in the middle of a synchronous loop, not at an edge. Making `Backend::complete` async makes
`fold::run` async, which makes `pipeline::extract` async, which makes every CLI subcommand
async. The "edge" is the whole call graph.

The only stage with real concurrency is the premap pass, which is embarrassingly parallel: one
independent call per chunk, results collected in order.

## Decision

Backends are blocking. `src/llm/api.rs` uses `ureq` (blocking, `rustls`), and the premap pass
runs its calls on scoped OS threads (`std::thread::scope`) bounded by `--concurrency`.

## Alternatives considered

- **`reqwest` + `tokio` as specified.** Rejected: it colors the entire call graph async for one
  genuinely concurrent stage, adds ~50 transitive dependencies to a CLI whose binary-size target
  is under 15 MB, and buys nothing — there is no I/O to interleave while waiting for a fold
  response, because the next call depends on that response.
- **`reqwest::blocking`.** Rejected: it still pulls in `tokio` and runs a runtime internally.
- **Subprocess `curl`.** Rejected: an undeclared runtime dependency, worse error handling, and
  no TLS guarantees.

## Consequences

- No `tokio` in the dependency tree. The `api` feature costs `ureq` and `rustls`.
- Concurrency is explicit and bounded: N threads for N premap calls, awaited before the fold
  begins. No detached tasks, satisfying the ownership rules in the methodology §7.1.
- Ctrl-C during a blocking call terminates the process; a fold checkpoint/resume design (roadmap
  M3, `specs/012-m3-cache-and-resume/`) must not assume a cancellation signal reaches the call.
- If a future stage needs genuine I/O concurrency across many slow calls — the probe loop in M5
  is the candidate — revisit this. A thread pool is likely still sufficient; probes are
  independent and few.
- `LlmBackend` is a plain (non-`async_trait`) trait, which keeps it object-safe and simple to
  implement for hosts embedding the library.
