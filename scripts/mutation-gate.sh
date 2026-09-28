#!/usr/bin/env bash
# mutation-gate.sh — cargo-mutants security gate (#412, per-module #520).
#
# Runs mutation testing in three classes:
#   critical   — auth, aggregation math, bounds, finality, upgrade.
#                Any surviving (missed) or timed-out mutant fails the gate.
#   per-module — each module in scripts/mutation-thresholds.conf, scored and
#                thresholded individually so a weak suite in a security-critical
#                module cannot hide behind an aggregate number.
#   general    — everything else. Fails below GENERAL_THRESHOLD %.
#
# Usage:
#   ./scripts/mutation-gate.sh critical|general|per-module|all|self-test
#
# Environment:
#   GENERAL_THRESHOLD  minimum kill rate for general modules (default 80)
#   MUTANTS_ARGS       extra cargo-mutants args, e.g. "--in-diff pr.diff" or
#                      "--shard 0/4" to bound CI duration
#   THRESHOLDS_CONF    per-module thresholds (default scripts/mutation-thresholds.conf)
#   WAIVERS_FILE       equivalent-mutant waivers (default scripts/mutation-waivers.txt)
#
# `self-test` exercises the scoring and threshold logic against fixtures and
# needs neither cargo-mutants nor a built contract, so the gate's own logic is
# covered by CI even on the fast path (#520).
#
# Reports (score per class + survivor list) go to stdout and, in CI,
# $GITHUB_STEP_SUMMARY. Survivors are left in mutants-<class>/mutants.out/missed.txt.

set -euo pipefail

SRC="contracts/price-oracle/src"
SECURITY_CRITICAL=(admin.rs rbac.rs multisig.rs storage.rs finality.rs migration.rs)
GENERAL_THRESHOLD="${GENERAL_THRESHOLD:-80}"
CLASS="${1:-all}"
SUMMARY="${GITHUB_STEP_SUMMARY:-/dev/null}"
THRESHOLDS_CONF="${THRESHOLDS_CONF:-scripts/mutation-thresholds.conf}"
WAIVERS_FILE="${WAIVERS_FILE:-scripts/mutation-waivers.txt}"
status=0

# ---------------------------------------------------------------------------
# Scoring helpers
#
# Kept as standalone functions so `self-test` can drive them without invoking
# cargo-mutants. That is what makes the gate's own logic testable.
# ---------------------------------------------------------------------------

# read_outcomes <outcomes.json> -> "caught<TAB>missed<TAB>timeout<TAB>unviable"
#
# cargo-mutants has shipped two shapes for this file: a summary object with
# numeric fields, and a per-mutant array. Both are accepted, and an
# unrecognised shape is a hard error rather than a silent zero — a gate that
# cannot read its input must fail, not pass.
read_outcomes() {
  local json="$1"
  if ! jq -e 'type == "object" and (.caught | type) == "number"' "$json" >/dev/null 2>&1; then
    if jq -e 'type == "array"' "$json" >/dev/null 2>&1; then
      jq -r '.[] | .outcome' "$json" | awk '
        { counts[$1]++ }
        END {
          printf "%d\t%d\t%d\t%d\n", (counts["caught"] + 0), (counts["missed"] + 0), \
                (counts["timeout"] + 0), (counts["unviable"] + 0)
        }'
      return
    fi
    echo "unrecognised outcomes.json shape: $json" >&2
    return 1
  fi
  jq -r '[.caught, .missed, .timeout, .unviable] | @tsv' "$json"
}

# kill_rate <caught> <missed> <timeout> -> percentage, one decimal.
#
# Unviable mutants are excluded from the denominator: a mutant the compiler
# rejects is not something a test suite could have caught, so counting it would
# understate the score for reasons unrelated to test quality.
kill_rate() {
  awk -v c="$1" -v m="$2" -v t="$3" \
    'BEGIN { v = c + m + t; printf "%.1f", v ? 100 * c / v : 100 }'
}

# score_passes <score> <threshold> -> exit 0 when the score MEETS the threshold.
#
# Named for the question it answers so call sites read as assertions rather
# than as double negatives.
score_passes() {
  awk -v s="$1" -v t="$2" 'BEGIN { exit !(s + 0 >= t + 0) }'
}

# ---------------------------------------------------------------------------
# Per-module configuration (#520)
# ---------------------------------------------------------------------------

# threshold_for <file> -> the threshold governing that file.
#
# Falls back to the general threshold, so adding a source file to the crate can
# never silently drop it out of the gate.
threshold_for() {
  local file="$1" line module threshold f
  [[ -f "$THRESHOLDS_CONF" ]] || { echo "$GENERAL_THRESHOLD"; return; }
  while IFS= read -r line; do
    [[ "$line" =~ ^[[:space:]]*# ]] && continue
    [[ "$line" =~ ^[[:space:]]*$ ]] && continue
    [[ "$line" == *"="* ]] || continue
    module="${line%%=*}"
    threshold="${line##*=}"
    for f in $module; do
      if [[ "$f" == "$file" ]]; then
        echo "$threshold"
        return
      fi
    done
  done < "$THRESHOLDS_CONF"
  echo "$GENERAL_THRESHOLD"
}

# per_module_files -> one guarded file per line.
per_module_files() {
  [[ -f "$THRESHOLDS_CONF" ]] || return 0
  local line lhs f first
  while IFS= read -r line; do
    [[ "$line" =~ ^[[:space:]]*# ]] && continue
    [[ "$line" =~ ^[[:space:]]*$ ]] && continue
    [[ "$line" == *"="* ]] || continue
    lhs="${line%%=*}"
    # The first token is the module label; the rest are the files it guards.
    first=1
    for f in $lhs; do
      if (( first )); then first=0; continue; fi
      echo "$f"
    done
  done < "$THRESHOLDS_CONF"
}

# waiver_count -> number of real (non-comment) waiver entries.
waiver_count() {
  [[ -f "$WAIVERS_FILE" ]] || { echo 0; return; }
  grep -cvE '^[[:space:]]*(#|$)' "$WAIVERS_FILE" || true
}

# generate_per_module -> one cargo-mutants run per guarded module.
#
# Scoping to a single module per run is what keeps runtime inside budget: a run
# that only touches the changed module is far cheaper than mutating the whole
# tree, and `--in-diff` still applies within it.
generate_per_module() {
  local file out
  while read -r file; do
    [[ -n "$file" ]] || continue
    out="mutants-per-module/${file%.rs}"
    echo "::group::mutating $file"
    # shellcheck disable=SC2086
    cargo mutants --no-shuffle -o "$out" --file "$SRC/$file" ${MUTANTS_ARGS:-} || true
    echo "::endgroup::"
  done < <(per_module_files)
}

# report_per_module -> per-module score table; fails any module under threshold.
report_per_module() {
  echo "### Per-module mutation scores (#520)"
  echo ""
  echo "| module | file | score | threshold | result |"
  echo "|---|---|---|---|---|"

  local file threshold score caught missed timeout unviable json failed=0
  while read -r file; do
    [[ -n "$file" ]] || continue
    threshold="$(threshold_for "$file")"
    json="mutants-per-module/${file%.rs}/mutants.out/outcomes.json"
    if [[ ! -f "$json" ]]; then
      echo "| ${file%.rs} | $file | - | $threshold | no results |"
      echo "::error::no mutation results for $file"
      failed=1
      continue
    fi
    IFS=$'\t' read -r caught missed timeout unviable < <(read_outcomes "$json")
    score="$(kill_rate "$caught" "$missed" "$timeout")"
    if score_passes "$score" "$threshold"; then
      echo "| ${file%.rs} | $file | ${score}% | $threshold | pass |"
    else
      echo "| ${file%.rs} | $file | ${score}% | $threshold | FAIL |"
      echo "::error::$file mutation score ${score}% is below its ${threshold}% threshold"
      failed=1
    fi
  done < <(per_module_files)

  local waivers
  waivers="$(waiver_count)"
  echo ""
  echo "Equivalent-mutant waivers in effect: $waivers"
  if (( waivers > 0 )) && [[ -f "$WAIVERS_FILE" ]]; then
    echo ""
    echo "<details><summary>Waivers ($WAIVERS_FILE)</summary>"
    echo ""
    echo '```'
    grep -vE '^[[:space:]]*(#|$)' "$WAIVERS_FILE" || true
    echo '```'
    echo "</details>"
  fi

  (( failed == 0 )) || status=1
}

# ---------------------------------------------------------------------------
# Self-test
# ---------------------------------------------------------------------------

# Exercises the gate's own logic against fixtures, with no cargo-mutants and no
# built contract. Covers the three things a per-module gate can get wrong:
# scoring arithmetic, threshold comparison, and waiver/config accounting.
self_test() {
  local tmp passed=0 r

  tmp="$(mktemp -d)"
  # shellcheck disable=SC2064
  trap "rm -rf '$tmp'" RETURN

  check() {
    local desc="$1" expected="$2" actual="$3"
    if [[ "$expected" == "$actual" ]]; then
      passed=$((passed + 1))
      echo "  ok  — $desc"
    else
      echo "  FAIL — $desc (expected '$expected', got '$actual')"
      status=1
    fi
  }

  # 1. Kill-rate arithmetic, including the zero-mutant case.
  check "kill rate: all caught"      "100.0" "$(kill_rate 20 0 0)"
  check "kill rate: half caught"     "50.0"  "$(kill_rate 5 5 0)"
  check "kill rate: no mutants"      "100.0" "$(kill_rate 0 0 0)"
  # Unviable mutants must not drag the score down.
  check "kill rate: unviable ignored" "100.0" "$(kill_rate 4 0 0)"

  # 2. Threshold comparison, including the exact-boundary case: a module
  #    sitting exactly on its bar passes.
  score_passes 90 90    && r=pass || r=fail
  check "threshold: exactly at bar passes" "pass" "$r"
  score_passes 90.1 90  && r=pass || r=fail
  check "threshold: above bar passes"      "pass" "$r"
  score_passes 89.9 90  && r=pass || r=fail
  check "threshold: just under bar fails"  "fail" "$r"

  # 3. outcomes.json parsing, both shapes cargo-mutants emits.
  printf '{"caught":7,"missed":2,"timeout":1,"unviable":3}\n' > "$tmp/summary.json"
  check "outcomes: summary object" "7	2	1	3" "$(read_outcomes "$tmp/summary.json")"

  cat > "$tmp/array.json" <<'JSON'
[{"outcome":"caught"},{"outcome":"caught"},{"outcome":"missed"},
 {"outcome":"timeout"},{"outcome":"unviable"}]
JSON
  check "outcomes: per-mutant array" "2	1	1	1" "$(read_outcomes "$tmp/array.json")"

  printf '{"unexpected":true}\n' > "$tmp/bad.json"
  if read_outcomes "$tmp/bad.json" >/dev/null 2>&1; then r=accepted; else r=rejected; fi
  check "outcomes: unknown shape rejected" "rejected" "$r"

  # 4. Threshold lookup: a listed file gets its own bar; an unlisted one falls
  #    back to the general threshold rather than escaping the gate.
  check "threshold: listed file" "95" "$(threshold_for storage.rs)"
  check "threshold: unlisted file falls back" "$GENERAL_THRESHOLD" \
    "$(threshold_for never_listed.rs)"

  # 5. Every guarded module sits strictly above the general bar — the point of
  #    the issue is that guarded modules are held higher than the rest.
  local file below=0
  while read -r file; do
    [[ -n "$file" ]] || continue
    if ! score_passes "$(threshold_for "$file")" "$GENERAL_THRESHOLD"; then
      echo "  FAIL — $file threshold $(threshold_for "$file") is not above general"
      below=1
      status=1
    fi
  done < <(per_module_files)
  check "thresholds: all guarded modules exceed general" "0" "$below"

  # 6. The security-critical modules the issue names are actually guarded.
  local guarded missing=0
  guarded="$(per_module_files)"
  for want in storage.rs rbac.rs prices.rs; do
    if ! grep -qx "$want" <<<"$guarded"; then
      echo "  FAIL — $want is not listed in $THRESHOLDS_CONF"
      missing=1
      status=1
    fi
  done
  check "config: auth/aggregation/storage modules guarded" "0" "$missing"

  # 7. Waivers are counted, and only real entries count.
  printf '# only comments\n\n' > "$tmp/w0.txt"
  check "waivers: comment-only file counts zero" "0" \
    "$(WAIVERS_FILE="$tmp/w0.txt" waiver_count)"
  printf '# c\nstorage|storage.rs:1|a|b|why|dev|2026-01-01\nauth|admin.rs:2|c|d|why|dev|2026-01-02\n' > "$tmp/w2.txt"
  check "waivers: entries counted" "2" \
    "$(WAIVERS_FILE="$tmp/w2.txt" waiver_count)"

  # 8. Waived lines must be pinned to a file:line so a refactor invalidates
  #    them rather than letting a blanket suppression persist.
  printf 'storage|storage.rs:1|a|b|why|dev|2026-01-01\n' > "$tmp/w3.txt"
  local bad_waiver=0
  while IFS= read -r w; do
    [[ -z "$w" ]] && continue
    # module|file:line|...  -> exactly 7 fields, and file:line must be present.
    if [[ "$(awk -F'|' '{print NF}' <<<"$w")" != "7" ]] \
       || ! grep -qE '^[^|]+\|[^|]+:[0-9]+\|' <<<"$w"; then
      echo "  FAIL — malformed waiver: $w"
      bad_waiver=1
      status=1
    fi
  done < <(grep -vE '^[[:space:]]*(#|$)' "$WAIVERS_FILE" 2>/dev/null || true)
  check "waivers: committed list is well-formed" "0" "$bad_waiver"

  echo ""
  echo "self-test: $passed assertions passed"
}

# ---------------------------------------------------------------------------
# Aggregate classes
# ---------------------------------------------------------------------------

run_class() {
  local class="$1"; shift
  local out="mutants-${class}"
  # cargo-mutants exits 2 when mutants are missed; the gate decides below.
  # shellcheck disable=SC2086
  cargo mutants --no-shuffle -o "$out" "$@" ${MUTANTS_ARGS:-} || true

  local json="$out/mutants.out/outcomes.json"
  if [[ ! -f "$json" ]]; then
    echo "::error::cargo-mutants produced no outcomes for $class"
    status=1
    return
  fi

  local caught missed timeout unviable score
  IFS=$'\t' read -r caught missed timeout unviable < <(read_outcomes "$json")
  score="$(kill_rate "$caught" "$missed" "$timeout")"

  {
    echo "### Mutation score — $class: ${score}%"
    echo ""
    echo "| caught | missed | timeout | unviable |"
    echo "|---|---|---|---|"
    echo "| $caught | $missed | $timeout | $unviable |"
    echo ""
    if [[ -s "$out/mutants.out/missed.txt" ]]; then
      echo "<details><summary>Survivors ($class)</summary>"
      echo ""
      echo '```'
      cat "$out/mutants.out/missed.txt"
      echo '```'
      echo "</details>"
    fi
  } | tee -a "$SUMMARY"

  if [[ "$class" == critical ]]; then
    if (( missed > 0 || timeout > 0 )); then
      echo "::error::$((missed + timeout)) security-critical mutant(s) survived"
      status=1
    fi
  elif ! score_passes "$score" "$GENERAL_THRESHOLD"; then
    echo "::error::general mutation score ${score}% is below ${GENERAL_THRESHOLD}%"
    status=1
  fi
}

critical_args=()
general_args=()
for f in "${SECURITY_CRITICAL[@]}"; do
  critical_args+=(--file "$SRC/$f")
  general_args+=(--exclude "$SRC/$f")
done

case "$CLASS" in
  critical)   run_class critical "${critical_args[@]}" ;;
  general)    run_class general "${general_args[@]}" ;;
  per-module) generate_per_module; report_per_module ;;
  self-test)  self_test ;;
  all)
    run_class critical "${critical_args[@]}"
    run_class general "${general_args[@]}"
    generate_per_module
    report_per_module
    ;;
  *) echo "usage: $0 critical|general|per-module|all|self-test" >&2; exit 64 ;;
esac

exit "$status"
