#!/usr/bin/env bash
# deterministic-deploy.sh — Soroban deterministic contract deployment
# Uses a salt to produce a predictable contract address before deployment.
#
# Usage:
#   ./scripts/deterministic-deploy.sh \
#     --deployer <identity-name> \
#     --network <testnet|mainnet|standalone> \
#     --salt <64-char-hex> \
#     --wasm <path-to-wasm> \
#     --admin <address> \
#     [--decimals <n>] \
#     [--description <str>] \
#     [--dry-run]

set -euo pipefail

# ── Defaults ────────────────────────────────────────────────────────────────
DEPLOYER=""
NETWORK="testnet"
SALT=""
WASM=""
ADMIN=""
DECIMALS="18"
DESCRIPTION="Stellar Unified Price Oracle"
DRY_RUN=false

# ── Argument parsing ─────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
  case "$1" in
    --deployer)   DEPLOYER="$2";   shift 2 ;;
    --network)    NETWORK="$2";    shift 2 ;;
    --salt)       SALT="$2";       shift 2 ;;
    --wasm)       WASM="$2";       shift 2 ;;
    --admin)      ADMIN="$2";      shift 2 ;;
    --decimals)   DECIMALS="$2";   shift 2 ;;
    --description) DESCRIPTION="$2"; shift 2 ;;
    --dry-run)    DRY_RUN=true;    shift ;;
    *) echo "Unknown argument: $1" >&2; exit 1 ;;
  esac
done

# ── Validation ───────────────────────────────────────────────────────────────
for var in DEPLOYER SALT WASM ADMIN; do
  if [[ -z "${!var}" ]]; then
    echo "ERROR: --${var,,} is required" >&2
    exit 1
  fi
done

if [[ ! -f "$WASM" ]]; then
  echo "ERROR: WASM file not found: $WASM" >&2
  exit 1
fi

# Salt must be exactly 64 hex chars (32 bytes)
if [[ ! "$SALT" =~ ^[0-9a-fA-F]{64}$ ]]; then
  echo "ERROR: --salt must be exactly 64 hex characters (32 bytes)" >&2
  exit 1
fi

echo "====================================================="
echo " Stellar Deterministic Oracle Deployment"
echo "====================================================="
echo " Deployer : $DEPLOYER"
echo " Network  : $NETWORK"
echo " WASM     : $WASM"
echo " Salt     : $SALT"
echo " Admin    : $ADMIN"
echo " Dry run  : $DRY_RUN"
echo "====================================================="

# ── Step 1: Upload WASM (get hash) ───────────────────────────────────────────
echo ""
echo "[1/4] Uploading WASM to get hash..."
WASM_HASH=$(stellar contract upload \
  --wasm "$WASM" \
  --source "$DEPLOYER" \
  --network "$NETWORK")
echo "      WASM hash: $WASM_HASH"

# ── Step 2: Pre-compute contract address ────────────────────────────────────
echo ""
echo "[2/4] Pre-computing contract address..."
EXPECTED_ADDRESS=$(stellar contract id wasm \
  --wasm-hash "$WASM_HASH" \
  --salt "$SALT" \
  --source "$DEPLOYER" \
  --network "$NETWORK")
echo "      Expected address: $EXPECTED_ADDRESS"

if [[ "$DRY_RUN" == true ]]; then
  echo ""
  echo "[DRY RUN] Would deploy to: $EXPECTED_ADDRESS"
  echo "[DRY RUN] No deployment performed."
  exit 0
fi

# ── Step 3: Deploy with salt ─────────────────────────────────────────────────
echo ""
echo "[3/4] Deploying contract with salt..."
ACTUAL_ADDRESS=$(stellar contract deploy \
  --wasm-hash "$WASM_HASH" \
  --salt "$SALT" \
  --source "$DEPLOYER" \
  --network "$NETWORK")
echo "      Actual address:   $ACTUAL_ADDRESS"

# ── Step 4: Verify address match ─────────────────────────────────────────────
echo ""
echo "[4/4] Verifying address match..."
if [[ "$EXPECTED_ADDRESS" == "$ACTUAL_ADDRESS" ]]; then
  echo "      ✓ Address match verified!"
else
  echo "      ✗ ERROR: Address mismatch!" >&2
  echo "        Expected: $EXPECTED_ADDRESS" >&2
  echo "        Actual:   $ACTUAL_ADDRESS" >&2
  exit 1
fi

# ── Summary ──────────────────────────────────────────────────────────────────
echo ""
echo "====================================================="
echo " Deployment Summary"
echo "====================================================="
echo " Deployer        : $DEPLOYER"
echo " Network         : $NETWORK"
echo " WASM Hash       : $WASM_HASH"
echo " Salt            : $SALT"
echo " Contract Address: $ACTUAL_ADDRESS"
echo " Address Match   : YES"
echo "====================================================="
echo ""
echo "Next: Initialize the contract:"
echo "  stellar contract invoke \\"
echo "    --id $ACTUAL_ADDRESS \\"
echo "    --source $DEPLOYER \\"
echo "    --network $NETWORK \\"
echo "    -- initialize \\"
echo "    --admin $ADMIN \\"
echo "    --min_sources_required 1 \\"
echo "    --max_history_length 100 \\"
echo "    --decimals $DECIMALS \\"
echo "    --description \"$DESCRIPTION\""
