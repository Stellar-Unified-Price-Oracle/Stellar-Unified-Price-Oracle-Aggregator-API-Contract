#!/usr/bin/env bash
# fuzz-corpus-gate.sh — #514 corpus size check + crash gate.
#
# A committed fuzz corpus is only worth something if it is (a) small enough to
# review, and (b) actually replayed on every PR so a regression is caught before
# merge rather than during the next long run. This script does both:
#
#   1. Size-check the committed corpus: entry count and per-entry size must stay
#      within budget, so the corpus cannot silently grow into unreviewable bloat.
#   2. Replay every committed corpus entry through its target. Any crash fails
#      the gate.
#
# The long coverage-guided runs live in `.github/workflows/fuzz.yml`; this is the
# fast, deterministic, per-PR half of the gate.
#
# Environment:
#   CORPUS_MAX_ENTRIES  max entries per target (default 64)
#   CORPUS_MAX_BYTES    max bytes per corpus entry (default 4096)
#   FUZZ_RUNS           replay iterations per target (default 2000)
#   FUZZ_SKIP_REPLAY    set to 1 to only run the size checks (no toolchain needed)
#
# Exit codes:
#   0  corpus is within budget and replays cleanly
#   1  a corpus entry crashed, or the corpus exceeds its size budget
#   3  a required corpus directory is missing

set -euo pipefail

MAX_ENTRIES="${CORPUS_MAX_ENTRIES:-64}"
MAX_BYTES="${CORPUS_MAX_BYTES:-4096}"
RUNS="${FUZZ_RUNS:-2000}"

cd "$(dirname "$0")/.."
status=0

# Every target that must have a committed, minimized corpus.
TARGETS=(fuzz_aggregation fuzz_quickselect fuzz_aggregation_invariants fuzz_storage_layer)

echo "== #514 corpus gate =="

# ── 1. Size checks ─────────────────────────────────────────────────────────
for t in "${TARGETS[@]}"; do
  dir="fuzz/corpus/${t}"
  if [[ ! -d "${dir}" ]]; then
    echo "::error::corpus directory ${dir} is missing"
    status=3
    continue
  fi

  count=$(find "${dir}" -type f | wc -l | tr -d ' ')
  if [[ "${count}" -eq 0 ]]; then
    echo "::error::corpus ${dir} is empty; a target with no corpus replays nothing"
    status=3
    continue
  fi
  if [[ "${count}" -gt "${MAX_ENTRIES}" ]]; then
    echo "::error::corpus ${t} has ${count} entries, over the ${MAX_ENTRIES} budget"
    status=1
  fi

  oversized=$(find "${dir}" -type f -size +${MAX_BYTES}c | wc -l | tr -d ' ')
  if [[ "${oversized}" -gt 0 ]]; then
    echo "::error::corpus ${t} has ${oversized} entries over ${MAX_BYTES} bytes; minimize it"
    status=1
  fi

  echo "  ${t}: ${count} entries (budget ${MAX_ENTRIES}), all under ${MAX_BYTES} bytes"
done

if [[ "${FUZZ_SKIP_REPLAY:-0}" == "1" ]]; then
  echo "FUZZ_SKIP_REPLAY=1: size checks only, replay skipped."
  exit "${status}"
fi

# ── 2. Replay the committed corpus ─────────────────────────────────────────
# Replaying the corpus (rather than fresh random input) is what makes a crash
# reproducible: the same bytes that found it are in version control.
if ! command -v cargo-fuzz >/dev/null 2>&1 && ! cargo fuzz --help >/dev/null 2>&1; then
  echo "::error::cargo-fuzz is not installed; cannot replay the corpus"
  echo "Install with: cargo install --locked cargo-fuzz"
  exit 3
fi

for t in "${TARGETS[@]}"; do
  dir="fuzz/corpus/${t}"
  [[ -d "${dir}" ]] || continue

  echo "-- replaying ${t} (${RUNS} iterations from the committed corpus)"
  if cargo fuzz run "${t}" "${dir}" -- -runs="${RUNS}" -timeout=30 -error_exitcode=1 \
       > /tmp/fuzz-replay-${t}.log 2>&1; then
    echo "   ${t}: clean"
  else
    echo "::error::${t} crashed while replaying the committed corpus"
    tail -25 /tmp/fuzz-replay-${t}.log | sed 's/^/   | /'
    status=1
  fi
done

if [[ "${status}" -ne 0 ]]; then
  exit "${status}"
fi
echo "Corpus gate passed: all corpora within budget and replaying cleanly."
