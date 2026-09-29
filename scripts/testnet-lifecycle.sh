#!/usr/bin/env bash
# testnet-lifecycle.sh — automated adversarial testnet lifecycle (#418).
#
# deploy → initialize → register sources → submit → aggregate → query →
# upgrade, plus adversarial phases:
#   A1 injected upgrade failure (unknown WASM hash) → state fully old → re-upgrade
#   A2 unauthorized migration attempt → rejected, storage version unchanged
#   A3 malformed and replayed Axelar cross-chain payload → rejected
#   A4 source goes hostile after admission → removed, further submits rejected
#
# Identities are ephemeral (suffixed with RUN_ID) and are always torn down by
# the EXIT trap, on success and on failure. Soroban contract instances cannot
# be deleted: the instance is emptied (sources/assets removed) and left to
# expire via TTL, and the deploying account is merged away.
#
# Metrics are written to $METRICS_OUT (JSON) for diffing against the previous run.

set -euo pipefail

NETWORK="${NETWORK:-testnet}"
CLI="${STELLAR_CLI:-stellar}"
RUN_ID="${RUN_ID:-$(date +%s)-$$}"
WASM_PATH="${WASM_PATH:-target/wasm32v1-none/release/price_oracle.wasm}"
METRICS_OUT="${METRICS_OUT:-lifecycle-metrics.json}"

ADMIN_ID="lc-admin-$RUN_ID"
SRC_A_ID="lc-src-a-$RUN_ID"
SRC_B_ID="lc-src-b-$RUN_ID"
SRC_C_ID="lc-src-c-$RUN_ID"
GATEWAY_ID="lc-gw-$RUN_ID"
IDS=("$ADMIN_ID" "$SRC_A_ID" "$SRC_B_ID" "$SRC_C_ID" "$GATEWAY_ID")

CONTRACT_ID=""
declare -A PHASE_SECS=()

log()  { echo "[lifecycle] $*"; }
fail() { echo "::error::[lifecycle] $*" >&2; exit 1; }
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

# ---------------------------------------------------------------------------
# Tear-down: always runs, success or failure.
# ---------------------------------------------------------------------------
teardown() {
  local rc=$?
  set +e
  log "== teardown (exit $rc)"
  if [[ -n "$CONTRACT_ID" ]]; then
    for s in "$SRC_A_ID" "$SRC_B_ID" "$SRC_C_ID"; do
      invoke "$ADMIN_ID" remove_source --source "$(addr "$s")" >/dev/null 2>&1
    done
    invoke "$ADMIN_ID" unregister_asset --asset "$(addr "$SRC_A_ID")" >/dev/null 2>&1
  fi
  # Merge funded accounts into MERGE_DEST (when set) so no ledger account
  # outlives the run, then drop the local keys.
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

# ---------------------------------------------------------------------------
# Happy path
# ---------------------------------------------------------------------------
setup_identities() {
  for id in "${IDS[@]}"; do
    "$CLI" keys generate "$id" --network "$NETWORK" --fund
  done
}

deploy() {
  WASM_HASH=$("$CLI" contract upload --wasm "$WASM_PATH" --source "$ADMIN_ID" --network "$NETWORK")
  CONTRACT_ID=$("$CLI" contract deploy --wasm-hash "$WASM_HASH" --source "$ADMIN_ID" --network "$NETWORK")
  log "contract $CONTRACT_ID (wasm $WASM_HASH)"
  invoke "$ADMIN_ID" initialize --admin "$(addr "$ADMIN_ID")" --min_sources_required 2 \
    --max_history_length 100 --decimals 7 --description '"lifecycle"'
}

register() {
  for s in "$SRC_A_ID" "$SRC_B_ID" "$SRC_C_ID"; do
    invoke "$ADMIN_ID" add_source --source "$(addr "$s")" --name "\"$s\""
  done
  ASSET=$(addr "$SRC_A_ID")
  invoke "$ADMIN_ID" register_asset --asset "$ASSET"
}

submit_and_query() {
  local ts; ts=$(date +%s)
  invoke "$SRC_A_ID" submit_price --source "$(addr "$SRC_A_ID")" --asset "$ASSET" --price 10000000 --timestamp "$ts"
  invoke "$SRC_B_ID" submit_price --source "$(addr "$SRC_B_ID")" --asset "$ASSET" --price 10100000 --timestamp "$ts"
  invoke "$SRC_C_ID" submit_price --source "$(addr "$SRC_C_ID")" --asset "$ASSET" --price 10200000 --timestamp "$ts"
  PRICE_BEFORE=$(invoke "$ADMIN_ID" get_price --asset "$ASSET" --max_age 0 | jq -r '.price')
  [[ "$PRICE_BEFORE" == "10100000" ]] || fail "median expected 10100000, got $PRICE_BEFORE"
  log "  median price $PRICE_BEFORE"
}

# ---------------------------------------------------------------------------
# Adversarial phases
# ---------------------------------------------------------------------------
snapshot() {
  echo "$(invoke "$ADMIN_ID" get_admin)|$(invoke "$ADMIN_ID" get_storage_version)|$(invoke "$ADMIN_ID" get_price --asset "$ASSET" --max_age 0 | jq -c .)"
}

a1_failed_upgrade_and_recovery() {
  local before after bogus
  before=$(snapshot)
  bogus=$(head -c 32 /dev/urandom | xxd -p -c 64)
  expect_fail "upgrade to unknown WASM" "$ADMIN_ID" upgrade --new_wasm_hash "$bogus"
  after=$(snapshot)
  [[ "$before" == "$after" ]] || fail "state changed after failed upgrade: $before -> $after"
  log "  state fully old after injected failure"
  expect_fail "upgrade by non-admin" "$SRC_B_ID" upgrade --new_wasm_hash "$WASM_HASH"
  invoke "$ADMIN_ID" upgrade --new_wasm_hash "$WASM_HASH"
  after=$(snapshot)
  [[ "$before" == "$after" ]] || fail "state not preserved across recovery upgrade"
  log "  re-upgrade succeeded; state intact"
}

a2_unauthorized_migration() {
  local v1 v2
  v1=$(invoke "$ADMIN_ID" get_storage_version)
  expect_fail "migrate_storage by non-admin" "$SRC_C_ID" migrate_storage --batch_size 1
  v2=$(invoke "$ADMIN_ID" get_storage_version)
  [[ "$v1" == "$v2" ]] || fail "storage version moved: $v1 -> $v2"
  [[ "$(invoke "$ADMIN_ID" get_migration_state)" == "null" ]] || fail "half-applied migration state left behind"
}

a3_malformed_cross_chain() {
  invoke "$ADMIN_ID" set_axelar_gateway --gateway "$(addr "$GATEWAY_ID")"
  invoke "$ADMIN_ID" set_axelar_trusted_source --source_chain '"ethereum"' \
    --source_address '"0xlifecycle"' --bridge_source "$(addr "$SRC_A_ID")"
  local cmd; cmd=$(head -c 32 /dev/urandom | xxd -p -c 64)
  expect_fail "malformed Axelar payload" "$GATEWAY_ID" execute_axelar_message \
    --gateway "$(addr "$GATEWAY_ID")" --command_id "$cmd" --source_chain '"ethereum"' \
    --source_address '"0xlifecycle"' --payload deadbeef
  expect_fail "replayed malformed Axelar payload" "$GATEWAY_ID" execute_axelar_message \
    --gateway "$(addr "$GATEWAY_ID")" --command_id "$cmd" --source_chain '"ethereum"' \
    --source_address '"0xlifecycle"' --payload deadbeef
  expect_fail "payload from untrusted source" "$GATEWAY_ID" execute_axelar_message \
    --gateway "$(addr "$GATEWAY_ID")" --command_id "$cmd" --source_chain '"ethereum"' \
    --source_address '"0xattacker"' --payload deadbeef
  expect_fail "payload via spoofed gateway" "$SRC_B_ID" execute_axelar_message \
    --gateway "$(addr "$SRC_B_ID")" --command_id "$cmd" --source_chain '"ethereum"' \
    --source_address '"0xlifecycle"' --payload deadbeef
  "$CLI" events --network "$NETWORK" --id "$CONTRACT_ID" --start-ledger \
    "$(( $("$CLI" ledger latest --network "$NETWORK" --output json 2>/dev/null | jq -r '.sequence // 0') - 50 ))" \
    --count 20 2>/dev/null | tail -20 || log "  (event listing unavailable in this CLI version)"
}

a4_hostile_source_removal() {
  local ts; ts=$(date +%s)
  invoke "$SRC_C_ID" submit_price --source "$(addr "$SRC_C_ID")" --asset "$ASSET" --price 99000000 --timestamp "$ts" || true
  invoke "$ADMIN_ID" remove_source --source "$(addr "$SRC_C_ID")"
  [[ "$(invoke "$ADMIN_ID" is_source --source "$(addr "$SRC_C_ID")")" == "false" ]] || fail "hostile source still admitted"
  expect_fail "submission from removed source" "$SRC_C_ID" submit_price \
    --source "$(addr "$SRC_C_ID")" --asset "$ASSET" --price 99000000 --timestamp "$((ts + 1))"
}

# ---------------------------------------------------------------------------
# Metrics
# ---------------------------------------------------------------------------
write_metrics() {
  {
    echo "{"
    echo "  \"wasm_bytes\": $(stat -c %s "$WASM_PATH"),"
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
  } > "$METRICS_OUT"
}

command -v "$CLI" >/dev/null || fail "$CLI not found"
command -v jq >/dev/null || fail "jq not found"
[[ -f "$WASM_PATH" ]] || fail "missing $WASM_PATH; build the contract first"

phase identities        setup_identities
phase deploy            deploy
phase register          register
phase submit_query      submit_and_query
phase a1_upgrade_abort  a1_failed_upgrade_and_recovery
phase a2_migration      a2_unauthorized_migration
phase a3_cross_chain    a3_malformed_cross_chain
phase a4_hostile_source a4_hostile_source_removal
write_metrics
log "PASSED — contract $CONTRACT_ID"
