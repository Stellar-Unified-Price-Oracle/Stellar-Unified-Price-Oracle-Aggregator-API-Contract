#!/usr/bin/env bash
# kani-proofs.sh — run the #512 bounded proofs for the aggregation math.
#
# Property tests sample the input space; these proofs cover it. The harnesses in
# `contracts/price-oracle/src/kani_proofs.rs` are compiled only under `cfg(kani)`
# and discharged by the Kani bounded model checker over the documented bounded
# domain (values in [-4, 4], arrays of length <= 6).
#
# Environment:
#   KANI_BUDGET_SECONDS  wall-clock budget for the proof run (default 1800)
#   KANI_HARNESSES      space-separated harness filter (default: all)
#
# Exit codes:
#   0  all harnesses verified
#   1  a property failed (a counterexample exists in the bounded domain)
#   2  the run exceeded its time budget
#   3  Kani is not installed

set -euo pipefail

BUDGET="${KANI_BUDGET_SECONDS:-1800}"
HARNESSES="${KANI_HARNESSES:-}"
cd "$(dirname "$0")/.."

if ! command -v cargo-kani >/dev/null 2>&1; then
  echo "::error::cargo-kani is not installed."
  echo "Install with: cargo install --locked cargo-kani"
  echo "See docs/security/formal-verification.md for the CI setup."
  exit 3
fi

# The harnesses are cfg(kani)-gated; if the module is not wired in, the run
# would silently verify nothing.
if ! grep -q 'mod kani_proofs;' contracts/price-oracle/src/lib.rs; then
  echo "::error::kani_proofs is not registered in lib.rs; nothing would be proven"
  exit 3
fi

echo "Verifying bounded proofs for the aggregation math (budget ${BUDGET}s)…"
start=$(date +%s)
set +e
# shellcheck disable=SC2086
timeout "${BUDGET}" cargo kani -p price-oracle --tests --harness "${HARNESSES:-}" 2>/dev/null \
  || timeout "${BUDGET}" cargo kani -p price-oracle --harness "${HARNESSES:-}"
status=$?
set -e
elapsed=$(( $(date +%s) - start ))

if [[ ${status} -eq 124 ]]; then
  echo "::error::bounded proofs exceeded the ${BUDGET}s budget"
  exit 2
fi
if [[ ${status} -ne 0 ]]; then
  echo "::error::a bounded proof failed: a counterexample exists in the bounded domain"
  echo "Any counterexample is reported by Kani above as a concrete input."
  exit 1
fi

echo "Bounded proofs passed in ${elapsed}s (budget ${BUDGET}s)."
