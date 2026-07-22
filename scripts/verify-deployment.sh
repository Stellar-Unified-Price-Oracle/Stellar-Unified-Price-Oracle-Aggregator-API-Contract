#!/usr/bin/env bash
# verify-deployment.sh — Post-deployment sanity check for the Price Oracle contract.
#
# Usage:
#   ./scripts/verify-deployment.sh \
#       --contract <CONTRACT_ID> \
#       --admin    <ADMIN_IDENTITY>   \   # soroban identity name or keypair file
#       --network  <NETWORK>             # testnet | mainnet | standalone
#
# Environment variables (alternative to flags):
#   CONTRACT_ID, ADMIN_IDENTITY, NETWORK
#
# Exit codes: 0 = all checks passed, 1 = one or more checks failed.

set -euo pipefail

# ---------------------------------------------------------------------------
# Helpers
# ---------------------------------------------------------------------------
RED='\033[0;31m'; GREEN='\033[0;32m'; YELLOW='\033[1;33m'; NC='\033[0m'
PASS="[${GREEN}PASS${NC}]"; FAIL="[${RED}FAIL${NC}]"
FAILURES=0

info()  { echo -e "${YELLOW}[INFO]${NC} $*"; }
pass()  { echo -e "${PASS} $*"; }
fail()  { echo -e "${FAIL} $*"; FAILURES=$((FAILURES + 1)); }

check() {
    local desc="$1"; local expected="$2"; local actual="$3"
    if [[ "$actual" == *"$expected"* ]]; then
        pass "$desc"
    else
        fail "$desc — expected '$expected', got '$actual'"
    fi
}

invoke() {
    # Invoke a read-only contract function and return its output.
    stellar contract invoke \
        --id    "$CONTRACT_ID" \
        --source "$ADMIN_IDENTITY" \
        --network "$NETWORK" \
        -- "$@" 2>&1
}

invoke_auth() {
    # Invoke a function that requires the admin to authorise (write op).
    stellar contract invoke \
        --id    "$CONTRACT_ID" \
        --source "$ADMIN_IDENTITY" \
        --network "$NETWORK" \
        -- "$@" 2>&1
}

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
    case "$1" in
        --contract)  CONTRACT_ID="$2";       shift 2 ;;
        --admin)     ADMIN_IDENTITY="$2";    shift 2 ;;
        --network)   NETWORK="$2";           shift 2 ;;
        *) echo "Unknown argument: $1"; exit 1 ;;
    esac
done

CONTRACT_ID="${CONTRACT_ID:?'--contract <CONTRACT_ID> is required'}"
ADMIN_IDENTITY="${ADMIN_IDENTITY:?'--admin <ADMIN_IDENTITY> is required'}"
NETWORK="${NETWORK:-testnet}"

# Resolve the admin's public key from the identity name so we can compare it
# against what the contract reports.
ADMIN_ADDRESS=$(stellar keys address "$ADMIN_IDENTITY" 2>/dev/null || echo "$ADMIN_IDENTITY")

# Test identities — generated on the fly so they are never real accounts.
TEST_SOURCE_SECRET=$(stellar keys generate --no-fund --overwrite _verify_source 2>/dev/null; stellar keys show _verify_source 2>/dev/null || true)
TEST_SOURCE_ADDRESS=$(stellar keys address _verify_source 2>/dev/null || true)
TEST_ASSET="VERIFY_TEST"
TEST_PRICE="100000000"       # 1.0 with 8 decimals
TEST_TIMESTAMP="1000000000"

# ---------------------------------------------------------------------------
# 1. Admin address
# ---------------------------------------------------------------------------
info "=== 1. Admin address ==="
RESULT=$(invoke get_admin_address)
check "admin address matches deployer" "$ADMIN_ADDRESS" "$RESULT"

# ---------------------------------------------------------------------------
# 2. min_sources and max_history
# ---------------------------------------------------------------------------
info "=== 2. min_sources / max_history ==="
MIN_SRC=$(invoke get_min_sources_required)
MAX_HIST=$(invoke get_max_history_length)
[[ "$MIN_SRC" =~ ^[0-9]+$ ]] && pass "min_sources is a number ($MIN_SRC)" || fail "min_sources not numeric: $MIN_SRC"
[[ "$MAX_HIST" =~ ^[0-9]+$ ]] && pass "max_history is a number ($MAX_HIST)" || fail "max_history not numeric: $MAX_HIST"

# ---------------------------------------------------------------------------
# 3. decimals, resolution, description
# ---------------------------------------------------------------------------
info "=== 3. decimals / resolution / description ==="
DECIMALS=$(invoke get_decimals)
RESOLUTION=$(invoke resolution)
DESCRIPTION=$(invoke get_description)
[[ "$DECIMALS" =~ ^[0-9]+$ ]] && pass "decimals is a number ($DECIMALS)" || fail "decimals not numeric: $DECIMALS"
[[ "$RESOLUTION" =~ ^[0-9]+$ ]] && pass "resolution is a number ($RESOLUTION)" || fail "resolution not numeric: $RESOLUTION"
[[ -n "$DESCRIPTION" ]] && pass "description is set" || fail "description is empty"

# ---------------------------------------------------------------------------
# 4. Register test source and verify
# ---------------------------------------------------------------------------
info "=== 4. Register test source ==="
if [[ -z "$TEST_SOURCE_ADDRESS" ]]; then
    fail "could not generate test source identity"
else
    invoke_auth add_source \
        --address "$TEST_SOURCE_ADDRESS" \
        --name    "verify-test-source" > /dev/null 2>&1 || true
    SOURCE_CHECK=$(invoke is_source --address "$TEST_SOURCE_ADDRESS")
    check "test source is registered" "true" "$SOURCE_CHECK"
fi

# ---------------------------------------------------------------------------
# 5. Register test asset and verify
# ---------------------------------------------------------------------------
info "=== 5. Register test asset ==="
invoke_auth register_asset --asset "$TEST_ASSET" > /dev/null 2>&1 || true
ASSET_CHECK=$(invoke is_asset_registered --asset "$TEST_ASSET")
check "test asset is registered" "true" "$ASSET_CHECK"

# ---------------------------------------------------------------------------
# 6. Submit test price and verify
# ---------------------------------------------------------------------------
info "=== 6. Submit test price ==="
# Price submission must be signed by the source identity.
stellar contract invoke \
    --id      "$CONTRACT_ID" \
    --source  "_verify_source" \
    --network "$NETWORK" \
    -- submit_price \
        --source    "$TEST_SOURCE_ADDRESS" \
        --asset     "$TEST_ASSET" \
        --price     "$TEST_PRICE" \
        --timestamp "$TEST_TIMESTAMP" > /dev/null 2>&1 || true

PRICE_RESULT=$(invoke get_source_price \
    --asset  "$TEST_ASSET" \
    --source "$TEST_SOURCE_ADDRESS" 2>&1 || echo "")
if [[ "$PRICE_RESULT" == *"$TEST_PRICE"* ]]; then
    pass "submitted price is retrievable"
else
    fail "submitted price not found — got: $PRICE_RESULT"
fi

# ---------------------------------------------------------------------------
# 7. Clean up test data
# ---------------------------------------------------------------------------
info "=== 7. Cleanup ==="
invoke_auth remove_source   --address "$TEST_SOURCE_ADDRESS" > /dev/null 2>&1 || true
invoke_auth unregister_asset --asset  "$TEST_ASSET"          > /dev/null 2>&1 || true

SRC_AFTER=$(invoke  is_source          --address "$TEST_SOURCE_ADDRESS" 2>&1 || echo "false")
ASSET_AFTER=$(invoke is_asset_registered --asset "$TEST_ASSET"           2>&1 || echo "false")
check "test source removed"       "false" "$SRC_AFTER"
check "test asset unregistered"   "false" "$ASSET_AFTER"

# Clean up ephemeral local key
stellar keys rm _verify_source 2>/dev/null || true

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
echo ""
if [[ "$FAILURES" -eq 0 ]]; then
    echo -e "${GREEN}All verification checks passed.${NC}"
    exit 0
else
    echo -e "${RED}${FAILURES} check(s) failed.${NC}"
    exit 1
fi
