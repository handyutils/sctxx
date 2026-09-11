# Decide the croft reuse boundary, and the licence mechanics for taking any of it

Type: research
Status: resolved (2026-09-11) — decision in [ADR 0003](../../../docs/adr/0003-tui-stack-and-msrv.md)

## Question

`sctxx --tui` (block `specs/024-m8-interactive-tui/`) wants four panes that already exist in
[croft](https://github.com/vitali87/croft), a VS Code-shaped TUI in Rust, in the maintainer's checkout
at `../croft`. Which parts, if any, should be ported — and what must be true of the repository the
moment the first line is copied?

## What is already known

- **Licence: MIT**, `Copyright (c) 2026 Vitali Avagyan` (`LICENSE`, crate `croft-software` v0.1.941,
  `license = "MIT"`, edition 2024, repo <https://github.com/vitali87/croft>).
- **Size: 177 Rust files, ~281,000 LOC**, flat `src/` plus `app/`, and it carries whole subsystems
  `sctxx` has no use for: LSP, DAP, sqlite, docx, iTerm2, ghostty, collaboration, agent lanes.
- **Its stack is exactly the one this block wants**: `ratatui` 0.30, `crossterm` 0.29,
  `portable-pty` 0.9 + `alacritty_terminal` 0.26 (terminal), `ignore` 0.4 (walking), `tree-sitter`
  (highlighting).

## The licensing point that changes the plan

The maintainer's instruction was to strip Croft branding and "attribute later, when we understand
which parts we keep". **Stripping the branding is right; deferring attribution is not.**

- MIT grants no trademark rights, so removing "croft" as a name/branding is correct and required —
  `AGENTS.md` rule 1 already forbids another project's name in ours.
- MIT *does* require: *"The above copyright notice and this permission notice shall be included in all
  copies or substantial portions of the Software."* A ported module is a substantial portion. So the
  notice ships with the **first** copied line, exactly as `src/vendor/codex/` does for Apache-2.0
  code — not in a later cleanup, and not conditionally.
- MIT is compatible with this crate's Apache-2.0: the combined work may stay Apache-2.0 **provided**
  the MIT notice is retained for the MIT-derived portions. The crate must therefore gain a second
  provenance class: `LICENSE-MIT`, a `NOTICE` entry, a vendor-manifest row per file,
  `src/vendor/croft/README.md`, and coverage by `scripts/check-vendor-headers.sh`.

## Decide

1. **Port or re-implement, per pane.** Options: (a) adopt the stack and write sctxx-native panes;
   (b) port a specific, self-contained croft module; (c) port nothing and use croft only as a design
   reference. 281k LOC against a <15 MB binary target argues hard against wholesale vendoring, so the
   realistic answer is (a) with (b) only where a module is genuinely self-contained.
2. **For each candidate module**, whether it can be lifted without dragging Croft's app state, event
   loop, config, and theme with it. Report per module: path, LOC, dependencies pulled in, and the glue
   that would have to be rewritten.
3. **The attribution mechanics**: the exact file layout (`LICENSE-MIT`, `NOTICE`, `src/vendor/croft/`
   headers, manifest) and the check-script change, so the first ported file is compliant on arrival.

## Evidence to produce

A per-pane table: pane · croft module (path, LOC) · reusable as-is / needs glue / reference only ·
what gets dragged along · decision. Plus the proposed attribution file layout, verified by running
`scripts/check-vendor-headers.sh` against a stub.

Unblocks the `plan` of `specs/024-m8-interactive-tui/`. Spec refs: the block's FR-026 to FR-028
(FR-028 is the no-deferral rule above). Related: `AGENTS.md` hard rule 1, constitution IV.

## Answer

**Reference only. No croft code is copied for the first slice, and the panes are written on lighter
crates that do the same job.** Full reasoning and the dependency table are in
[ADR 0003](../../../docs/adr/0003-tui-stack-and-msrv.md); the substance:

1. **The port is not worth its chrome.** All four croft widgets are individually portable — none has a
   `crate::app` back-reference — but `theme.rs` (1,181 lines) is required by every pane, and with
   icons, scrollbar, prefs, workspace, and output the bill is **~28–30k lines for bare panes** and
   **55–70k at full fidelity**. Croft's canvas, the pane `sctxx` most wants to be small, is its
   largest: 24,751 lines of editable editor with vim mode and LSP. `sctxx` needs a read-only markdown
   view of its own artifact.
2. **The stack is kept, the code is not.** `ratatui` + `crossterm` for rendering, `portable-pty` for
   the PTY (the one part genuinely worth not rewriting), `tui-tree-widget` (3 deps) instead of 3,434
   hand-rolled lines, `fuzzy-matcher` (1 dep) for the finder, `tui-term` + **`vt100`** (3 deps) instead
   of `alacritty_terminal` (17 deps, Apache-2.0) — sctxx runs one agent in a pane, it is not a terminal
   emulator — and find-in-files on the `ignore` walker plus the **already-present** `regex` and
   `memchr`, so it costs no new dependency at all.
3. **MSRV moves 1.85 → 1.88**, because `ratatui` 0.30.1+, `ignore` 0.4.31+ and `tui-markdown` require
   it. The alternative was pinning a beta of ratatui and year-old versions of every widget, and paying
   the bump later anyway. `sctxx` is pre-1.0; this is the cheap moment, and it is recorded in
   `Cargo.toml`, the CI MSRV job, and the CHANGELOG.
4. **The MIT machinery is defined and currently unused.** Because nothing is copied, there is no notice
   to carry yet. `LICENSE-MIT`, the `NOTICE` entry, `src/vendor/croft/README.md`, per-file headers, and
   the header check are all specified so the first copied line is compliant on arrival. **Attribution
   is still not deferrable** — the earlier instruction to "attribute later" would not have survived a
   copy; it survives today only because there is nothing to attribute.
5. **What is taken from croft is knowledge**, and it is worth reading rather than copying: the PTY
   spawn and, more importantly, its lifetime discipline (`Drop` kills the child and joins the reader
   thread); the parallel `ignore::WalkBuilder` search with 200 ms debounce and an `AtomicBool` cancel;
   the flat `SearchHit { path, line_no, line_text }` model; and the confirmation that a hand-rolled
   file tree is a trap a 3-dependency crate avoids.

**Residual risk:** if a later slice does copy a croft function, the paperwork is a real cost, and the
temptation to skip it will be highest exactly then. The header check in CI is the mitigation.
