# sctxx-linux-arm64

The prebuilt **sctxx** binary for **Linux on arm64** (linux/arm64).

> **Install [`sctxx`](https://www.npmjs.com/package/sctxx), not this package.**
> This is an internal platform package: the `sctxx` wrapper declares it as an
> `optionalDependency`, so npm installs exactly the one that matches your machine and you never name
> it yourself.
>
> ```sh
> npm i -g sctxx
> ```

## What sctxx does

Extract any coding-agent session — Claude Code, Codex CLI, Pi — into a compact, verified,
provenance-linked handoff artifact that the next agent loads instead of starting blind. Deterministic
Rust, offline by default, with algorithms ported from
[OpenAI's Codex CLI](https://github.com/openai/codex).

```text
302 MB transcript, 141,409 events  ──►  7.9 KB handoff, 3.2k tokens   (5.0 s, no model)
```

- **Documentation and quick start:** https://handyutils.github.io/sctxx/
- **Prompts to give your agent:** https://handyutils.github.io/sctxx/#prompts
- **Worked examples:** https://handyutils.github.io/sctxx/#workflows
- **Source, issues, changelog:** https://github.com/handyutils/sctxx

## Licence

Apache-2.0. Ships the same `LICENSE` and `NOTICE` as the main package, because the binary inside it
contains code derived from [openai/codex](https://github.com/openai/codex) (Apache-2.0).
