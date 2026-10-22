#!/usr/bin/env bash
# Canary snapshot + decision for the oracle (#414).
#
# Reads the aggregate for every asset in ASSETS_FILE from both the live (blue)
# and candidate (green) contracts at the same ledger window, then runs the
# decision engine in services/canary/pipeline.py. Exits non-zero (rollback)
# on any per-asset divergence.
#
# Usage:
#   scripts/canary.sh <network> <source-identity> <blue-id> <green-id> <assets-file> <out-dir>
set -euo pipefail

NETWORK="$1" IDENTITY="$2" BLUE_ID="$3" GREEN_ID="$4" ASSETS_FILE="$5" OUT_DIR="$6"
CANARY_BPS="${CANARY_BPS:-1000}"
SALT="${CANARY_SALT:-$GREEN_ID}"
mkdir -p "$OUT_DIR"

snapshot() {
  local id="$1" out="$2" first=1
  echo "{" > "$out"
  while read -r asset; do
    [[ -z "$asset" || "$asset" == \#* ]] && continue
    local price
    price=$(stellar contract invoke --id "$id" --source "$IDENTITY" --network "$NETWORK" \
      --send=no -- get_price --asset "$asset" --max_age 0 2>/dev/null || echo null)
    [[ $first -eq 1 ]] || echo "," >> "$out"
    first=0
    if [[ "$price" == "null" || -z "$price" ]]; then
      printf '"%s": {"price": 0, "decimals": null, "num_sources": 0}' "$asset" >> "$out"
    else
      printf '"%s": %s' "$asset" "$price" >> "$out"
    fi
  done < "$ASSETS_FILE"
  echo "}" >> "$out"
}

snapshot "$BLUE_ID" "$OUT_DIR/blue.json"
snapshot "$GREEN_ID" "$OUT_DIR/green.json"

python3 -m services.canary.pipeline \
  --blue-snapshot "$OUT_DIR/blue.json" \
  --green-snapshot "$OUT_DIR/green.json" \
  --blue-contract "$BLUE_ID" --green-contract "$GREEN_ID" \
  --canary-bps "$CANARY_BPS" --salt "$SALT" \
  --out "$OUT_DIR/deployment-record.json"
