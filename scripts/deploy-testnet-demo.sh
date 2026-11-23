#!/usr/bin/env bash
set -euo pipefail

CLI="${STELLAR_CLI:-stellar}"
WASM_PATH="${WASM_PATH:-target/wasm32v1-none/release/price_oracle.wasm}"

required() {
  local name="$1" value="$2"
  [[ -n "$value" ]] || { echo "ERROR: $name is required." >&2; exit 1; }
}

required TESTNET_ADMIN_SECRET "${TESTNET_ADMIN_SECRET:-}"
required TESTNET_SOURCE_1_SECRET "${TESTNET_SOURCE_1_SECRET:-}"
required TESTNET_SOURCE_2_SECRET "${TESTNET_SOURCE_2_SECRET:-}"
required TESTNET_SOURCE_3_SECRET "${TESTNET_SOURCE_3_SECRET:-}"
required ORACLE_TESTNET_ASSETS "${ORACLE_TESTNET_ASSETS:-}"
[[ -f "$WASM_PATH" ]] || { echo "ERROR: missing WASM at $WASM_PATH." >&2; exit 1; }
command -v jq >/dev/null || { echo "ERROR: jq is required." >&2; exit 1; }

jq -e '
  type == "array" and length >= 4
  and ([.[].symbol] | index("XLM") != null)
  and ([.[].symbol] | index("USDC") != null)
  and ([.[].symbol] | index("BTC") != null)
  and ([.[].symbol] | index("ETH") != null)
  and all(.[]; (.address | type == "string" and length > 0))
' <<<"$ORACLE_TESTNET_ASSETS" >/dev/null || {
  echo "ERROR: ORACLE_TESTNET_ASSETS must define addresses for XLM, USDC, BTC, and ETH." >&2
  exit 1
}

add_identity() {
  local name="$1" secret="$2"
  if ! "$CLI" keys show "$name" >/dev/null 2>&1; then
    "$CLI" keys add "$name" --secret-key "$secret" >/dev/null
  fi
  local address
  address=$("$CLI" keys address "$name")
  curl -fsS "https://friendbot.stellar.org/?addr=${address}" >/dev/null \
    || echo "Friendbot did not fund $name; deployment will fail if the account is unfunded." >&2
}

add_identity oracle-demo-admin "$TESTNET_ADMIN_SECRET"
add_identity oracle-demo-source-1 "$TESTNET_SOURCE_1_SECRET"
add_identity oracle-demo-source-2 "$TESTNET_SOURCE_2_SECRET"
add_identity oracle-demo-source-3 "$TESTNET_SOURCE_3_SECRET"

ADMIN_ADDRESS=$("$CLI" keys address oracle-demo-admin)
SOURCE_1=$("$CLI" keys address oracle-demo-source-1)
SOURCE_2=$("$CLI" keys address oracle-demo-source-2)
SOURCE_3=$("$CLI" keys address oracle-demo-source-3)
if [[ $(printf '%s\n' "$ADMIN_ADDRESS" "$SOURCE_1" "$SOURCE_2" "$SOURCE_3" | sort -u | wc -l) -ne 4 ]]; then
  echo "ERROR: the admin and three source secrets must identify four distinct accounts." >&2
  exit 1
fi

WASM_HASH=$("$CLI" contract upload --wasm "$WASM_PATH" \
  --source oracle-demo-admin --network testnet)
CONTRACT_ID=$("$CLI" contract deploy --wasm-hash "$WASM_HASH" \
  --source oracle-demo-admin --network testnet)
"$CLI" contract invoke --id "$CONTRACT_ID" --source oracle-demo-admin --network testnet -- \
  initialize --admin "$ADMIN_ADDRESS" --min_sources_required 3 \
  --max_history_length 100 --decimals 7 --description '"Public testnet demo"'

"$CLI" contract invoke --id "$CONTRACT_ID" --source oracle-demo-admin --network testnet -- \
  add_source --source "$SOURCE_1" --name '"demo-source-1"'
"$CLI" contract invoke --id "$CONTRACT_ID" --source oracle-demo-admin --network testnet -- \
  add_source --source "$SOURCE_2" --name '"demo-source-2"'
"$CLI" contract invoke --id "$CONTRACT_ID" --source oracle-demo-admin --network testnet -- \
  add_source --source "$SOURCE_3" --name '"demo-source-3"'

while IFS= read -r asset; do
  address=$(jq -r '.address' <<<"$asset")
  "$CLI" contract invoke --id "$CONTRACT_ID" --source oracle-demo-admin --network testnet -- \
    register_asset --asset "$address"
done < <(jq -c '.[]' <<<"$ORACLE_TESTNET_ASSETS")

printf 'contract_id=%s\n' "$CONTRACT_ID" >>"${GITHUB_OUTPUT:-/dev/null}"
echo "Deployed testnet contract: $CONTRACT_ID"
