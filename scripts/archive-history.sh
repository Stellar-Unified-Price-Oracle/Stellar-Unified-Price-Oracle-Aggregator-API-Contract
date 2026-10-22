#!/usr/bin/env bash
# =============================================================================
# archive-history.sh — Off-chain history archiving tool
#
# Exports paginated price-history snapshots for a given asset from the
# Stellar Unified Price Oracle contract and writes them to a local JSONL file.
#
# Usage:
#   ./scripts/archive-history.sh [OPTIONS]
#
# Required environment:
#   ORACLE_CONTRACT_ID  — deployed contract address
#
# Options:
#   --asset        ADDR   Asset contract address to export (required)
#   --from-ledger  N      Starting ledger (default: 0)
#   --limit        N      Page size, max 200 (default: 100)
#   --output       FILE   Output file path (default: history-<asset>-<ts>.jsonl)
#   --network      NAME   Stellar network alias (default: testnet)
#   --source-key   NAME   Stellar CLI source identity (default: default)
#   --verify              Verify each page hash before writing
#   --help                Show this message
#
# Output format: one JSON object per line (JSONL), e.g.
#   {"asset":"G...","ledger":101,"price":"5000","timestamp":1000001,...}
#
# The script also writes a companion .manifest.json with:
#   - asset, from_ledger, to_ledger, total_entries, pages, first_hash, last_hash
# =============================================================================

set -euo pipefail

# ---------------------------------------------------------------------------
# Defaults
# ---------------------------------------------------------------------------
NETWORK="${ORACLE_NETWORK:-testnet}"
SOURCE_KEY="${ORACLE_SOURCE_KEY:-default}"
CONTRACT_ID="${ORACLE_CONTRACT_ID:-}"
ASSET=""
FROM_LEDGER=0
PAGE_LIMIT=100
OUTPUT_FILE=""
VERIFY=false

# ---------------------------------------------------------------------------
# Argument parsing
# ---------------------------------------------------------------------------
while [[ $# -gt 0 ]]; do
  case "$1" in
    --asset)        ASSET="$2";        shift 2 ;;
    --from-ledger)  FROM_LEDGER="$2";  shift 2 ;;
    --limit)        PAGE_LIMIT="$2";   shift 2 ;;
    --output)       OUTPUT_FILE="$2";  shift 2 ;;
    --network)      NETWORK="$2";      shift 2 ;;
    --source-key)   SOURCE_KEY="$2";   shift 2 ;;
    --verify)       VERIFY=true;       shift ;;
    --help)
      sed -n '3,40p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      exit 1
      ;;
  esac
done

# ---------------------------------------------------------------------------
# Validation
# ---------------------------------------------------------------------------
if [[ -z "$CONTRACT_ID" ]]; then
  echo "ERROR: ORACLE_CONTRACT_ID is not set." >&2
  exit 1
fi
if [[ -z "$ASSET" ]]; then
  echo "ERROR: --asset is required." >&2
  exit 1
fi
if ! command -v stellar &>/dev/null; then
  echo "ERROR: 'stellar' CLI not found. Install it from https://developers.stellar.org/docs/tools/developer-tools/cli/install-cli" >&2
  exit 1
fi
if ! command -v jq &>/dev/null; then
  echo "ERROR: 'jq' is required. Install it with your system package manager." >&2
  exit 1
fi

# ---------------------------------------------------------------------------
# Setup output
# ---------------------------------------------------------------------------
TIMESTAMP=$(date -u +"%Y%m%dT%H%M%SZ")
if [[ -z "$OUTPUT_FILE" ]]; then
  ASSET_SHORT="${ASSET:0:8}"
  OUTPUT_FILE="history-${ASSET_SHORT}-${TIMESTAMP}.jsonl"
fi
MANIFEST_FILE="${OUTPUT_FILE%.jsonl}.manifest.json"

echo "==> Archiving history for asset ${ASSET}"
echo "    Contract : $CONTRACT_ID"
echo "    Network  : $NETWORK"
echo "    From     : ledger $FROM_LEDGER"
echo "    Page size: $PAGE_LIMIT"
echo "    Output   : $OUTPUT_FILE"
[[ "$VERIFY" == "true" ]] && echo "    Verify   : enabled"
echo ""

# Truncate / create output file
: > "$OUTPUT_FILE"

# ---------------------------------------------------------------------------
# Helper: invoke a read-only contract function
# ---------------------------------------------------------------------------
invoke_readonly() {
  local fn="$1"
  shift
  stellar contract invoke \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    --source "$SOURCE_KEY" \
    -- "$fn" "$@"
}

# ---------------------------------------------------------------------------
# Pagination loop
# ---------------------------------------------------------------------------
CURSOR=$FROM_LEDGER
PAGE=0
TOTAL_ENTRIES=0
FIRST_HASH=""
LAST_HASH=""

while true; do
  PAGE=$((PAGE + 1))
  echo "  Page $PAGE (cursor=$CURSOR) ..."

  # Call export_history
  RESPONSE=$(invoke_readonly export_history \
    --asset "$ASSET" \
    --from-ledger "$CURSOR" \
    --limit "$PAGE_LIMIT")

  # Parse fields using jq
  ENTRY_COUNT=$(echo "$RESPONSE" | jq '.entries | length')
  DATA_HASH=$(echo "$RESPONSE"   | jq -r '.data_hash')
  FROM_L=$(echo "$RESPONSE"      | jq -r '.from_ledger')
  TO_L=$(echo "$RESPONSE"        | jq -r '.to_ledger')
  NEXT_CURSOR=$(echo "$RESPONSE" | jq -r '.next_cursor')

  if [[ "$ENTRY_COUNT" -eq 0 ]]; then
    echo "  No entries on this page. Done."
    break
  fi

  # Optional verification
  if [[ "$VERIFY" == "true" && "$ENTRY_COUNT" -gt 0 ]]; then
    echo "  Verifying hash for ledger range [$FROM_L, $TO_L] ..."
    VERIFY_RESULT=$(invoke_readonly verify_export \
      --asset "$ASSET" \
      --from-ledger "$FROM_L" \
      --to-ledger "$TO_L" \
      --expected-data-hash "$DATA_HASH")
    if [[ "$VERIFY_RESULT" != "true" ]]; then
      echo "  ERROR: Hash verification failed for page $PAGE (ledgers $FROM_L-$TO_L)!" >&2
      exit 2
    fi
    echo "  Hash verified OK."
  fi

  # Write entries to JSONL
  echo "$RESPONSE" | jq -c '.entries[]' >> "$OUTPUT_FILE"

  TOTAL_ENTRIES=$((TOTAL_ENTRIES + ENTRY_COUNT))
  [[ -z "$FIRST_HASH" ]] && FIRST_HASH="$DATA_HASH"
  LAST_HASH="$DATA_HASH"

  echo "  Wrote $ENTRY_COUNT entries (ledgers $FROM_L-$TO_L), hash=$DATA_HASH"

  # Stop if no more pages
  if [[ "$NEXT_CURSOR" == "0" || "$NEXT_CURSOR" == "null" ]]; then
    break
  fi
  CURSOR="$NEXT_CURSOR"
done

# ---------------------------------------------------------------------------
# Write manifest
# ---------------------------------------------------------------------------
jq -n \
  --arg asset        "$ASSET" \
  --arg contract     "$CONTRACT_ID" \
  --arg network      "$NETWORK" \
  --argjson from     "$FROM_LEDGER" \
  --argjson pages    "$PAGE" \
  --argjson total    "$TOTAL_ENTRIES" \
  --arg first_hash   "$FIRST_HASH" \
  --arg last_hash    "$LAST_HASH" \
  --arg archived_at  "$TIMESTAMP" \
  '{
    asset:        $asset,
    contract:     $contract,
    network:      $network,
    from_ledger:  $from,
    total_entries: $total,
    pages:        $pages,
    first_page_hash: $first_hash,
    last_page_hash:  $last_hash,
    archived_at:  $archived_at
  }' > "$MANIFEST_FILE"

echo ""
echo "==> Archive complete."
echo "    Total entries : $TOTAL_ENTRIES"
echo "    Pages fetched : $PAGE"
echo "    Data file     : $OUTPUT_FILE"
echo "    Manifest      : $MANIFEST_FILE"
