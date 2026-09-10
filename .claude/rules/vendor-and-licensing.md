---
paths:
  - "src/vendor/**"
  - "prompts/**"
  - "NOTICE"
  - "LICENSE"
  - "Cargo.toml"
  - "npm/**"
---

# Vendored code and licensing

- Every file in `src/vendor/codex/` starts with this header (fill in the path and change summary):

  ```rust
  // Portions derived from OpenAI Codex (https://github.com/openai/codex),
  // commit 818f1cca8ccf8899f0f4d59336baebaccf358eed, file <original path>.
  // Copyright 2025 OpenAI. Licensed under the Apache License, Version 2.0.
  // Modified by the sctxx authors: <one-line description of changes>.
  ```

- Prompt templates derived from Codex carry the same notice as an HTML comment at the top of the file.
- Adding or removing a vendored file updates `src/vendor/codex/README.md` and spec Appendix A in the same change.
- `NOTICE` keeps the "OpenAI Codex / Copyright 2025 OpenAI" attribution. Don't add the upstream Ratatui
  notice unless TUI code is actually copied.
- `Cargo.toml`: no `codex-*` dependencies, no git/path dependencies in `[dependencies]`; `include` must keep
  `LICENSE`, `NOTICE`, `prompts/**`, `schemas/**`, `skill/**`. npm tarballs ship `LICENSE` and `NOTICE` too.
- Package, crate, binary, and npm names never contain "codex" or "openai".
