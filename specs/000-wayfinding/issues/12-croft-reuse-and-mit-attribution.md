# Decide the croft reuse boundary, and the licence mechanics for taking any of it

Type: research
Status: open

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
