# ADR 0003 — The TUI stack: libraries instead of a port, and MSRV 1.85 → 1.88

- **Status**: accepted
- **Date**: 2026-09-11
- **Affects**: `specs/024-m8-interactive-tui/` (FR-026 to FR-028), `Cargo.toml` (`rust-version`,
  feature `tui`), `.github/workflows/ci.yml` (MSRV job), `CHANGELOG.md`
- **Resolves**: [`12-croft-reuse-and-mit-attribution`](../../specs/000-wayfinding/issues/12-croft-reuse-and-mit-attribution.md)

## Context

`sctxx --tui` needs a file tree, a search pane, a terminal, and a pane that renders the artifact so a
developer can read what was extracted before they hand it on. All four exist in
[croft](https://github.com/vitali87/croft) (MIT), which the maintainer had checked out with the intent
of taking those panes and stripping its branding.

A survey of croft (`v0.1.941`; 178 files, 281,489 lines, ~161k non-test) found:

- The four widgets are individually portable — **none has a `crate::app` back-reference** — but the
  chrome they carry is not: `theme.rs` alone is 1,181 lines and every pane requires a `Theme`, plus
  icons, gradient, scrollbar, hover, prefs, workspace, and output. **~28–30k lines for bare panes**,
  and **55–70k (35–45% of production code) at full fidelity** once LSP, viewers, vim mode, shell
  integration, OSC rewinding, and problem matchers come along.
- The pane the block most needs to be *small* is the canvas, and croft's is the largest:
  `widgets/editor.rs` is 24,751 lines — a full editable editor with tabs, vim mode, LSP semantic
  tokens, and ten embedded viewers. `sctxx` wants a read-only markdown view of its own artifact.
- Croft pins toolchain **1.97.1** with no `rust-version` field, against `sctxx`'s **MSRV 1.85**.
- Its licence is MIT, `Copyright (c) 2026 Vitali Avagyan`, with **no per-file headers and no NOTICE
  file** — 0 of 178 files carry a copyright line.

The maintainer's instruction was to take what is useful, strip Croft branding, and "attribute later,
once we understand which parts we keep". The first half is right; the second is not compatible with
the licence (see Decision 3).

## Decision

### 1. Adopt the ecosystem, not the codebase

Write sctxx-native panes on the same proven stack croft uses, choosing lighter crates where croft's
choice was driven by features we do not need. Costs are crates.io normal-dependency counts.

| Need | Crate | Deps | Licence | Why this and not croft's |
|---|---|---|---|---|
| TUI core | `ratatui` 0.30 + `crossterm` 0.29 | 11 + 15 | MIT | The ecosystem standard, and what croft and agentman both use. Nothing to gain by differing. |
| File tree | `tui-tree-widget` 0.24 | 3 | MIT | Croft hand-rolled 3,434 lines (`widgets/file_tree.rs`) for what a 3-dependency widget provides. |
| Walking / ignore rules | `ignore` 0.4 | 8 | MIT OR Unlicense | Respects `.gitignore`; the walker ripgrep uses. |
| Fuzzy find | `fuzzy-matcher` 0.3 | 1 | MIT | `SkimMatcherV2`, the same engine agentman uses, at one dependency. `nucleo-matcher` is MPL-2.0 — avoided for licence uniformity. `skim` pulls 40. |
| Find-in-files | `ignore` + **the existing** `regex` + `memchr` | 0 | — | Croft uses `grep-searcher`/`grep-regex`; `sctxx` already depends on `regex` and `memchr`, and a v1 find-in-files is a walker plus a line scan. No new dependency. |
| PTY | `portable-pty` 0.9 | 15 | MIT | Same crate croft uses; there is no lighter serious option and this is the part genuinely worth not rewriting. |
| VT parsing | `vt100` 0.16 | 3 | MIT | **This replaces `alacritty_terminal` (17 deps, Apache-2.0).** Croft needs a full terminal emulator because it *is* a terminal app; sctxx needs to run one agent and let it draw. |
| Terminal widget | `tui-term` 0.3 | 4 | MIT | Renders a `vt100` screen into a ratatui area; ~50 lines of glue instead of 5,108. |
| Markdown view | `tui-markdown` 0.3 | 9 | MIT OR Apache-2.0 | The artifact viewer. Croft's editor needed 13,937 lines and tree-sitter; the artifact is markdown. |
| Text input | `tui-input` 0.15 | 6 | MIT | Filter fields and the search box. The only `tui-*` crate croft itself depends on. |

**Consequence: no croft code is copied for the first slice.** The panes are written against these
crates, and croft is a *reference* — its PTY spawn (`widgets/terminal.rs:1758-1775`), its reader-thread
and `Drop` discipline (child killed, reader joined), its `ignore::WalkBuilder::build_parallel` +
debounce + `AtomicBool` cancellation pattern for search, and its `SearchHit { path, line_no, line_text }`
model are all worth reading and none of them needs copying to be used.

**If a specific function is later judged worth copying**, it is ported then, with the header, the
manifest row, and the notice described in Decision 3 — applied to that file, not deferred.

### 2. Pay the MSRV once, on purpose: 1.85 → 1.88

`ratatui` 0.30.1+ requires Rust **1.88**; `ignore` 0.4.31+ and `tui-markdown` likewise. Staying at 1.85
would mean `ratatui` 0.30.0-beta.0, `tui-tree-widget` 0.23, `tui-term` 0.2, and `ignore` 0.4.30 —
pinning the whole UI to a beta or to year-old versions of every widget, and paying the bump later
anyway with more code depending on the pinned APIs.

`sctxx` is pre-1.0 and its MSRV is a documented contract, not a promise to a downstream consumer with
a fixed toolchain. So: **`rust-version = "1.88"`**, the CI MSRV job changes with it, and the CHANGELOG
records it as a breaking-ish change in the release that introduces the TUI.

`rust-version` is per-package, so a `tui` feature cannot have its own. A separate `sctxx-tui` crate
could have kept `sctxx` at 1.85, but the requirement is the `sctxx --tui` flag on the one binary, and
splitting the crate to preserve an MSRV two releases old is the wrong trade.

### 3. The attribution rule, unchanged in principle, now cheap in practice

MIT grants no trademark rights, so stripping "croft" as a name is correct and required by `AGENTS.md`
rule 1. MIT *does* require that its copyright and permission notice be retained in copies and
substantial portions of the software — so attribution cannot be deferred once code is copied.

Because Decision 1 copies nothing for the first slice, **there is no MIT-derived file and therefore no
notice to carry yet**. The machinery is defined now so that the first copied line is compliant on
arrival rather than retrofitted:

- `src/vendor/croft/README.md` — the manifest, one row per ported file: upstream path, upstream
  version, and what changed.
- A header on each ported file: the MIT notice, the upstream path, the version, the modification.
- `LICENSE-MIT` at the repository root, and an entry in `NOTICE` naming Croft for the MIT-derived
  portions while the crate as a whole stays Apache-2.0.
- `scripts/check-vendor-headers.sh` extended to cover `src/vendor/croft/`.

## Alternatives considered

- **Port croft's four panes.** Rejected: ~28–30k lines of chrome to get three panes we can write
  small, and ~14k lines of *editable editor* for a pane that renders our own markdown. The
  maintainer's own words: overkill.
- **Adopt croft's stack verbatim, including `alacritty_terminal` and tree-sitter.** Rejected: 17 more
  dependencies and an Apache-2.0 notice for a terminal emulator's feature set, and 20 grammar crates
  for syntax highlighting that the artifact viewer does not need.
- **Wait for `ratatui` to lower its MSRV again.** Rejected: no such commitment, and 1.88 is a normal
  floor for the current ecosystem.
- **A separate `sctxx-tui` crate at MSRV 1.88.** Rejected: the requirement is one binary with a
  `--tui` flag.
- **Build on agentman.** Rejected here, not on merit: it is a TUI over the same domain, but the
  relationship needs its own decision — ticket
  [`13-sctxx-and-agentman-relationship`](../../specs/000-wayfinding/issues/13-sctxx-and-agentman-relationship.md).

## Consequences

- The TUI lands behind a `tui` feature in the existing crate. `--no-default-features` (the `minimal`
  build) stays free of ratatui and friends, so the network-free deterministic build does not grow a UI.
- **Binary size**: ratatui, crossterm, vt100, portable-pty, tui-markdown and friends are expected to
  add roughly 1.5–2.5 MB to a stripped release binary, against §16's <15 MB target. This must be
  measured when the feature lands, not assumed; the feature is the escape hatch if it does not fit.
- **MSRV 1.88** becomes a prerequisite for every future contributor, recorded in `Cargo.toml`, the CI
  job, and the README.
- A second provenance class is *defined* but unused. If it never gets used, nothing was paid for it.
- `ignore` at 1.88 also becomes available for session discovery later — it is the right walker for
  scanning agent stores, and today's `walkdir` is the weaker tool.
- Revisit if `tui-markdown` proves too heavy or too opinionated for the artifact renderer: the fallback
  is rendering our own known markdown subset with ratatui primitives, which is a bounded job because
  the artifact's shape is ours.
