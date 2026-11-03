#!/usr/bin/env bash
# testnet-key-rotation.sh — source key rotation rehearsal on testnet (#641).
#
# Rehearses the staged rotation in docs/operations/source-key-rotation.md
# against a freshly deployed contract:
#   R1 pre-check     the outgoing key is an authorized source and can submit
#   R2 stage         the incoming key is added while the outgoing key stays
#   R3 verify        both keys are sources and both submissions are accepted
#   R4 rollback      the incoming key is removed again; the outgoing key keeps
#                    working, so an aborted rotation leaves the feed intact
#   R5 retire        the rotation is redone and the outgoing key removed; its
#                    submissions are rejected and the incoming key still works
#
# Identities are ephemeral (suffixed with RUN_ID) and always torn down by the
# EXIT trap. Metrics are written to $ROTATION_METRICS_OUT (JSON).

set -euo pipefail

NETWORK="${NETWORK:-testnet}"
CLI="${STELLAR_CLI:-stellar}"
RUN_ID="${RUN_ID:-$(date +%s)-$$}"
WASM_PATH="${WASM_PATH:-target/wasm32v1-none/release/price_oracle.wasm}"
ROTATION_METRICS_OUT="${ROTATION_METRICS_OUT:-rotation-metrics.json}"

ADMIN_ID="kr-admin-$RUN_ID"
OLD_ID="kr-old-$RUN_ID"
NEW_ID="kr-new-$RUN_ID"
PEER_ID="kr-peer-$RUN_ID"
ASSET_ID="kr-asset-$RUN_ID"
IDS=("$ADMIN_ID" "$OLD_ID" "$NEW_ID" "$PEER_ID" "$ASSET_ID")

CONTRACT_ID=""
declare -A PHASE_SECS=()

log()  { echo "[key-rotation] $*"; }
fail() { echo "::error::[key-rotation] $*" >&2; exit 1; }
addr() { "$CLI" keys address "$1"; }

invoke() {
  local as="$1"; shift
  "$CLI" contract invoke --id "$CONTRACT_ID" --network "$NETWORK" --source "$as" -- "$@"
}

# Runs an invocation that MUST fail; prints the rejection so it is visible in logs.
expect_fail() {
  local what="$1"; shift
  local out
  if out=$(invoke "$@" 2>&1); then
    fail "$what was accepted but must be rejected: $out"
  fi
  log "  rejected as expected ($what): $(echo "$out" | grep -m1 -Eo 'Error\(Contract, #[0-9]+\)|Error\([A-Za-z]+, [A-Za-z]+\)' || echo "${out##*$'\n'}")"
}

phase() {
  local name="$1"; shift
  local start=$SECONDS
  log "== $name"
  "$@"
  PHASE_SECS[$name]=$((SECONDS - start))
}

is_source() { invoke "$ADMIN_ID" is_source --source "$(addr "$1")"; }

# Submits a price as identity $1. Timestamps must increase per source.
submit() {
  local id="$1" price="$2"
  invoke "$id" submit_price --source "$(addr "$id")" --asset "$ASSET" \
    --price "$price" --timestamp "$(date +%s)"
}

teardown() {
  local rc=$?
  set +e
  log "== teardown (exit $rc)"
  if [[ -n "$CONTRACT_ID" ]]; then
    for s in "$OLD_ID" "$NEW_ID" "$PEER_ID"; do
      invoke "$ADMIN_ID" remove_source --source "$(addr "$s")" >/dev/null 2>&1
    done
  fi
  for id in "${IDS[@]}"; do
    if "$CLI" keys show "$id" >/dev/null 2>&1; then
      [[ -n "${MERGE_DEST:-}" ]] && "$CLI" tx new account-merge --source "$id" \
        --account "$MERGE_DEST" --network "$NETWORK" >/dev/null 2>&1
      "$CLI" keys rm "$id" >/dev/null 2>&1
    fi
  done
  log "identities removed: ${IDS[*]}"
  exit "$rc"
}
trap teardown EXIT

setup() {
  for id in "${IDS[@]}"; do
    "$CLI" keys generate "$id" --network "$NETWORK" --fund
  done
  local hash
  hash=$("$CLI" contract upload --wasm "$WASM_PATH" --source "$ADMIN_ID" --network "$NETWORK")
  CONTRACT_ID=$("$CLI" contract deploy --wasm-hash "$hash" --source "$ADMIN_ID" --network "$NETWORK")
  log "contract $CONTRACT_ID"
  invoke "$ADMIN_ID" initialize --admin "$(addr "$ADMIN_ID")" --min_sources_required 1 \
    --max_history_length 100 --decimals 7 --description '"key-rotation"'
  invoke "$ADMIN_ID" add_source --source "$(addr "$OLD_ID")" --name '"old"'
  invoke "$ADMIN_ID" add_source --source "$(addr "$PEER_ID")" --name '"peer"'
  ASSET=$(addr "$ASSET_ID")
  invoke "$ADMIN_ID" register_asset --asset "$ASSET"
}

r1_precheck() {
  [[ "$(is_source "$OLD_ID")" == "true" ]] || fail "outgoing key is not an authorized source"
  submit "$OLD_ID" 10000000
  submit "$PEER_ID" 10000000
}

r2_stage() {
  invoke "$ADMIN_ID" add_source --source "$(addr "$NEW_ID")" --name '"new"'
}

r3_verify_overlap() {
  [[ "$(is_source "$OLD_ID")" == "true" ]] || fail "outgoing key lost authority during overlap"
  [[ "$(is_source "$NEW_ID")" == "true" ]] || fail "incoming key not authorized after staging"
  sleep 1
  submit "$OLD_ID" 10010000
  submit "$NEW_ID" 10010000
  log "  both keys accepted during the overlap"
}

r4_rollback() {
  invoke "$ADMIN_ID" remove_source --source "$(addr "$NEW_ID")"
  [[ "$(is_source "$NEW_ID")" == "false" ]] || fail "rolled-back key still authorized"
  sleep 1
  submit "$OLD_ID" 10020000
  expect_fail "submission from rolled-back key" "$NEW_ID" submit_price \
    --source "$(addr "$NEW_ID")" --asset "$ASSET" --price 10020000 --timestamp "$(date +%s)"
  log "  rollback left the outgoing key working"
}

r5_retire() {
  invoke "$ADMIN_ID" add_source --source "$(addr "$NEW_ID")" --name '"new"'
  invoke "$ADMIN_ID" remove_source --source "$(addr "$OLD_ID")"
  [[ "$(is_source "$OLD_ID")" == "false" ]] || fail "retired key still authorized"
  sleep 1
  expect_fail "submission from retired key" "$OLD_ID" submit_price \
    --source "$(addr "$OLD_ID")" --asset "$ASSET" --price 10030000 --timestamp "$(date +%s)"
  submit "$NEW_ID" 10030000
  log "  retired key rejected; incoming key serving"
}

write_metrics() {
  {
    echo "{"
    echo "  \"total_seconds\": $SECONDS,"
    echo "  \"phases\": {"
    local first=1
    for k in "${!PHASE_SECS[@]}"; do
      [[ $first == 1 ]] || echo ","
      printf '    "%s": %s' "$k" "${PHASE_SECS[$k]}"
      first=0
    done
    echo ""
    echo "  }"
    echo "}"
  } > "$ROTATION_METRICS_OUT"
}

command -v "$CLI" >/dev/null || fail "$CLI not found"
command -v jq >/dev/null || fail "jq not found"
[[ -f "$WASM_PATH" ]] || fail "missing $WASM_PATH; build the contract first"

phase setup      setup
phase r1_precheck r1_precheck
phase r2_stage   r2_stage
phase r3_overlap r3_verify_overlap
phase r4_rollback r4_rollback
phase r5_retire  r5_retire
write_metrics
log "PASSED — contract $CONTRACT_ID"
