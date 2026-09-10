#!/usr/bin/env bash
# Every file under src/vendor/ must carry the Apache-2.0 attribution header for
# the code it was ported from, and must name the pinned upstream commit.
# Required by AGENTS.md hard rule 1 and .specify/memory/constitution.md IV.
set -euo pipefail

cd "$(dirname "$0")/.."

PINNED_COMMIT="818f1cca8ccf8899f0f4d59336baebaccf358eed"
status=0

check() {
  local file="$1" pattern="$2" description="$3"
  if ! head -n 12 "$file" | grep -qF "$pattern"; then
    echo "FAIL $file: missing $description" >&2
    status=1
  fi
}

shopt -s nullglob
for file in src/vendor/codex/*.rs; do
  case "$(basename "$file")" in
    mod.rs) continue ;;  # the module root ports no upstream file
  esac
  check "$file" "Portions derived from OpenAI Codex" "the derivation notice"
  check "$file" "$PINNED_COMMIT" "the pinned upstream commit"
  check "$file" "Copyright 2025 OpenAI" "the upstream copyright line"
  check "$file" "Apache License, Version 2.0" "the license reference"
  check "$file" "Modified by the sctxx authors" "the modification notice"

  name="$(basename "$file")"
  if ! grep -qF "\`$name\`" src/vendor/codex/README.md; then
    echo "FAIL $file: no row in src/vendor/codex/README.md" >&2
    status=1
  fi
done

for file in prompts/*.md; do
  if grep -qF "derived from OpenAI Codex" "$file"; then
    check "$file" "$PINNED_COMMIT" "the pinned upstream commit"
    check "$file" "Apache-2.0" "the license reference"
  fi
done
shopt -u nullglob

if [ "$status" -eq 0 ]; then
  echo "vendor headers OK"
fi
exit "$status"
