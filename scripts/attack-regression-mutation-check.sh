#!/usr/bin/env bash
# attack-regression-mutation-check.sh — prove the #511 corpus has teeth.
#
# A regression corpus is only worth its CI time if it actually fails when a
# historical fix is reverted. This script mechanically reverts three fixes, one
# at a time, and requires the corpus to fail for each. It is the mechanical
# counterpart to `attack-regression-gate.sh`:
#
#   attack-regression-gate.sh            -> the corpus passes on real code
#   attack-regression-mutation-check.sh  -> the corpus fails on reverted code
#
# Each mutation is a single, surgical edit (an exact-string replacement) that
# undoes one security fix. The working tree is restored on exit, including on
# failure or interrupt, so the script is safe to run on a dirty checkout.
#
# Exit codes:
#   0  every mutation was caught by the corpus (the corpus has teeth)
#   1  at least one mutation survived, or the corpus errored unexpectedly

set -euo pipefail

cd "$(dirname "$0")/.."
FILTER="attack_regression_tests"
RESTORE_LIST=()

cleanup() {
  local f
  for f in "${RESTORE_LIST[@]:-}"; do
    [[ -n "${f}" ]] && git checkout -- "${f}" 2>/dev/null || true
  done
}
trap cleanup EXIT

# snapshot <path> — remember a file so cleanup can restore it.
snapshot() { RESTORE_LIST+=("$1"); cp "$1" "$1.mutbak"; }

# revert <path> <exact-old> <exact-new> <description>
# Replaces an exact multi-line snippet. Fails loudly if the snippet is not
# found, so a drifting codebase cannot make a mutation silently no-op.
revert() {
  local file="$1" old="$2" new="$3" desc="$4"
  snapshot "${file}"
  if ! grep -qF -- "${old}" "${file}"; then
    echo "::error::mutation '${desc}': anchor not found in ${file}"
    echo "The source has drifted; update this script's anchor."
    exit 1
  fi
  python3 - "${file}" "${old}" "${new}" <<'PY'
import sys
path, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(path).read()
assert s.count(old) == 1, f"anchor not unique in {path}: {s.count(old)} matches"
open(path, "w").write(s.replace(old, new))
PY
  echo "  reverted: ${desc}"
}

# The corpus test filter that each mutation must break.
run_corpus() {
  cargo test -p price-oracle --lib "${FILTER}" 2>&1 | tail -40
}

# expect_caught <name> — the corpus must FAIL right now.
expect_caught() {
  local name="$1" out
  out="$(run_corpus || true)"
  if grep -qE 'test result: FAILED|^error' <<<"${out}"; then
    echo "  PASS: corpus caught the reverted fix '${name}'"
    return 0
  fi
  echo "  FAIL: corpus did NOT catch the reverted fix '${name}'"
  echo "${out}" | sed 's/^/    | /'
  return 1
}

failures=0

echo "== #511 mutation check: each historical fix must be caught when reverted =="

# ── Mutation 1: reentrancy guard (#reentrancy) ──────────────────────────────
# Undo the guard: `enter` no longer refuses a nested entry.
echo "[1/3] reentrancy guard"
revert contracts/price-oracle/src/reentrancy.rs \
  '    if env
        .storage()
        .temporary()
        .get::<_, bool>(&DataKey::ReentrancyGuard)
        .unwrap_or(false)
    {
        panic_with_error!(env, ErrorCode::Reentrant);
    }' \
  '    if false {
        panic_with_error!(env, ErrorCode::Reentrant);
    }' \
  "reentrancy::enter no longer rejects a nested entry"
expect_caught "reentrancy guard" || failures=$((failures + 1))
cleanup; RESTORE_LIST=()

# ── Mutation 2: VWAP non-positive volume guard (#manipulation) ─────────────
# Undo the guard: a source reporting volume <= 0 now influences the VWAP.
echo "[2/3] VWAP non-positive volume guard"
revert contracts/price-oracle/src/storage.rs \
  '        let volume = volumes.get_unchecked(i);
        if volume <= 0 {
            continue;
        }' \
  '        let volume = volumes.get_unchecked(i);' \
  "compute_vwap no longer skips non-positive volume"
expect_caught "VWAP non-positive volume" || failures=$((failures + 1))
cleanup; RESTORE_LIST=()

# ── Mutation 3: fail-closed quorum (#fail-open) ────────────────────────────
# Undo the fix: an evicted quorum config silently falls back to 1 source, which
# is the exact fail-open the corpus pins.
echo "[3/3] fail-closed quorum"
revert contracts/price-oracle/src/admin.rs \
  '        // Fail closed: `initialize` always writes this key, so a missing entry means it was
        // evicted. Falling back to a permissive default would let a single source set prices.
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ConfigMissing))' \
  '        // REVERTED: fail-open fallback to a single source.
        .unwrap_or(1)' \
  "get_min_sources_required falls back to 1 instead of failing closed"
expect_caught "fail-closed quorum" || failures=$((failures + 1))
cleanup; RESTORE_LIST=()

echo
if [[ ${failures} -ne 0 ]]; then
  echo "::error::${failures} mutation(s) survived; the corpus does not pin every fix"
  exit 1
fi
echo "All 3 reverted fixes were caught by the corpus."
