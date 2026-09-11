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

## Deviation from the spec

§14.3 of `docs/SCTXX-SPEC.md` proposed scoped packages (`@sctxx/cli-linux-x64`). npm scopes require
an organisation, and the publishing account has no `sctxx` org, so the packages are unscoped:
`sctxx-<platform>`. The user-visible command — `npm i -g sctxx` — is identical either way. Moving to
the scope later is a rename plus a deprecation notice on the unscoped names.
