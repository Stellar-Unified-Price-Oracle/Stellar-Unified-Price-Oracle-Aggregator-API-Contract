#!/usr/bin/env bash
# Release evidence gate.
#
# Parses a mainnet release checklist (Markdown) and fails when any required
# item is unchecked or lacks an evidence link.
#
# Checklist format:
#   - [ ] Item text — evidence: https://...      (required, unchecked -> fail)
#   - [x] Item text — evidence: https://...      (required, checked -> pass)
#   - [x] Item text — evidence: https://...      (required, no link -> fail)
#   - [~] Item text                              (optional, never gated)
#
# Usage: scripts/check-release-evidence.sh [checklist.md]
set -euo pipefail

CHECKLIST="${1:-docs/mainnet-release-checklist.md}"

if [[ ! -f "$CHECKLIST" ]]; then
  echo "::error::release evidence checklist not found: $CHECKLIST" >&2
  exit 2
fi

# A required item is a task-list entry that is not marked optional with [~].
# Evidence is any Markdown link or bare URL on the same line.
failures=0
required=0
checked=0

lineno=0
while IFS= read -r line || [[ -n "$line" ]]; do
  lineno=$((lineno + 1))

  # Only task-list entries are gated.
  if [[ ! "$line" =~ ^[[:space:]]*[-*][[:space:]]+\[(.)\][[:space:]]+(.*)$ ]]; then
    continue
  fi

  mark="${BASH_REMATCH[1]}"
  body="${BASH_REMATCH[2]}"

  # [~] marks an explicitly optional item.
  if [[ "$mark" == "~" ]]; then
    continue
  fi

  required=$((required + 1))

  if [[ "$mark" != "x" && "$mark" != "X" ]]; then
    echo "::error file=$CHECKLIST,line=$lineno::unchecked required item: $body" >&2
    failures=$((failures + 1))
    continue
  fi

  checked=$((checked + 1))

  if [[ ! "$body" =~ \[[^]]+\]\([^)]+\) && ! "$body" =~ https?://[^[:space:]]+ ]]; then
    echo "::error file=$CHECKLIST,line=$lineno::missing evidence link: $body" >&2
    failures=$((failures + 1))
  fi
done < "$CHECKLIST"

if [[ "$required" -eq 0 ]]; then
  echo "::error file=$CHECKLIST::no required checklist items found" >&2
  exit 1
fi

if [[ "$failures" -gt 0 ]]; then
  echo "release evidence gate FAILED: $failures problem(s) in $CHECKLIST ($checked/$required required items checked)" >&2
  exit 1
fi

echo "release evidence gate passed: $checked/$required required items checked with evidence"
