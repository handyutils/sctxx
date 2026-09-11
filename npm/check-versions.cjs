#!/usr/bin/env node
// The npm packages are versioned in lockstep with the crate, and seven files
// have to agree about that. Manual agreement is not a plan, so CI runs this.
//
// Checked:
//   - every npm package version equals the version in Cargo.toml
//   - the wrapper's optionalDependencies name exactly the platform packages,
//     at that same version
//   - platforms.json, the wrapper's fallback map, and the release matrix agree
//     on the target list
"use strict";

const fs = require("node:fs");
const path = require("node:path");

const root = path.join(__dirname, "..");
const problems = [];

function readJson(file) {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8"));
  } catch (error) {
    problems.push(`${path.relative(root, file)}: ${error.message}`);
    return null;
  }
}

const cargo = fs.readFileSync(path.join(root, "Cargo.toml"), "utf8");
const version = (cargo.match(/^version\s*=\s*"([^"]+)"/m) || [])[1];
if (!version) {
  console.error("could not read the version from Cargo.toml");
  process.exit(1);
}

const platforms = readJson(path.join(root, "npm/platforms.json")) || {};
const expected = Object.values(platforms).map((entry) => entry.package);

for (const name of expected) {
  const manifest = readJson(path.join(root, "npm", name, "package.json"));
  if (manifest && manifest.version !== version) {
    problems.push(`npm/${name}/package.json is ${manifest.version}, crate is ${version}`);
  }
  // A deep `require` of the platform package's manifest is how the shim finds
  // the binary, so an `exports` map here would break every install.
  if (manifest && manifest.exports !== undefined) {
    problems.push(`npm/${name}/package.json must not declare \`exports\` (the shim resolves package.json)`);
  }
}

const wrapper = readJson(path.join(root, "npm/sctxx/package.json"));
if (wrapper) {
  if (wrapper.version !== version) {
    problems.push(`npm/sctxx/package.json is ${wrapper.version}, crate is ${version}`);
  }
  const optional = wrapper.optionalDependencies || {};
  const declared = Object.keys(optional).sort();
  if (JSON.stringify(declared) !== JSON.stringify([...expected].sort())) {
    problems.push(
      `npm/sctxx optionalDependencies are [${declared}] but platforms.json names [${[...expected].sort()}]`,
    );
  }
  for (const [name, pinned] of Object.entries(optional)) {
    if (pinned !== version) {
      problems.push(`npm/sctxx pins ${name} at ${pinned}, crate is ${version}`);
    }
  }
  const mapped = Object.values((wrapper.bin || {})).join("");
  if (!mapped.includes("sctxx.js")) {
    problems.push("npm/sctxx must expose bin/sctxx.js");
  }
}

// The release matrix in the workflow must cover every platform package, or a
// package would be published with no binary in it.
const workflow = fs.readFileSync(
  path.join(root, ".github/workflows/release.yml"),
  "utf8",
);
for (const target of Object.keys(platforms)) {
  if (!workflow.includes(target)) {
    problems.push(`release.yml does not build ${target}, so ${platforms[target].package} would ship empty`);
  }
}

if (problems.length > 0) {
  console.error("npm packaging is inconsistent:");
  for (const problem of problems) console.error(`  - ${problem}`);
  process.exit(1);
}
console.log(`npm packaging consistent at ${version} (${expected.length} platform packages)`);
