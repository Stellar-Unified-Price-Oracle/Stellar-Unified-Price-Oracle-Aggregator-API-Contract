#!/usr/bin/env bash
# attack-regression-gate.sh — run the #511 attack-regression corpus.
#
# The corpus is run as its own target (rather than as part of `make test`) so it
# can be given an explicit wall-clock budget. Budget breaches fail the gate: a
# corpus that cannot run on every PR is a corpus that stops being run.
#
# Environment:
#   ATTACK_BUDGET_SECONDS  wall-clock budget for the corpus (default 300)
#
# Exit codes:
#   0  corpus passed within budget
#   1  a corpus entry failed (a historical fix regressed)
#   2  the corpus exceeded its time budget
#   3  the corpus module is missing or empty

set -euo pipefail

BUDGET="${ATTACK_BUDGET_SECONDS:-300}"
FILTER="attack_regression_tests"

cd "$(dirname "$0")/.."

# The module must exist; an empty corpus compiles and passes, which would make
# this gate silently vacuous.
if ! grep -q "mod ${FILTER};" contracts/price-oracle/src/lib.rs; then
  echo "::error::corpus module '${FILTER}' is not registered in lib.rs"
  exit 3
fi

# Count the pins so an accidentally emptied corpus is caught before running.
# rustfmt may split a macro call across lines, so count the macro invocations
# rather than a fixed `attack!("…"` prefix.
pins=$(grep -c 'attack!(' "contracts/price-oracle/src/${FILTER}.rs" || true)
if [[ "${pins}" -lt 1 ]]; then
  echo "::error::corpus '${FILTER}' contains no attack!() pins"
  exit 3
fi
echo "Running ${pins} attack-regression pins (budget ${BUDGET}s)…"

start=$(date +%s)
set +e
timeout "${BUDGET}" cargo test -p price-oracle --lib "${FILTER}" -- --nocapture
status=$?
set -e
elapsed=$(( $(date +%s) - start ))

if [[ ${status} -eq 124 ]]; then
  echo "::error::attack-regression corpus exceeded its ${BUDGET}s budget"
  exit 2
fi
if [[ ${status} -ne 0 ]]; then
  echo "::error::attack-regression corpus failed (${pins} pins, ${elapsed}s)"
  exit 1
fi

echo "Attack-regression corpus passed: ${pins} pins in ${elapsed}s (budget ${BUDGET}s)."
