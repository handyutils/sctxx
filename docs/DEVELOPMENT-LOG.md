# sctxx Development Log

Important product, architecture, workflow, and operational evolutions, newest first. Each entry is added in
a separate documentation commit after the implementation commit and cites full canonical commit SHAs.
Update logged hashes after any history rewrite or squash.

<!-- Entry format:
## YYYY-MM-DD - <short title>

Commits: `<full sha>`, `<full sha>`

<What changed, why, and what later work must know. Link the ledger block: specs/NNN-slug/.>
-->

## 2026-09-11 - v0.1.0 published: crates.io, GitHub Release, and the documentation site

Commits: `0b81048d81808574c86437e4290f89c629cde40e`, `f9591fdf9a9193134bef924d6e4870cadbd9f97b`,
`8105fb71889833ab96e0b57e92ad1618e7845656`, `1fc1848ff2c48a4322b91de9da8c400fcb7d0093`,
`3a458a5159fe517fccffee5ca7d8d7c2acb8a265` · Tag: `v0.1.0` · Release run: `34554821224`

The first public release. Everything below was found by *shipping* — none of it showed up in the
local gate, which is the point worth remembering.

**Published.** `cargo install sctxx` installs 0.1.0 from crates.io (verified from a clean root, and
the installed binary extracts a fixture correctly). Six target archives — macOS x86_64/aarch64,
Linux musl x86_64/aarch64, Windows x86_64/aarch64 — with `.sha256` sidecars are on the GitHub
Release. The documentation site deploys to <https://handyutils.github.io/sctxx> on every push that
touches `website/`. `CARGO_REGISTRY_TOKEN` is now a repository secret, so a tag publishes without a
local token.

**Three bugs only CI could see, all fixed:**

1. **The crate shipped the website's `node_modules`.** Cargo matches `include` the way gitignore
   does, so the bare `README.md`, `LICENSE`, `NOTICE`, and `CHANGELOG.md` entries matched at every
   depth: 187 files instead of 66, 118 of them JS dependencies. Anchoring every entry to the package
   root fixed it, and CI now fails on any file outside the expected set (fix `f9591fd`).
2. **`-D warnings` broke the minimal build.** CI sets `RUSTFLAGS: -D warnings` for every crate, which
   turned two latent warnings on `cargo test --no-default-features` into errors: `src/llm/api.rs`
   compiled its parsing helpers and the `Capabilities` import with the `api` feature off, and
   `tests/adapters.rs` imported `adapters::source` unconditionally while only the `.jsonl.zst` test
   used it. Both are now gated with the feature (fix `8105fb7`).
3. **Windows could not check out the repository.** A zero-byte file named
   `tests/snapshots/pipeline__*.snap` — a literal `*` in a filename — made `git checkout` fail with
   exit 128 before the build started. Deleted (fix `8105fb7`).

**A cross-platform determinism bug that only Windows exposed** (fix `1fc1848`): the pipeline snapshot
expects `Session source hash: 5eb5de4` and got `8ac319b`. `source_hash` is the sha256 of the session
file's *bytes*, and with git's default `core.autocrlf=true` Windows checks fixtures out with CRLF. A
`.gitattributes` pinning `* text=auto eol=lf` fixes it and protects any future binary fixture. This
is the class of bug that cannot be caught on one platform.

**An unsourced claim removed** (fix `3a458a5`): the README and site advertised "212 MB / 94,164 events
→ 61 KB handoff / 7.6 s" with no evidence file, no command, and no corpus behind it. Replaced with a
measured run on the M1 Max against a synthetic 302 MB session — 141,409 events, 288 user turns →
7.9 KB handoff in 5.0 s, zero model calls — recorded in
`specs/004-m1-deterministic-handoff-skeleton/evidence/perf-synthetic-2026-09-11.md`. The evidence also
records the real constraint: ~1.06 GiB peak RSS, about 3.8 bytes per input byte, which leaves little
margin above §16's 400 MB budget for a 100 MB session. **Do not print a performance number without
running it.**

**Process debt carried forward:** TDD's RED step was not observed as a separate run for the compaction
slice (`specs/006-m2-codex-adapter/evidence/T0601-T0603.md` records the gap), and T0604/T0605 remain
open in that block.

## 2026-09-11 - Compaction kind in the IR, and `--since-compact` implemented

Commits: `0b81048d81808574c86437e4290f89c629cde40e`

The research entry below identified two dead wires: `NativeCompaction` could not say whether a provider
boundary was a window re-anchor or a real history reset, and `--since-compact` (spec §3.4) had no flag.
Both are now closed, and the Codex `compacted` path is exercised by a fixture.

- **IR**: `NativeCompaction` gains `windowed: bool` (serde-defaulted, so an older `ir.v1` document still
  deserializes). Codex sets it from `compacted.payload.window_number`; every other adapter defaults to a
  reset, which is correct for Claude Code's `isCompactSummary` and Pi's `compactionSummary`.
- **CLI**: `extract --since-compact` starts at the newest *reset* when the session has one, otherwise at
  the earliest *re-anchor*, keeps the boundary event as the low-trust seed, and leaves `events`
  untouched so `expand` still resolves every `[evt a-b]` pointer. A session that never compacted is a
  stderr notice with exit 0, never a failure.
- **Bug fixed**: a `compacted.replacement_history` message envelope nests its text under `content`, so a
  local compaction summary was previously read as an empty string. Nesting is now walked, bounded to
  eight levels so a corrupt session file cannot exhaust the stack.
- **Snapshot churn**: exactly one added line (`"windowed": false`) in each of the three existing
  compaction snapshots, reviewed individually, plus one new snapshot for the new fixture.
- **Still open in this block**: T0604 (pass provider summaries to the fold as low-trust seeds — a
  versioned-prompt change, deferred to its own slice) and T0605 (`prompts/baseline_codex_compact.md`,
  M5-gated on the eval harness). Both are recorded unchecked in
  `specs/006-m2-codex-adapter/tasks.md`.
- **Process debt recorded**: RED was not captured as a separate failing run for T0601–T0603; the tests
  were written alongside the implementation and the pre-change observable is stated in
  `specs/006-m2-codex-adapter/evidence/T0601-T0603.md`. Do not let this become the habit — the
  methodology's RED step is what proves a test can fail.

Ledger: `specs/006-m2-codex-adapter/` (spec, tasks, evidence).

## 2026-09-11 - Codex compaction algorithm extracted; the reuse boundary drawn

Commits: `7e814ef08d518aad2f645a702afa441d533bf039`

The specs existed but the Codex side of the design was an assessment, not a reading. This change reads
the reference clone's compaction path end to end and records what sctxx actually takes from it.

Findings that later work must honor:

- **`compacted` lines have two meanings.** `window_number == null` (with a `replacement_history`) is a
  legacy history reset; `window_number != null` is a window re-anchor that leaves the full transcript
  intact. sctxx's adapter emits the right event in every case — including the empty-`message` case,
  because `str_field` filters empty strings — but `NativeCompaction` does not record which kind it saw,
  so `--since-compact` is currently undefinable. Ticket
  `specs/000-wayfinding/issues/05-codex-compacted-readability.md` is resolved; the IR change is scoped
  as candidate tasks T0601–T0605 in `specs/006-m2-codex-adapter/research.md`.
- **Codex now ships a summarization-free compaction path.** Token-budget compaction replaces the window
  with canonical context plus retained evidence and writes `message: ""`. That is the upstream
  equivalent of sctxx's `--llm none` artifact, and it needs no new mechanism here.
- **Two dead wires were found in the shipped code.** `Session.native_compactions` is populated and read
  by nothing, so spec §7.2's "low-trust seeds" reach no consumer; and `--since-compact` (spec §3.4) has
  no flag. Both are in the candidate task list (T0604, T0605).
- **An anti-pattern to avoid:** Codex's tiered budgeting returns an *empty* evidence string when the
  render still exceeds the budget. sctxx budgets must fail soft to the deterministic artifact.
- **A provenance problem:** the `codex/` reference clone is a `0.0.0-dev`, ≥0.120-line build with no
  `.git`, so the pinned commit asserted in `AGENTS.md`, spec §2.3/Appendix A, the vendor README, and
  every vendored header cannot be verified. Filed as
  `specs/000-wayfinding/issues/11-codex-vendoring-pin.md`; must be resolved before the M4 publish.

Decision record: `docs/adr/0002-codex-compaction-algorithm-reuse.md`. Evidence:
`specs/006-m2-codex-adapter/research.md`. Spec §19 item 2 is annotated as resolved.
