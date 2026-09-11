# sctxx-linux-arm64

The prebuilt **sctxx** binary for **Linux on arm64** (linux/arm64).

> **Install [`sctxx`](https://www.npmjs.com/package/sctxx), not this package.** This is an internal
> platform package: the `sctxx` wrapper declares it as an `optionalDependency`, so npm installs exactly
> the one matching your machine and you never name it yourself.
>
> ```sh
> npm i -g sctxx
> ```

## What sctxx does

Extract any coding-agent session — Claude Code, Codex CLI, Pi — into a compact, verified,
provenance-linked handoff artifact the next agent loads instead of starting blind. Deterministic Rust,
offline by default, with algorithms ported from
[OpenAI's Codex CLI](https://github.com/openai/codex).

```text
302 MB transcript, 141,409 events  ──►  7.9 KB handoff, 3.2k tokens   (5.0 s, no model)
```

## Links

- **Documentation — everything on one page:** <https://handyutils.github.io/sctxx/>
  - [Quick start](https://handyutils.github.io/sctxx/#quickstart) · [Prompts for your agent](https://handyutils.github.io/sctxx/#prompts) ·
    [Session ids](https://handyutils.github.io/sctxx/#session-ids) · [The artifact](https://handyutils.github.io/sctxx/#artifact) ·
    [Why you can trust it](https://handyutils.github.io/sctxx/#trust) · [Commands](https://handyutils.github.io/sctxx/#commands) ·
    [Worked examples](https://handyutils.github.io/sctxx/#workflows) · [Troubleshooting](https://handyutils.github.io/sctxx/#troubleshooting)
- **Repository (original source):** <https://github.com/handyutils/sctxx>
- **Issues:** <https://github.com/handyutils/sctxx/issues> · **Releases:** <https://github.com/handyutils/sctxx/releases>
- **Changelog:** <https://github.com/handyutils/sctxx/blob/main/CHANGELOG.md>
- **Vendored Codex manifest, file by file:** <https://github.com/handyutils/sctxx/blob/main/src/vendor/codex/README.md>
- **crates.io:** <https://crates.io/crates/sctxx>

## Licence

Apache-2.0. Ships the same `LICENSE` and `NOTICE` as the main package, because the binary inside it
contains code derived from [openai/codex](https://github.com/openai/codex) (Apache-2.0). Not affiliated
with or endorsed by OpenAI.
