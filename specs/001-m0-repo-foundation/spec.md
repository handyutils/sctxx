# Feature Specification: M0 - Repository foundation

**Feature Branch**: `001-m0-repo-foundation`
**Created**: 2026-09-10
**Status**: Stub
**Input**: docs/SCTXX-ROADMAP.md M0; docs/SCTXX-SPEC.md §2.2–2.3, §4.1–4.3, §14.4, §15.1

## Scope seed

- Single `sctxx` crate (lib + bin), `xtask/`, pinned toolchain, MSRV 1.85, edition 2024, features `zstd`/`api`/`cli-backends`/`eval`/`minimal`.
- `LICENSE` (Apache-2.0), `NOTICE`, `src/vendor/codex/README.md`, first vendored file (UTF-8-safe truncation) with header, `scripts/check-vendor-headers.sh`.
- CI matrix: macOS arm64, Linux x86_64/aarch64 musl, Windows x86_64 — fmt, clippy, tests, `--no-default-features`, MSRV, `cargo package --list`.
- Spec Kit initialized with the same options as ACRYL, constitution ratified, package names reserved as resolved by the name ticket.

## Unlocked by

- [Check the sctxx name and reserve package names](../000-wayfinding/issues/08-name-and-trademark-check.md)

## Next step

Not approved scope. When unlocked, set `.specify/feature.json` to `specs/001-m0-repo-foundation` and run
`/speckit-specify` to replace this stub with a full specification.
