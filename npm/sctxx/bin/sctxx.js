#!/usr/bin/env node
// The `sctxx` npm package is a shim: it installs the prebuilt binary for the
// user's platform as an optional dependency and hands off to it. All argv,
// stdio, and the exit code belong to the real binary — this file must never
// get in the way of `sctxx ... > file` or of a calling agent branching on an
// exit code (docs/SCTXX-SPEC.md §3.1).
"use strict";

const { spawnSync } = require("node:child_process");
const path = require("node:path");

// Kept in step with npm/platforms.json and the release matrix. A map rather
// than a dependency on the local platforms.json, because the published tarball
// contains only this file.
const PLATFORMS = {
  "darwin arm64": "sctxx-darwin-arm64",
  "darwin x64": "sctxx-darwin-x64",
  "linux arm64": "sctxx-linux-arm64",
  "linux x64": "sctxx-linux-x64",
  "win32 arm64": "sctxx-win32-arm64",
  "win32 x64": "sctxx-win32-x64",
};

const key = `${process.platform} ${process.arch}`;
const pkg = PLATFORMS[key];

if (!pkg) {
  process.stderr.write(
    `sctxx: no prebuilt binary for ${key}.\n` +
      `Install from source instead: cargo install sctxx\n`,
  );
  process.exit(1);
}

let binary;
try {
  // A deep require is deliberate: the platform packages ship no `exports` map,
  // so the path to the binary next to their package.json is reachable.
  const manifest = require.resolve(`${pkg}/package.json`);
  const name = process.platform === "win32" ? "sctxx.exe" : "sctxx";
  binary = path.join(path.dirname(manifest), "bin", name);
} catch {
  process.stderr.write(
    `sctxx: the ${pkg} package is not installed.\n` +
      `Optional dependencies are sometimes skipped; reinstall with:\n` +
      `  npm install -g sctxx --include=optional\n` +
      `Or install from source: cargo install sctxx\n`,
  );
  process.exit(1);
}

const result = spawnSync(binary, process.argv.slice(2), { stdio: "inherit" });

if (result.error) {
  process.stderr.write(`sctxx: could not run ${binary}: ${result.error.message}\n`);
  process.exit(1);
}
// A signal-terminated child reports a null status; that is a failure, not a 0.
process.exit(result.status === null ? 1 : result.status);
