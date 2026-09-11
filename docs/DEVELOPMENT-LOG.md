# sctxx Development Log

Important product, architecture, workflow, and operational evolutions, newest first. Each entry is added in
a separate documentation commit after the implementation commit and cites full canonical commit SHAs.
Update logged hashes after any history rewrite or squash.

<!-- Entry format:
## YYYY-MM-DD - <short title>

Commits: `<full sha>`, `<full sha>`

<What changed, why, and what later work must know. Link the ledger block: specs/NNN-slug/.>
-->

## 2026-09-11 - The TUI starts, and the LLM backends stop writing into your history

Commits: `db8e53f`, `a1b9c65`, `d7363e0`, `97caaed`

**M8 begins: `sctxx --tui` browses sessions.** The first slice of block
`specs/024-m8-interactive-tui/` is the list you can narrow without knowing an id — fuzzy search over
title, first message, id and directory; agent/recency/project filters; a preview read from discovery
alone, so browsing never opens a transcript. MSRV moved 1.85 → 1.88 for the viewport crates
(`ratatui` 0.30.1+, `ignore` 0.4.31+); `rust-version` is per-package, so a feature cannot carry its own.
The TUI is behind a default-on `tui` feature that `--no-default-features` excludes. ADR 0003 decided
the stack — lighter crates, no croft code copied, so the MIT machinery exists and is unused.

**Wayfinder tickets 12, 13 and 14 are closed, which is what gated block 024's plan.** The two that were
still open turned into ADR 0004 and ADR 0005 and are worth knowing:

- **Seeding a fresh agent session (ADR 0004).** Nothing was reusable: the maintainer's launcher has no
  prompt, file, or stdin channel. The design is that the artifact **never travels inline** — only a
  one-line pointer crosses argv, the artifact crosses as a path or over stdin — with a cwd fallback for
  every agent that asks nothing of the agent and so cannot break. Two findings changed the design rather
  than decorating it: Claude Code's `--append-system-prompt-file` is **not in `--help`** (only in the
  `--bare` prose, confirmed by argument parsing), and it **fails lazily and silently** on an unreadable
  path, so the launch must pre-check every path it names. A `--version` probe cannot test flag existence
  — version printing short-circuits before validation.
- **sctxx and agentman (ADR 0005).** `sctxx` keeps the only scanner and owns discovery semantics; no
  code is shared either way yet; the boundary is the versioned `list --json` contract. A shared crate is
  deferred with a trigger (after block 022) rather than rejected, because freezing discovery now would
  version an API known to be incomplete.

**The `cli:` backends were polluting the user's own session history, and that is now fixed.** Every
`cli:claude` completion wrote a real session into `~/.claude/projects`: the backends run in an empty
scratch directory so the agent cannot see the repository, but Claude Code records a session *per working
directory*, so the scratch cwd only named the pollution. `sctxx list` then showed sctxx's own fold
prompts as sessions. Each CLI has a switch for this (`--no-session-persistence`, `--ephemeral`,
`--no-session`); all three templates pass it, and a unit test fails if one loses it — the failure is
silent, so it has to be a test rather than a review. The templates also now record the CLI version each
argv was verified against, which the doc comment had promised since the table was written.

**What later work must know:** block 024's `tasks.md` is authoritative (T2401–T2402 done, T2403 gated
behind nothing and gating every pane); every seeding row in ADR 0004 is *probed, not verified end to
end*, and T2419 is the task that starts real sessions; sessions already written by earlier versions were
deliberately **not** deleted — they are in the user's store.

## 2026-09-11 - npm works; the first real session found two artifact bugs

Commits: `2de85f0`, `c58e79c`, `d5c8a28` · Release re-runs: `34559661480`, `34560054937`

**`npm i -g sctxx` installs the right binary.** A wrapper package declares six per-platform packages
as `optionalDependencies`, each carrying `os`/`cpu`; npm installs exactly one and no Rust toolchain is
involved. Verified by installing from the public registry into a clean prefix and running `sctxx
doctor` through the shim. The packages are unscoped (`sctxx-darwin-arm64`, not `@sctxx/cli-*` as
§14.3 proposed) because npm scopes need an organisation this account does not have; the command users
type is unchanged.

**Two release failures that only the second and third runs could reveal.** The first dispatch died
because the npm job checked out the *tag*, and v0.1.0 predates `npm/` — the binaries already come from
build artifacts, so the job now takes only the recipe from the running ref, and the version from the
tag. The second died on `aarch64-unknown-linux-musl` with `GLIBC_2.28 not found` while CI's identical
cross build passed: a restored `target/` carried build scripts compiled on the runner, which the
container cannot execute. Cross jobs no longer take a cache. Both publish jobs are now idempotent, so
a partially-failed release is safe to re-dispatch — which is how the six npm packages got out.

**One npm name is still missing.** `sctxx-win32-arm64` is refused by npm's spam detection (a new
account publishing a burst of similar names). Six of seven published; Windows on ARM falls back to
the shim's `cargo install` message until a re-dispatch gets it through. Provenance is inconsistent for
0.1.0 as a result — npm versions are immutable, so that is recorded rather than papered over.

**The first real session extraction paid for itself immediately.** A 14,151-line Claude Code session
(6,753 events on the active branch — the rewind resolution doing real work) extracted deterministically
in 7.6 s, and two defects that no synthetic fixture had exposed fell out:

1. **Every artifact's header claimed `masked: 0, artifact: 0`.** `token_counts` hardcoded both, so the
   line a reader uses to judge how much was thrown away said nothing was. Neither is computable where
   it was written: the masked count belongs to the pipeline, and the artifact's own size is the size of
   the text being rendered. Both now come from the caller, and `markdown` measures a first pass so the
   header can state its own size — the two passes differ by one token, far below the estimate's
   precision.
2. **`git check-ignore` printed git's errors at the user.** Added hours earlier with
   `Command::status`, which hands the child the parent's stderr, so writing an artifact outside a
   repository produced `fatal: not a git repository` — from a check whose normal answer is exactly
   that.

Also observed, not yet acted on: `cli:` LLM backends leave a real session in the user's store for
every call (five were visible in `~/.claude/projects`, identifiable by their `sctxx-llm-<pid>-<n>`
temp cwd), because using the user's login requires using the user's config directory. They pollute
`sctxx list`. Excluding them from discovery is the obvious fix and needs its own decision.

## 2026-09-11 - The working tree moves to the normal checkout; artifacts stop being committable

Commits: `c03e4b5328f63aa1c979dc8d2a6c728fa07260d0`, `684382cad54a656a861c60f6154e7918f7e6a18c`

**The repository now lives where it looks like it lives.** All work up to this point happened in a
Delta-managed worktree under `.delta/worktrees/…`, while the project's own directory sat on the
original `Initial commit` with a stale untracked copy of `docs/`, `specs/`, `AGENTS.md`, and
`.specify/`. Two trees, one of them wrong, is a trap: an agent that opens the obvious path edits
yesterday's code. The main checkout was fast-forwarded to `origin/main` after verifying that every
untracked path it held was either an older copy or a byte-identical duplicate (the two files that
looked unique, `docs/clean-room-adapters.md` and `docs/contracts-and-pipeline.md`, already exist as
`.claude/rules/*.md`), and the stale trees were backed up to `/tmp/sctxx-migration-backup` first.
269 tests, the vendor check, `cargo package`, and a fresh `npm ci && vite build` all pass in the new
location. `.delta/` remains on disk (3.5 GB, ignored locally) until it is deliberately deleted.

**The last trace of that layout left the tests.** `tests/pipeline.rs` normalized snapshot paths by
matching the fragments `.delta/worktrees` and `tests/fixtures`, then discarding the line and keeping
its last path segment. That encoded one machine's layout into the corpus and mangled the content it
was supposed to pin — the L3 retrieval line snapshotted as `<path>/basic.jsonl\`` rather than the real
`- \`…/tests/fixtures/claude/basic.jsonl\``. Normalization is now anchored on `CARGO_MANIFEST_DIR`.

**`extract --out` warns when git would track the artifact.** An artifact quotes the session — user
messages verbatim, paths, error output — so writing it into a repository leaves it one `git add -A`
away from being committed and pushed. Spec §19 item 6 asked whether sctxx should edit the user's git
config; the answer is no, but silence is worse. `extract` now asks `git check-ignore` (read-only, on
the §10.1 allowlist) and, when the answer is "not ignored", prints the reason and the exact command
that fixes it — on stderr, so stdout stays the artifact path. This is the first thing a new user will
see, because it is on their first real run.

## 2026-09-11 - The Codex pin verified, and the fold finally receives provider summaries

Commits: `c8678dedb7a9145014ee8cf3c328e417258394ba`, `58c4315c65485cab1225157adb1b8b6afc64c178`

Two of the three items left open at the v0.1.0 release.

**The vendoring pin is real** (ticket 11). Fetching
`818f1cca8ccf8899f0f4d59336baebaccf358eed` directly resolves it the opposite way from how the ticket
was written: the commit exists (dated 2026-09-10, matching the spec), every upstream path in the vendor
manifest is present at it, and the symbols the ports keep are there — including the `sk-…`/`AKIA…`
regexes, `serialize_tiered_input` with its 2 000/10 000 caps, and a nine-line `prompt.md`. **This
corrected a claim made in this repository's own research**: `window_number` on `CompactedItem`,
`compact_token_budget.rs`, and `compact_remote_v2.rs` are all in the pin, so the windowed-compaction
behaviour the ADR depends on is grounded in the pinned source rather than in a newer build. The narrow
real problem was the unversioned `codex/` clone, which `src/vendor/codex/README.md` now labels as a
reading aid and never as provenance.

**The fold now sees what the provider concluded** (T0604). Spec §7.2 says native compactions are seeds
and §7.3 renders them as masked rows; only the row half existed. A chunk a few turns after a compaction
boundary saw nothing of what came before it, and nothing told the model the text was the provider's
lossy leftovers. `fold_user` version 2 adds a `PRIOR PROVIDER SUMMARIES (LOW TRUST)` block — summaries
strictly before the chunk, oldest first, the three most recent, 400 tokens each — with an explicit
"corroborate anything you act on, an item still needs a source range from this chunk" instruction. A
boundary left encrypted or never written is reported as unreadable rather than dropped.

**What is still not proven:** the seed block is justified by construction and unit tests, not by a
measured probe score, and its caps are reasoned defaults. Until M5's eval harness exists, prompt
changes cannot be shown to help rather than distract. That is the next real milestone, and
`specs/000-wayfinding/map.md` now names it as the destination.

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
