# Security policy

## Reporting a vulnerability

Report privately through
[GitHub Security Advisories](https://github.com/handyutils/sctxx/security/advisories/new).
Please do not open a public issue for a vulnerability.

Include what you did, what happened, and what you expected. If a session file is needed to
reproduce, run it through `sctxx redact --strict` first and say so.

Expect an acknowledgement within a few days.

## What sctxx does with your data

sctxx reads coding-agent session transcripts. Those contain everything someone typed to an
agent, plus their code, paths, and sometimes their secrets. The design reflects that.

- **Local first.** No telemetry, no accounts, no cloud storage. The only network egress is the
  LLM backend you selected. `--llm none` and a `--no-default-features` build contain no network
  code at all.
- **Redaction before egress.** Every masked row is redacted before it reaches a backend, every
  model response is redacted on the way back, and the rendered artifact is redacted again.
  `--redact off` is honored only with `--llm none`, where nothing leaves the machine.
- **Read-only on your repository.** Reconciliation runs a fixed allowlist of git subcommands
  (`rev-parse`, `branch`, `status`, `log`, `cat-file`) through `std::process::Command`, never
  through a shell.
- **Transcripts are data.** Nothing found in a session is ever executed, and every prompt that
  embeds transcript text fences it and states that it must not be followed.
- **`cli:` backends are sandboxed.** An agent CLI used as a completion engine runs in an empty
  temporary directory with tool use disabled, so it cannot read or modify your repository, and
  it is killed if it outlives its call.
- **Nothing is written outside the paths you name.** sctxx writes only to `--out` and never to
  any agent's session store.

## What redaction can and cannot do

Redaction is best-effort pattern matching over 12 secret classes by default and 15 with
`--redact strict` (`sctxx doctor` lists them). It recognizes bearer tokens, OpenAI, Anthropic,
AWS, GitHub, Slack, Stripe, and Google keys, JWTs, PEM private-key blocks, connection strings
with inline passwords, and secret-shaped assignments.

**It cannot recognize a secret that does not look like one.** A password in prose, a
custom-format internal token, or proprietary source code will pass through. Before sharing an
artifact or contributing a fixture, read it.

## Threat model

sctxx assumes the session file is **untrusted input**. It is written by an agent, and it
contains text from users, tools, web pages, and other agents. Accordingly:

- Adapters never fail a session on one bad line and never panic on malformed input; the library
  denies `unwrap`, `expect`, and `panic!` outside tests.
- Prompt-injection attempts inside a transcript are treated as data, not instructions.
- A model's output is schema-validated and passed through provenance, quote, length, and
  redaction gates before it can change any state.

sctxx does **not** defend against a hostile local user or a compromised machine: it runs with
your privileges and reads files you can already read.

## Supported versions

Before 1.0, only the latest published version receives fixes.
