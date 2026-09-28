#!/usr/bin/env bash
# hermetic-integration.sh — one-command hermetic integration harness (#519).
#
# Spins up the full local topology deterministically, runs the cross-contract
# integration suite, and proves the run is repeatable. No network, no testnet,
# no external services: everything runs in-process against the fixtures in
# contracts/price-oracle/src/hermetic_harness.rs.
#
# Usage:
#   ./scripts/hermetic-integration.sh              # run + verify determinism
#   ./scripts/hermetic-integration.sh --once       # single pass, skip the replay
#   ./scripts/hermetic-integration.sh --repeat 3   # verify determinism N times
#
# Environment:
#   ARTIFACT_DIR   where logs and artifacts land (default: hermetic-artifacts)
#
# On failure the harness leaves behind, in $ARTIFACT_DIR:
#   run-N.log        full test output for run N
#   summary.md       pass/fail table and the determinism verdict
#   test_snapshots/  Soroban ledger snapshots, when the SDK wrote any
#
# Exit status is non-zero if any run fails or if two runs disagree, so this is
# usable directly as a CI gate.

set -euo pipefail

cd "$(dirname "$0")/.."

ARTIFACT_DIR="${ARTIFACT_DIR:-hermetic-artifacts}"
REPEAT=2
RUN_ONCE=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --once)   RUN_ONCE=1; shift ;;
    --repeat) REPEAT="${2:-2}"; shift 2 ;;
    -h|--help)
      sed -n '2,20p' "$0" | sed 's/^# \{0,1\}//'
      exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 64 ;;
  esac
done

# The harness owns its whole world, so ordering must not matter either. Running
# single-threaded keeps the output stable enough to diff across runs, which is
# what the determinism check below relies on.
FILTER="hermetic_integration_tests"

log()  { printf '[hermetic] %s\n' "$*"; }
fail() { printf '[hermetic] FAIL: %s\n' "$*" >&2; exit 1; }

# extract_outcomes <run.log> -> the per-test result lines, in a stable order.
#
# This is what the determinism check compares. Only the outcome lines are kept
# (not timings or absolute paths), so a diff means a test genuinely behaved
# differently between runs rather than that the machine was busier.
extract_outcomes() {
  local log_file="$1"
  # With --nocapture libtest prints "test NAME ..." and the verdict on the next
  # line, so both the one-line and two-line forms have to be handled. Timings
  # are stripped so a slower machine does not read as a behaviour change.
  awk '
    /^test [^ ]+ \.\.\./ {
      if ($0 ~ /\.\.\. (ok|FAILED|ignored)/) {
        sub(/ \([0-9]+ ms\)/, ""); print
      } else {
        pending = $0
      }
      next
    }
    pending != "" && /^(ok|FAILED|ignored)/ {
      sub(/ \([0-9]+ ms\)/, "", $0)
      print pending " " $0
      pending = ""
    }
  ' "$log_file" | sort
}

command -v cargo >/dev/null 2>&1 || fail "cargo not found on PATH"

rm -rf "$ARTIFACT_DIR"
mkdir -p "$ARTIFACT_DIR"

log "artifact dir: $ARTIFACT_DIR"

# `cargo test` writes Soroban snapshots next to the sources on failure; clear
# them so a stale snapshot from a previous run cannot make this one look green.
rm -rf test_snapshots

status=0
run_index=0

run_once() {
  run_index=$((run_index + 1))
  local log_file="$ARTIFACT_DIR/run-${run_index}.log"

  log "run ${run_index}/${REPEAT}: cargo test --lib ${FILTER}"
  # Default capture mode (not --nocapture): the SDK writes a "Writing test
  # snapshot ..." line per test, and with capture off that line interleaves into
  # the result line and drags a path into the determinism diff.
  if cargo test -p price-oracle --lib -- --test-threads=1 "$FILTER" \
       > "$log_file" 2>&1; then
    log "run ${run_index}: pass ($(grep -oE '[0-9]+ passed' "$log_file" | head -1))"
    return 0
  fi

  log "run ${run_index}: FAIL — see $log_file"
  grep -E '^(test result|failures:|error)' "$log_file" | head -20 | sed 's/^/[hermetic]   /' || true
  status=1
  return 1
}

# Snapshot any Soroban ledger dumps the SDK produced, so a failure can be
# replayed from the exact ledger state rather than re-derived.
collect_snapshots() {
  if [[ -d test_snapshots ]]; then
    cp -r test_snapshots "$ARTIFACT_DIR/test_snapshots" 2>/dev/null || true
    log "collected ledger snapshots into $ARTIFACT_DIR/test_snapshots"
  fi
}

run_once || true

if (( RUN_ONCE == 0 )); then
  # Determinism: replay the suite and require byte-identical results. Comparing
  # the per-test outcome lines is what makes this a real check — a suite that
  # passes twice by luck would still show a diff here.
  reference="$ARTIFACT_DIR/outcomes-1.txt"
  extract_outcomes "$ARTIFACT_DIR/run-1.log" > "$reference"

  if [[ ! -s "$reference" ]]; then
    log "run 1 produced no per-test outcome lines — cannot verify determinism"
    status=1
  fi

  i=2
  while (( i <= REPEAT )); do
    run_once || true
    extract_outcomes "$ARTIFACT_DIR/run-${i}.log" > "$ARTIFACT_DIR/outcomes-${i}.txt"

    if [[ ! -s "$ARTIFACT_DIR/outcomes-${i}.txt" ]]; then
      log "run ${i} produced no per-test outcome lines — cannot compare"
      status=1
      continue
    fi

    if ! diff -u "$reference" "$ARTIFACT_DIR/outcomes-${i}.txt" > "$ARTIFACT_DIR/determinism-${i}.diff" 2>&1; then
      log "run ${i} DISAGREES with run 1 — the harness is not deterministic"
      head -40 "$ARTIFACT_DIR/determinism-${i}.diff" | sed 's/^/[hermetic]   /'
      status=1
    else
      log "run ${i}: identical to run 1 (deterministic)"
    fi
    i=$((i + 1))
  done
fi

collect_snapshots

# Summary, so a CI failure is legible without opening the logs.
{
  echo "# Hermetic integration harness (#519)"
  echo ""
  echo "- runs: $run_index"
  echo "- verdict: $( ((status == 0)) && echo PASS || echo FAIL )"
  echo ""
  if (( RUN_ONCE == 0 )); then
    echo "## Determinism"
    echo ""
    for i in $(seq 2 "$run_index"); do
      if [[ -s "$ARTIFACT_DIR/determinism-${i}.diff" ]]; then
        echo "- run ${i}: differs from run 1 (see determinism-${i}.diff)"
      else
        echo "- run ${i}: identical to run 1"
      fi
    done
    echo ""
  fi
  echo "## Per-test outcomes (run 1)"
  echo ""
  echo '```'
  cat "$ARTIFACT_DIR/outcomes-1.txt" 2>/dev/null || \
    grep -E '^test ' "$ARTIFACT_DIR/run-1.log" || true
  echo '```'
  echo ""
  echo "Full logs: $ARTIFACT_DIR/run-*.log"
} > "$ARTIFACT_DIR/summary.md"

log "summary: $ARTIFACT_DIR/summary.md"

if (( status != 0 )); then
  fail "hermetic integration harness failed (artifacts in $ARTIFACT_DIR)"
fi

log "PASS — $run_index run(s), deterministic, artifacts in $ARTIFACT_DIR"
