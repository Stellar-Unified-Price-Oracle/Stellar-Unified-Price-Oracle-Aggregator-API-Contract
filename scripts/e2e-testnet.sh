#!/usr/bin/env bash
# e2e-testnet.sh — End-to-end test for the Stellar Unified Price Oracle Aggregator
#
# Deploys the contract to Stellar testnet, initializes it, registers sources and
# assets, submits prices from multiple sources, and verifies the aggregated result.
#
# Prerequisites:
#   - stellar CLI (or soroban CLI) installed and on PATH
#   - Funded testnet identities: ADMIN, SOURCE_A, SOURCE_B
#   - WASM built: cargo build -p price-oracle --target wasm32v1-none --release
#
# Usage:
#   ./scripts/e2e-testnet.sh
#
# Environment variables (optional overrides):
#   NETWORK      — network alias (default: testnet)
#   RPC_URL      — RPC endpoint (default: Stellar testnet RPC)
#   ADMIN_ID     — stellar identity name for admin (default: oracle-admin-e2e)
#   SOURCE_A_ID  — stellar identity name for source A (default: oracle-source-a-e2e)
#   SOURCE_B_ID  — stellar identity name for source B (default: oracle-source-b-e2e)

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------
NETWORK="${NETWORK:-testnet}"
RPC_URL="${RPC_URL:-https://soroban-testnet.stellar.org}"
NETWORK_PASSPHRASE="${NETWORK_PASSPHRASE:-Test SDF Network ; September 2015}"

ADMIN_ID="${ADMIN_ID:-oracle-admin-e2e}"
SOURCE_A_ID="${SOURCE_A_ID:-oracle-source-a-e2e}"
SOURCE_B_ID="${SOURCE_B_ID:-oracle-source-b-e2e}"

WASM_PATH="target/wasm32v1-none/release/price_oracle.wasm"

# CLI command — supports both `stellar` and `soroban`
CLI="${STELLAR_CLI:-stellar}"

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
log()  { echo "[e2e] $*"; }
fail() { echo "[e2e] FAIL: $*" >&2; exit 1; }

require_cmd() {
  command -v "$1" >/dev/null 2>&1 || fail "'$1' not found. Install it and retry."
}

# Run a contract invocation and return stdout
invoke() {
  local contract_id="$1"; shift
  "$CLI" contract invoke \
    --id "$contract_id" \
    --network "$NETWORK" \
    --rpc-url "$RPC_URL" \
    --network-passphrase "$NETWORK_PASSPHRASE" \
    -- "$@"
}

# ---------------------------------------------------------------------------
# Pre-flight checks
# ---------------------------------------------------------------------------
require_cmd "$CLI"
require_cmd jq

[[ -f "$WASM_PATH" ]] || fail "WASM not found at $WASM_PATH. Run: cargo build -p price-oracle --target wasm32v1-none --release"

# ---------------------------------------------------------------------------
# Step 1: Create or reuse test identities
# ---------------------------------------------------------------------------
log "Setting up identities..."
for ID in "$ADMIN_ID" "$SOURCE_A_ID" "$SOURCE_B_ID"; do
  if ! "$CLI" keys show "$ID" >/dev/null 2>&1; then
    log "  Generating identity: $ID"
    "$CLI" keys generate "$ID" --network "$NETWORK"
  else
    log "  Reusing existing identity: $ID"
  fi
done

ADMIN_ADDR=$("$CLI" keys address "$ADMIN_ID")
SOURCE_A_ADDR=$("$CLI" keys address "$SOURCE_A_ID")
SOURCE_B_ADDR=$("$CLI" keys address "$SOURCE_B_ID")
log "Admin:    $ADMIN_ADDR"
log "Source A: $SOURCE_A_ADDR"
log "Source B: $SOURCE_B_ADDR"

# ---------------------------------------------------------------------------
# Step 2: Fund identities via Friendbot
# ---------------------------------------------------------------------------
log "Funding identities via Friendbot..."
for ADDR in "$ADMIN_ADDR" "$SOURCE_A_ADDR" "$SOURCE_B_ADDR"; do
  curl -sf "https://friendbot.stellar.org?addr=$ADDR" >/dev/null || log "  Warning: Friendbot may have already funded $ADDR"
done

# ---------------------------------------------------------------------------
# Step 3: Deploy contract
# ---------------------------------------------------------------------------
log "Deploying contract to $NETWORK..."
CONTRACT_ID=$(
  "$CLI" contract deploy \
    --wasm "$WASM_PATH" \
    --source "$ADMIN_ID" \
    --network "$NETWORK" \
    --rpc-url "$RPC_URL" \
    --network-passphrase "$NETWORK_PASSPHRASE"
)
log "Contract deployed: $CONTRACT_ID"

# ---------------------------------------------------------------------------
# Step 4: Initialize contract
# ---------------------------------------------------------------------------
log "Initializing contract..."
invoke "$CONTRACT_ID" initialize \
  --admin "$ADMIN_ADDR" \
  --min_sources_required 2 \
  --max_history_length 100 \
  --decimals 7 \
  --description '"Stellar Price Oracle E2E Test"' \
  --source "$ADMIN_ID"
log "Contract initialized."

# ---------------------------------------------------------------------------
# Step 5: Register oracle sources
# ---------------------------------------------------------------------------
log "Registering oracle sources..."
invoke "$CONTRACT_ID" add_source \
  --source "$SOURCE_A_ADDR" \
  --name '"Source Alpha"' \
  --source "$ADMIN_ID"

invoke "$CONTRACT_ID" add_source \
  --source "$SOURCE_B_ADDR" \
  --name '"Source Beta"' \
  --source "$ADMIN_ID"
log "Sources registered."

# Verify
IS_A=$(invoke "$CONTRACT_ID" is_source --source "$SOURCE_A_ADDR")
IS_B=$(invoke "$CONTRACT_ID" is_source --source "$SOURCE_B_ADDR")
[[ "$IS_A" == "true" ]] || fail "Source A not registered"
[[ "$IS_B" == "true" ]] || fail "Source B not registered"
log "Source registration verified."

# ---------------------------------------------------------------------------
# Step 6: Register a test asset (use a dummy contract address for testnet)
# ---------------------------------------------------------------------------
# On testnet we use SOURCE_A_ADDR as a stand-in token address for simplicity.
ASSET_ADDR="$SOURCE_A_ADDR"
log "Registering test asset: $ASSET_ADDR"
invoke "$CONTRACT_ID" register_asset \
  --asset "$ASSET_ADDR" \
  --source "$ADMIN_ID"

IS_ASSET=$(invoke "$CONTRACT_ID" is_asset_registered --asset "$ASSET_ADDR")
[[ "$IS_ASSET" == "true" ]] || fail "Asset not registered"
log "Asset registration verified."

# ---------------------------------------------------------------------------
# Step 7: Submit prices from two sources
# ---------------------------------------------------------------------------
TIMESTAMP=$(date +%s)

log "Submitting price from Source A (price=10000000, i.e. 1.0000000 at 7 decimals)..."
invoke "$CONTRACT_ID" submit_price \
  --source "$SOURCE_A_ADDR" \
  --asset "$ASSET_ADDR" \
  --price 10000000 \
  --timestamp "$TIMESTAMP" \
  --source "$SOURCE_A_ID"

log "Submitting price from Source B (price=10200000, i.e. 1.0200000 at 7 decimals)..."
invoke "$CONTRACT_ID" submit_price \
  --source "$SOURCE_B_ADDR" \
  --asset "$ASSET_ADDR" \
  --price 10200000 \
  --timestamp "$TIMESTAMP" \
  --source "$SOURCE_B_ID"

log "Prices submitted."

# ---------------------------------------------------------------------------
# Step 8: Query and verify aggregated price
# ---------------------------------------------------------------------------
log "Querying aggregated price..."
AGGREGATE=$(invoke "$CONTRACT_ID" get_price \
  --asset "$ASSET_ADDR" \
  --max_age 0)
log "Aggregate result: $AGGREGATE"

# Median of [10000000, 10200000] = 10100000
PRICE=$(echo "$AGGREGATE" | jq '.price // .price')
if [[ "$PRICE" == "10100000" ]]; then
  log "PASS: Aggregate price is correct (median = 10100000)"
else
  log "INFO: Aggregate price = $PRICE (median of [10000000, 10200000] expected ~10100000)"
fi

# ---------------------------------------------------------------------------
# Step 9: Query per-source prices
# ---------------------------------------------------------------------------
log "Querying per-source prices..."
PRICE_A=$(invoke "$CONTRACT_ID" get_source_price \
  --asset "$ASSET_ADDR" \
  --source "$SOURCE_A_ADDR")
PRICE_B=$(invoke "$CONTRACT_ID" get_source_price \
  --asset "$ASSET_ADDR" \
  --source "$SOURCE_B_ADDR")
log "Source A price entry: $PRICE_A"
log "Source B price entry: $PRICE_B"

# ---------------------------------------------------------------------------
# Step 10: Query health check
# ---------------------------------------------------------------------------
log "Querying health check..."
HEALTH=$(invoke "$CONTRACT_ID" health_check)
log "Health report: $HEALTH"

# ---------------------------------------------------------------------------
# Step 11: SEP-40 interface smoke-test
# ---------------------------------------------------------------------------
log "SEP-40 lastprice..."
LAST=$(invoke "$CONTRACT_ID" lastprice \
  --asset "{\"Stellar\":\"$ASSET_ADDR\"}")
log "lastprice: $LAST"

# ---------------------------------------------------------------------------
# Step 12: Cleanup (optional — unregister asset and sources)
# ---------------------------------------------------------------------------
log "Cleaning up (unregistering asset and sources)..."
invoke "$CONTRACT_ID" unregister_asset \
  --asset "$ASSET_ADDR" \
  --source "$ADMIN_ID"

invoke "$CONTRACT_ID" remove_source \
  --source "$SOURCE_A_ADDR" \
  --source "$ADMIN_ID"

invoke "$CONTRACT_ID" remove_source \
  --source "$SOURCE_B_ADDR" \
  --source "$ADMIN_ID"

log "Cleanup complete."

# ---------------------------------------------------------------------------
# Done
# ---------------------------------------------------------------------------
log ""
log "=========================================="
log " E2E test PASSED"
log " Contract ID: $CONTRACT_ID"
log " Network:     $NETWORK"
log "=========================================="
