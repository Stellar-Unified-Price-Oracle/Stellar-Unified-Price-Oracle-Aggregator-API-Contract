#!/usr/bin/env bash
# check-script-references.sh — fail if a workflow, the Makefile, package.json
# or the docs reference a `scripts/...` path that does not exist (#840).
#
# Usage: ./scripts/check-script-references.sh [repo-root]
set -euo pipefail

root="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)}"
cd "$root"

# Files scanned for references. Missing entries are skipped.
sources=()
while IFS= read -r f; do sources+=("$f"); done < <(
  {
    find .github/workflows -type f \( -name '*.yml' -o -name '*.yaml' \) 2>/dev/null
    [ -f Makefile ] && echo Makefile
    [ -f package.json ] && echo package.json
    find docs -type f \( -name '*.md' -o -name '*.txt' -o -name '*.json' \) 2>/dev/null
  } | sort -u
)

if [ "${#sources[@]}" -eq 0 ]; then
  echo "check-script-references: no sources found" >&2
  exit 1
fi

# Extract `scripts/<path>` tokens, strip trailing punctuation and shell
# metacharacters, and keep only plausible file paths.
refs="$(
  grep -hoE 'scripts/[A-Za-z0-9_./-]+' "${sources[@]}" \
    | sed -E 's/[.,;:)]+$//' \
    | sort -u
)"

missing=0
while IFS= read -r ref; do
  [ -n "$ref" ] || continue
  # Skip globs and template placeholders.
  case "$ref" in
    *'*'*|*'<'*|*'$'*|*'{'*) continue ;;
  esac
  if [ ! -e "$ref" ]; then
    echo "::error file=scripts/check-script-references.sh::dangling script reference: $ref"
    missing=$((missing + 1))
  fi
done <<< "$refs"

if [ "$missing" -ne 0 ]; then
  echo "check-script-references: $missing dangling script reference(s)" >&2
  exit 1
fi

echo "check-script-references: all referenced scripts exist"
