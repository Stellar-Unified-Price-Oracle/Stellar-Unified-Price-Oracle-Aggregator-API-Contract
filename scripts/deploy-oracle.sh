#!/usr/bin/env bash
# deploy-oracle.sh — Interactive guided deployment CLI for the Price Oracle contract.
#
# Walks an operator through network selection, admin setup, initialization
# parameters, source/asset registration, and a test submission, then runs
# post-deployment verification and writes a deployment report.
#
# Usage: ./scripts/deploy-oracle.sh [--wasm <path/to/price_oracle.wasm>]
#
# Prerequisites: stellar CLI (https://developers.stellar.org/docs/tools/developer-tools)

set -euo pipefail

WASM_PATH="target/wasm32v1-none/release/price_oracle.wasm"
REPORT_FILE="deployment-report-$(date +%Y%m%d-%H%M%S).md"

info()  { echo -e "\033[1;33m==>\033[0m $*"; }
die()   { echo "ERROR: $*" >&2; exit 1; }
ask()   { local prompt="$1" def="${2:-}" reply; read -r -p "$prompt${def:+ [$def]}: " reply; echo "${reply:-$def}"; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --wasm) WASM_PATH="$2"; shift 2 ;;
    *) die "Unknown argument: $1" ;;
  esac
done

command -v stellar >/dev/null 2>&1 || die "stellar CLI not found on PATH"
[[ -f "$WASM_PATH" ]] || die "WASM not found at $WASM_PATH (build with 'npm run build:contract' first)"

info "Stellar Unified Price Oracle — Guided Deployment"

# ── 1. Network selection ─────────────────────────────────────────────────
echo "Select network:"
select NETWORK in testnet futurenet mainnet standalone; do
  [[ -n "$NETWORK" ]] && break
done

# ── 2. Admin setup ───────────────────────────────────────────────────────
ADMIN_IDENTITY=$(ask "Admin identity (stellar CLI identity name)" "default")
stellar keys address "$ADMIN_IDENTITY" >/dev/null 2>&1 || die "Identity '$ADMIN_IDENTITY' not found — run 'stellar keys generate $ADMIN_IDENTITY' first"
ADMIN_ADDRESS=$(stellar keys address "$ADMIN_IDENTITY")
info "Admin address: $ADMIN_ADDRESS"

# ── 3. Initialization parameters ─────────────────────────────────────────
MIN_SOURCES=$(ask "min_sources_required" "3")
MAX_HISTORY=$(ask "max_history_length" "100")
DECIMALS=$(ask "decimals" "14")
RESOLUTION=$(ask "resolution" "300")

# ── 4. Deploy ─────────────────────────────────────────────────────────────
info "Deploying contract to $NETWORK..."
CONTRACT_ID=$(stellar contract deploy \
  --wasm "$WASM_PATH" \
  --source "$ADMIN_IDENTITY" \
  --network "$NETWORK")
info "Deployed: $CONTRACT_ID"

info "Initializing..."
stellar contract invoke --id "$CONTRACT_ID" --source "$ADMIN_IDENTITY" --network "$NETWORK" \
  -- initialize --admin "$ADMIN_ADDRESS" \
  --min_sources_required "$MIN_SOURCES" --max_history_length "$MAX_HISTORY" \
  --decimals "$DECIMALS" --resolution "$RESOLUTION"

# ── 5. Source registration ───────────────────────────────────────────────
REGISTER_SOURCE=$(ask "Register an initial price source now? (y/n)" "n")
SOURCE_ADDRESS=""
if [[ "$REGISTER_SOURCE" == "y" ]]; then
  SOURCE_ADDRESS=$(ask "Source address")
  SOURCE_NAME=$(ask "Source name" "primary")
  stellar contract invoke --id "$CONTRACT_ID" --source "$ADMIN_IDENTITY" --network "$NETWORK" \
    -- add_source --source "$SOURCE_ADDRESS" --name "$SOURCE_NAME"
  info "Source registered: $SOURCE_ADDRESS"
fi

# ── 6. Asset registration ────────────────────────────────────────────────
REGISTER_ASSET=$(ask "Register an initial asset now? (y/n)" "n")
ASSET_ADDRESS=""
if [[ "$REGISTER_ASSET" == "y" ]]; then
  ASSET_ADDRESS=$(ask "Asset contract address")
  stellar contract invoke --id "$CONTRACT_ID" --source "$ADMIN_IDENTITY" --network "$NETWORK" \
    -- register_asset --asset "$ASSET_ADDRESS"
  info "Asset registered: $ASSET_ADDRESS"
fi

# ── 7. Test submission ───────────────────────────────────────────────────
TEST_SUBMIT="skipped"
if [[ -n "$SOURCE_ADDRESS" && -n "$ASSET_ADDRESS" ]]; then
  DO_TEST=$(ask "Submit a test price now? (y/n)" "n")
  if [[ "$DO_TEST" == "y" ]]; then
    TEST_PRICE=$(ask "Test price (integer, scaled by decimals)" "100000000000000")
    NOW=$(date +%s)
    stellar contract invoke --id "$CONTRACT_ID" --source "$ADMIN_IDENTITY" --network "$NETWORK" \
      -- submit_price --source "$SOURCE_ADDRESS" --asset "$ASSET_ADDRESS" \
      --price "$TEST_PRICE" --timestamp "$NOW"
    TEST_SUBMIT="ok (price=$TEST_PRICE @ $NOW)"
    info "Test submission successful."
  fi
fi

# ── 8. Post-deployment verification ──────────────────────────────────────
info "Running post-deployment verification..."
VERIFY_OUTPUT="(skipped — verify-deployment.sh not found)"
if [[ -x "$(dirname "$0")/verify-deployment.sh" ]]; then
  VERIFY_OUTPUT=$("$(dirname "$0")/verify-deployment.sh" \
    --contract "$CONTRACT_ID" --admin "$ADMIN_IDENTITY" --network "$NETWORK" 2>&1) || true
  echo "$VERIFY_OUTPUT"
fi

# ── 9. Deployment report ─────────────────────────────────────────────────
cat > "$REPORT_FILE" <<EOF
# Deployment Report

- **Date:** $(date -u +"%Y-%m-%dT%H:%M:%SZ")
- **Network:** $NETWORK
- **Contract ID:** $CONTRACT_ID
- **Admin:** $ADMIN_ADDRESS ($ADMIN_IDENTITY)
- **min_sources_required:** $MIN_SOURCES
- **max_history_length:** $MAX_HISTORY
- **decimals:** $DECIMALS
- **resolution:** $RESOLUTION
- **Initial source:** ${SOURCE_ADDRESS:-none}
- **Initial asset:** ${ASSET_ADDRESS:-none}
- **Test submission:** $TEST_SUBMIT

## Verification Output
\`\`\`
$VERIFY_OUTPUT
\`\`\`
EOF

info "Deployment report written to $REPORT_FILE"
info "Done."
