#!/usr/bin/env bash
# Governance policy review cadence check (issue #663).
#
# Validates the governance policy registry (docs/governance/policies.json):
#   - every critical policy declares an owner and a review date
#   - change history and supersession are recorded
#   - overdue reviews are surfaced and cannot be reported as current
#
# Exit codes:
#   0  all critical policies are current
#   1  one or more critical policies are overdue or malformed
#   2  registry missing or tooling unavailable
set -euo pipefail

REGISTRY="${GOVERNANCE_POLICY_REGISTRY:-docs/governance/policies.json}"
TODAY="${GOVERNANCE_TODAY:-$(date -u +%Y-%m-%d)}"

if [[ ! -f "$REGISTRY" ]]; then
  echo "error: governance policy registry not found: $REGISTRY" >&2
  exit 2
fi

if ! command -v jq >/dev/null 2>&1; then
  echo "error: jq is required to validate the governance policy registry" >&2
  exit 2
fi

if ! jq -e . "$REGISTRY" >/dev/null 2>&1; then
  echo "error: governance policy registry is not valid JSON: $REGISTRY" >&2
  exit 2
fi

fail=0

# Structural validation: required fields on every policy entry.
missing=$(jq -r '
  .policies[]
  | select(
      (.id // "") == ""
      or (.title // "") == ""
      or (.owner // "") == ""
      or (.review_date // "") == ""
      or (.critical | type) != "boolean"
      or (.history | type) != "array"
      or (.history | length) == 0
    )
  | .id // "<unknown>"
' "$REGISTRY")

if [[ -n "$missing" ]]; then
  echo "error: policies missing owner/review_date/history:" >&2
  echo "$missing" | sed 's/^/  - /' >&2
  fail=1
fi

# Supersession consistency: superseded_by must reference an existing policy.
bad_supersession=$(jq -r '
  (.policies | map(.id)) as $ids
  | .policies[]
  | select((.superseded_by // null) != null and (.superseded_by as $s | $ids | index($s) | not))
  | .id
' "$REGISTRY")

if [[ -n "$bad_supersession" ]]; then
  echo "error: policies reference unknown superseded_by target:" >&2
  echo "$bad_supersession" | sed 's/^/  - /' >&2
  fail=1
fi

# Overdue detection: critical, non-superseded policies past their review date.
overdue=$(jq -r --arg today "$TODAY" '
  .policies[]
  | select(.critical == true)
  | select((.superseded_by // null) == null)
  | select(.review_date < $today)
  | "\(.id)\t\(.owner)\t\(.review_date)"
' "$REGISTRY")

if [[ -n "$overdue" ]]; then
  echo "error: overdue critical governance policy reviews (today=$TODAY):" >&2
  while IFS=$'\t' read -r id owner review_date; do
    echo "  - $id (owner: $owner, review due: $review_date)" >&2
  done <<< "$overdue"
  fail=1
fi

# A policy flagged current must not be overdue or superseded.
stale_current=$(jq -r --arg today "$TODAY" '
  .policies[]
  | select(.status == "current")
  | select((.superseded_by // null) != null or .review_date < $today)
  | .id
' "$REGISTRY")

if [[ -n "$stale_current" ]]; then
  echo "error: policies reported as current but overdue or superseded:" >&2
  echo "$stale_current" | sed 's/^/  - /' >&2
  fail=1
fi

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi

echo "ok: all critical governance policies are current (today=$TODAY)"
