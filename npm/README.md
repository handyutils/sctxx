# npm packaging

`npm i -g sctxx` installs the prebuilt binary for the user's platform. This directory holds the
packages that make that work; the binaries themselves are added by the release workflow, never
committed.

```text
npm/
  sctxx/                     the wrapper users install: bin/sctxx.js + optionalDependencies
    bin/sctxx.js             resolves the platform package and execs the real binary
  sctxx-darwin-arm64/        one package per target, with os/cpu fields npm filters on
  sctxx-darwin-x64/
  sctxx-linux-arm64/
  sctxx-linux-x64/
  sctxx-win32-arm64/
  sctxx-win32-x64/
  platforms.json             target → package → binary name
  check-versions.cjs         CI: every version agrees with Cargo.toml
```

## Why a wrapper plus platform packages

npm has no way to fetch a different artifact per platform from one package, and a postinstall
download would break behind corporate mirrors and offline caches. So each platform gets its own
package — with `os` and `cpu` fields, npm installs only the matching one — and the wrapper declares
all six as `optionalDependencies`. Optional means an unsupported platform still installs the wrapper
and gets a clear message instead of a failed install.

The platform packages declare no `exports` map on purpose: the shim locates the binary with
`require.resolve("<package>/package.json")`, which an `exports` map would block. `check-versions.cjs`
fails if one appears.

The Linux packages deliberately omit npm's `libc` field. The binaries are statically linked against
musl, so they run on glibc and musl alike; declaring `libc: "musl"` would wrongly skip the install on
the far more common glibc distribution.

## Versioning

Seven `package.json` files and `Cargo.toml` have to agree. Nothing is typed by hand at release time:
the workflow reads the version from `npm/sctxx/package.json`, stamps it into every package with
`npm pkg set`, and publishes. `node npm/check-versions.cjs` runs in CI on every change and fails on
drift, on a missing platform package, or on a target the release matrix does not build.

## Publishing

`release.yml` → the `npm` job, gated on the same human approval as crates.io (the `npm` environment).
Platform packages publish first, then the wrapper, because the wrapper's `optionalDependencies`
cannot resolve until the versions it names exist. Every publish uses `--provenance`, which signs the
tarballs with the workflow's OIDC identity — npm shows the attesting commit.

The job needs one secret, `NPM_TOKEN`, from an account with publish rights on all seven names.

Both publish jobs are idempotent: each skips a version that already exists. That is what lets a new
channel be added for an already-tagged version, and what makes a partially-failed release safe to
re-dispatch.

### The Windows-on-ARM name, and why publishing is CI-only

`sctxx-win32-arm64` is refused by npm with `403 Package name triggered spam detection`. It was
refused at 0.1.0, again at 0.1.1, and a different name — `sctxx-windows-arm64` — published on the
first attempt. **The name, not the account or the mechanism, is what npm objects to.** The package is
therefore called `sctxx-windows-arm64`; nothing a user types changes.

That failure also explains a subtler problem. The loop publishes platform packages in order and stops
at the first error, so a blocked fifth package silently skipped the sixth *and* the wrapper — leaving
`npm i -g sctxx` serving the previous version while the crate on crates.io was current. A rename fixes
this instance; a release that must publish seven things in order will always be able to fail halfway,
which is why every publish is idempotent and a re-dispatch is the normal recovery.

**Publish from CI, not by hand.** Manual publishes from a workstation are accepted by the CLI
(`+ sctxx@0.1.1`) and then quietly held for npm's review — the version never appears, and `npm view`
still reports the old one. CI publishes carry `--provenance` and land immediately. If a manual publish
is ever unavoidable, verify with `curl https://registry.npmjs.org/<name>/<version>` rather than
trusting the CLI's success line.

Provenance in 0.1.0 is inconsistent for that reason: four platform packages were CI-published and
signed, the wrapper and `sctxx-win32-x64` were not. npm versions are immutable, so it cannot be fixed
retroactively — but nothing from 0.1.2 onward is published by hand.

## Deviation from the spec

§14.3 of `docs/SCTXX-SPEC.md` proposed scoped packages (`@sctxx/cli-linux-x64`). npm scopes require
an organisation, and the publishing account has no `sctxx` org, so the packages are unscoped:
`sctxx-<platform>`. The user-visible command — `npm i -g sctxx` — is identical either way. Moving to
the scope later is a rename plus a deprecation notice on the unscoped names.
