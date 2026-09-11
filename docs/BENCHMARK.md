# `sctxx bench` — measuring whether a handoff hands anything off

`sctxx bench <session> --llm <backend>` measures whether a fresh agent, given a handoff artifact,
can answer questions about the session the artifact came from.

It exists because the survey of the field
([`docs/research/2026-09-11-best-proven-oss-algorithm.md`](research/2026-09-11-best-proven-oss-algorithm.md))
found that **no published result measures what sctxx does**. The closest work, *Handoff Debt*
([arXiv:2606.02875](https://arxiv.org/abs/2606.02875), SWE-bench Verified, 181 handoff tasks, 724
runs across three successor agents), compares repo-only, raw trace, summary notes and structured
notes and finds no universal ranking. It has **no arm in which the successor can ask for the part of
the transcript it needs**, which is sctxx's whole hypothesis.

So this measures it. It is built to be able to lose.

## What it measures — and what it does not

**It measures:** whether a fresh agent can answer specific, checkable questions about a session from
a given context.

**It does not measure:** whether a successor resolves an issue. That is what SWE-bench measures, it
needs a container, a real repository and a test command, and this is not that. **A benchmark like
this is a proxy and is named as one everywhere it is reported.** Treat a win here as "the context
carries the facts", not as "the successor ships the fix".

## The arms

| arm | context the successor is given |
| --- | --- |
| `none` | nothing but the question. The floor. |
| `tail` | the near-verbatim recency tail — the obvious cheap answer, and the one the published evidence actually favours |
| `artifact` | the full `sctxx extract --llm none` artifact: brief, items, tail, retrieval pointers |
| `retrieval` | the artifact **plus the ability to ask for an event range and be given the real events** |

`none` is in the list because a win over doing nothing is not a win. `tail` is in the list because
[arXiv:2508.21433](https://arxiv.org/abs/2508.21433) found masking old observations beats LLM
summarisation on cost and matches it on solve rate — so the honest default is a tail, and an artifact
that cannot beat a tail has not earned its size.

`retrieval` is agentic: the successor replies with a line `EXPAND <start>..<end>`, sctxx renders
those events verbatim, and the successor is asked again. Up to `--expansions` rounds. This is the arm
Handoff Debt does not have and the reason the benchmark exists.

## The questions

Derived deterministically from the session's canonical events — **not** from the artifact, so a
question can be asked that the artifact does not answer. Each has a `key`: a string that appears
verbatim in the session, and a correct answer is one that contains it (case-, whitespace- and
punctuation-insensitive). No model decides who is right.

| class | what it is | source |
| --- | --- | --- |
| `brief` | the goal, the busiest file, the newest commit, an unresolved error, the turn count, the working directory | ledgers |
| `deep` | what was asked / reported / called / returned at a specific event in the middle of the session | the IR, at events uniformly spread over the first 85 % |
| `recent` | what was asked and said at the very end | the IR, in the last few events |

The 85 % cutoff is what makes `deep` mean something: those events are outside any recency tail, so
only retrieval reaches them. Without it, "the artifact adds value" would be measured against
questions a tail already answers.

## Reading the output

```
arm                   correct   accuracy       tokens  tok/correct
none                     3/14        21%          812           271
tail                     9/14        64%        18044          2005
artifact                12/14        86%        31002          2584
retrieval               13/14        93%        39450          3035

by question class
arm                       brief      deep    recent
none                         0%       17%       100%
tail                        60%       50%      100%
artifact                   100%       83%      100%
retrieval                  100%      100%      100%
```

`tok/correct` is the column that decides whether an arm is worth running: an arm five points better
at four times the cost is a different claim from one five points better for free.

## Running it

```sh
export SCTXX_BASE_URL=https://api.deepseek.com SCTXX_API_KEY=...
sctxx bench claude:last --llm api:compat/deepseek-v4-flash
sctxx bench claude:last --llm api:compat/deepseek-v4-flash --json > bench.json
sctxx bench a.jsonl b.jsonl --lm api:anthropic --deep 16     # several sessions, aggregated
```

Cost: one model call per (arm × question), plus one per expansion. Defaults are 5 + 8 + 4 = 17
questions over 4 arms = 68 calls. Lower `--brief/--deep/--recent` to spend less.

## What would make the result stronger

Named so that a reader can discount what is missing:

- **One session is an anecdote.** The reference method uses 181 tasks and three successor agents with
  a significance test. `sctxx bench` accepts several sessions and aggregates, which is the beginning
  of that and not the same thing.
- **A question-answering proxy is not a resolve rate.** See above.
- **The questions are derived from things sctxx already computes** (ledgers, the IR). A `brief`
  question is therefore easier for the `artifact` arm than a neutral third party would set it. The
  `deep` class is the one designed to resist that, and it is the one to read first.
- **One backend, one temperature, `0.0`.** The successor's identity is part of the measurement.
- **Keys are matched by substring**, so a lucky paraphrase can score and a correct answer phrased
  around the key can miss. Narrow, and stated.

## Reporting a number

If a claim comes out of this, it comes with the session, the backend, the question counts and the
raw `--json`. The project's rule is the same as everywhere else: a number without its report is not a
result.
