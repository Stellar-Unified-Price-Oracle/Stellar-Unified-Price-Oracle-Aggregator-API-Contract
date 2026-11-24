#!/usr/bin/env bash
# coverage-gate.sh — per-module coverage floor for the core modules (#826).
#
# Runs `cargo llvm-cov` over the price-oracle lib tests, writes an lcov report
# and an HTML report, then enforces a line-coverage floor for each core module
# (prices, storage, sources, history, admin). CI fails when any module is below
# its floor.
#
# Usage:
#   ./scripts/coverage-gate.sh            # run coverage + enforce floors
#   ./scripts/coverage-gate.sh --check    # enforce floors on an existing lcov
#
# Environment:
#   COVERAGE_DIR        output directory (default target/coverage)
#   COVERAGE_FLOORS     per-module floors, "module=percent" pairs, comma
#                       separated (default: the CORE_FLOORS below)
#   COVERAGE_LCOV       lcov file to read in --check mode
#                       (default $COVERAGE_DIR/lcov.info)
#   COVERAGE_SUMMARY    markdown summary path (default $GITHUB_STEP_SUMMARY)
#
# Reports: $COVERAGE_DIR/lcov.info and $COVERAGE_DIR/html/index.html.

set -euo pipefail

COVERAGE_DIR="${COVERAGE_DIR:-target/coverage}"
COVERAGE_LCOV="${COVERAGE_LCOV:-$COVERAGE_DIR/lcov.info}"
COVERAGE_SUMMARY="${COVERAGE_SUMMARY:-${GITHUB_STEP_SUMMARY:-/dev/null}}"
SRC="contracts/price-oracle/src"

# Core modules and their line-coverage floors (percent).
CORE_FLOORS="${COVERAGE_FLOORS:-prices=80,storage=80,sources=80,history=80,admin=80}"

MODE="run"
if [[ "${1:-}" == "--check" ]]; then
  MODE="check"
fi

if [[ "$MODE" == "run" ]]; then
  mkdir -p "$COVERAGE_DIR"
  cargo llvm-cov -p price-oracle --lib \
    --lcov --output-path "$COVERAGE_LCOV" \
    --html --output-dir "$COVERAGE_DIR/html"
fi

if [[ ! -f "$COVERAGE_LCOV" ]]; then
  echo "coverage-gate: missing lcov report at $COVERAGE_LCOV" >&2
  exit 1
fi

# Aggregate line coverage per core module from the lcov report.
# lcov records: SF:<path>, then LF:<lines found>, LH:<lines hit>.
report="$(awk -v src="$SRC" '
  /^SF:/ {
    file = substr($0, 4)
    mod = ""
    if (index(file, src "/") == 1) {
      rest = substr(file, length(src) + 2)
      mod = rest
      sub(/\.rs$/, "", mod)
      sub(/\/.*$/, "", mod)
    }
    next
  }
  /^LF:/ { lf = substr($0, 4) + 0 }
  /^LH:/ { lh = substr($0, 4) + 0 }
  /^end_of_record/ {
    if (mod != "") { found[mod] += lf; hit[mod] += lh }
    mod = ""; lf = 0; lh = 0
  }
  END {
    for (m in found) {
      pct = found[m] > 0 ? (100.0 * hit[m] / found[m]) : 100.0
      printf "%s %.2f %d %d\n", m, pct, hit[m], found[m]
    }
  }
' "$COVERAGE_LCOV")"

fail=0
{
  echo "## Coverage (#826)"
  echo
  echo "| Module | Lines hit | Lines found | Coverage | Floor | Status |"
  echo "| --- | --- | --- | --- | --- | --- |"
} >> "$COVERAGE_SUMMARY"

IFS=',' read -r -a floors <<< "$CORE_FLOORS"
for entry in "${floors[@]}"; do
  [[ -z "$entry" ]] && continue
  mod="${entry%%=*}"
  floor="${entry##*=}"
  row="$(awk -v m="$mod" '$1 == m { print; exit }' <<< "$report")"
  if [[ -z "$row" ]]; then
    echo "coverage-gate: no coverage data for core module '$mod'" >&2
    printf '| %s | - | - | - | %s%% | MISSING |\n' "$mod" "$floor" >> "$COVERAGE_SUMMARY"
    fail=1
    continue
  fi
  read -r _ pct hit found <<< "$row"
  status="ok"
  if awk -v p="$pct" -v f="$floor" 'BEGIN { exit !(p + 0 < f + 0) }'; then
    status="FAIL"
    fail=1
  fi
  printf '| %s | %s | %s | %s%% | %s%% | %s |\n' \
    "$mod" "$hit" "$found" "$pct" "$floor" "$status" >> "$COVERAGE_SUMMARY"
  echo "coverage-gate: $mod ${pct}% (floor ${floor}%) -> $status"
done

if [[ "$fail" -ne 0 ]]; then
  echo "coverage-gate: core module coverage below floor" >&2
  exit 1
fi

echo "coverage-gate: all core modules meet their coverage floor"
