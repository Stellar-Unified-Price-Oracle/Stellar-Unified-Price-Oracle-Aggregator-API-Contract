#!/usr/bin/env bash
# oracle-cli — Soroban oracle operator CLI plugin
# Wraps stellar contract invoke with oracle-specific ergonomics.
#
# Usage: oracle-cli <command> [options]
#
# Prerequisites:
#   - stellar CLI (https://developers.stellar.org/docs/tools/developer-tools)
#   - ORACLE_CONTRACT_ID  env var  OR  --contract flag
#   - ORACLE_NETWORK      env var  OR  --network flag  (default: testnet)
#   - ORACLE_SOURCE_KEY   env var  OR  --source-key flag (identity name for submit-price)

set -euo pipefail

# ── defaults ─────────────────────────────────────────────────────────────────
NETWORK="${ORACLE_NETWORK:-testnet}"
CONTRACT_ID="${ORACLE_CONTRACT_ID:-}"
SOURCE_KEY="${ORACLE_SOURCE_KEY:-}"

# ── helpers ───────────────────────────────────────────────────────────────────
die()  { echo "ERROR: $*" >&2; exit 1; }
info() { echo "==> $*"; }

need_contract() {
  [[ -n "$CONTRACT_ID" ]] || die "Set ORACLE_CONTRACT_ID or pass --contract <id>"
}

invoke() {
  # invoke <function> [extra stellar args...]
  local fn="$1"; shift
  stellar contract invoke \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    --function "$fn" \
    "$@"
}

# ── option parsing ─────────────────────────────────────────────────────────────
parse_flags() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --contract)  CONTRACT_ID="$2"; shift 2 ;;
      --network)   NETWORK="$2";     shift 2 ;;
      --source-key) SOURCE_KEY="$2"; shift 2 ;;
      *)           echo "$1"; shift ;;          # pass through unrecognised flags
    esac
  done
}

# ── commands ──────────────────────────────────────────────────────────────────

cmd_submit_price() {
  # oracle-cli submit-price --source <address> --asset <address> --price <i128> [--timestamp <u64>]
  local source="" asset="" price="" timestamp=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --source)    source="$2";    shift 2 ;;
      --asset)     asset="$2";     shift 2 ;;
      --price)     price="$2";     shift 2 ;;
      --timestamp) timestamp="$2"; shift 2 ;;
      --contract)  CONTRACT_ID="$2"; shift 2 ;;
      --network)   NETWORK="$2";   shift 2 ;;
      --source-key) SOURCE_KEY="$2"; shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$source" ]]  || die "--source is required"
  [[ -n "$asset" ]]   || die "--asset is required"
  [[ -n "$price" ]]   || die "--price is required"
  [[ -n "$SOURCE_KEY" ]] || die "Set ORACLE_SOURCE_KEY or pass --source-key <identity>"
  need_contract

  if [[ -z "$timestamp" ]]; then
    timestamp=$(date +%s)
  fi

  info "Submitting price $price for asset $asset from source $source"
  invoke submit_price \
    --source "$source" \
    --asset "$asset" \
    --price "$price" \
    --timestamp "$timestamp" \
    --source-account "$SOURCE_KEY"
}

cmd_get_price() {
  # oracle-cli get-price --asset <address>
  local asset=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --asset)    asset="$2";    shift 2 ;;
      --contract) CONTRACT_ID="$2"; shift 2 ;;
      --network)  NETWORK="$2";  shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$asset" ]] || die "--asset is required"
  need_contract

  info "Fetching price for asset $asset"
  invoke get_price --asset "$asset"
}

cmd_add_source() {
  # oracle-cli add-source --address <address> --name <string> --admin-key <identity>
  local address="" name="" admin_key=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --address)   address="$2";   shift 2 ;;
      --name)      name="$2";      shift 2 ;;
      --admin-key) admin_key="$2"; shift 2 ;;
      --contract)  CONTRACT_ID="$2"; shift 2 ;;
      --network)   NETWORK="$2";   shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$address" ]]   || die "--address is required"
  [[ -n "$name" ]]      || die "--name is required"
  [[ -n "$admin_key" ]] || die "--admin-key is required"
  need_contract

  info "Adding source $name ($address)"
  invoke add_source \
    --source "$address" \
    --name "$name" \
    --source-account "$admin_key"
}

cmd_register_asset() {
  # oracle-cli register-asset --asset <address> --admin-key <identity>
  local asset="" admin_key=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --asset)     asset="$2";     shift 2 ;;
      --admin-key) admin_key="$2"; shift 2 ;;
      --contract)  CONTRACT_ID="$2"; shift 2 ;;
      --network)   NETWORK="$2";   shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$asset" ]]     || die "--asset is required"
  [[ -n "$admin_key" ]] || die "--admin-key is required"
  need_contract

  info "Registering asset $asset"
  invoke register_asset \
    --asset "$asset" \
    --source-account "$admin_key"
}

cmd_health_check() {
  # oracle-cli health-check
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --contract) CONTRACT_ID="$2"; shift 2 ;;
      --network)  NETWORK="$2";     shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  need_contract

  info "Running health check on $CONTRACT_ID (network: $NETWORK)"

  local admin min_sources decimals description
  admin=$(invoke get_admin_address 2>/dev/null)       || { echo "  admin:       [error]"; }
  min_sources=$(invoke get_min_sources_required 2>/dev/null) || { echo "  min_sources: [error]"; }
  decimals=$(invoke get_decimals 2>/dev/null)         || { echo "  decimals:    [error]"; }
  description=$(invoke get_description 2>/dev/null)   || { echo "  description: [error]"; }

  echo "  contract:    $CONTRACT_ID"
  echo "  network:     $NETWORK"
  echo "  admin:       $admin"
  echo "  min_sources: $min_sources"
  echo "  decimals:    $decimals"
  echo "  description: $description"
  info "Health check complete"
}

cmd_oracle_state_dump() {
  # oracle-cli oracle-state-dump [--format json|text]
  local format="text"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --format)  format="$2";    shift 2 ;;
      --contract) CONTRACT_ID="$2"; shift 2 ;;
      --network)  NETWORK="$2";     shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  need_contract

  info "Dumping oracle state (format: $format)"

  local admin description min_sources max_history decimals resolution timestamp_threshold max_deviation heartbeat_interval
  admin=$(invoke get_admin_address 2>/dev/null) || die "Failed to read admin"
  description=$(invoke get_description 2>/dev/null) || die "Failed to read description"
  min_sources=$(invoke get_min_sources_required 2>/dev/null) || die "Failed to read min_sources_required"
  max_history=$(invoke get_max_history_length 2>/dev/null) || die "Failed to read max_history_length"
  decimals=$(invoke get_decimals 2>/dev/null) || die "Failed to read decimals"
  resolution=$(invoke get_resolution 2>/dev/null) || echo "0"
  timestamp_threshold=$(invoke get_timestamp_threshold 2>/dev/null) || echo "0"
  max_deviation=$(invoke get_max_price_deviation 2>/dev/null) || echo "0"
  heartbeat_interval=$(invoke get_heartbeat_interval 2>/dev/null) || echo "0"

  if [[ "$format" == "json" ]]; then
    printf '{"admin":"%s","description":"%s","min_sources":%s,"max_history":%s,"decimals":%s,"resolution":%s,"timestamp_threshold":%s,"max_deviation_bps":%s,"heartbeat_interval":%s}\n' \
      "$admin" "$description" "$min_sources" "$max_history" "$decimals" "$resolution" "$timestamp_threshold" "$max_deviation" "$heartbeat_interval"
  else
    echo "admin:            $admin"
    echo "description:      $description"
    echo "min_sources:      $min_sources"
    echo "max_history:      $max_history"
    echo "decimals:         $decimals"
    echo "resolution:       $resolution"
    echo "timestamp_threshold: $timestamp_threshold"
    echo "max_deviation_bps: $max_deviation"
    echo "heartbeat_interval: $heartbeat_interval"
  fi
}

cmd_oracle_state_analyze() {
  # oracle-cli oracle-state-analyze [--asset <address>]
  local asset=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --asset)     asset="$2";     shift 2 ;;
      --contract)  CONTRACT_ID="$2"; shift 2 ;;
      --network)   NETWORK="$2";   shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  need_contract

  info "Analyzing oracle state"

  local admin min_sources max_history decimals
  admin=$(invoke get_admin_address 2>/dev/null) || die "Failed to read admin"
  min_sources=$(invoke get_min_sources_required 2>/dev/null) || die "Failed to read min_sources_required"
  max_history=$(invoke get_max_history_length 2>/dev/null) || die "Failed to read max_history_length"
  decimals=$(invoke get_decimals 2>/dev/null) || die "Failed to read decimals"

  echo "contract:      $CONTRACT_ID"
  echo "admin:         $admin"
  echo "min_sources:   $min_sources"
  echo "max_history:   $max_history"
  echo "decimals:      $decimals"

  if [[ -n "$asset" ]]; then
    local agg price timestamp num_sources
    agg=$(invoke get_price --asset "$asset" 2>/dev/null) || { echo "asset_price: [error]"; return 0; }
    echo "asset:         $asset"
    echo "aggregate:     $agg"
  fi
}

cmd_oracle_state_diff() {
  # oracle-cli oracle-state-diff --contract <id_a> --contract <id_b> [--format json|text]
  local contract_a="" contract_b="" format="text"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --contract)  contract_a="$2"; shift 2 ;;
      --contract-b) contract_b="$2"; shift 2 ;;
      --format)    format="$2";    shift 2 ;;
      --network)   NETWORK="$2";   shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$contract_a" ]] || die "--contract <id_a> is required"
  [[ -n "$contract_b" ]] || die "--contract-b <id_b> is required"

  info "Diffing oracle state between $contract_a and $contract_b"

  local a_admin b_admin a_desc b_desc a_min b_min
  a_admin=$(stellar contract invoke --id "$contract_a" --network "$NETWORK" --function get_admin_address 2>/dev/null) || die "Failed to read admin from $contract_a"
  b_admin=$(stellar contract invoke --id "$contract_b" --network "$NETWORK" --function get_admin_address 2>/dev/null) || die "Failed to read admin from $contract_b"
  a_desc=$(stellar contract invoke --id "$contract_a" --network "$NETWORK" --function get_description 2>/dev/null) || die "Failed to read description from $contract_a"
  b_desc=$(stellar contract invoke --id "$contract_b" --network "$NETWORK" --function get_description 2>/dev/null) || die "Failed to read description from $contract_b"
  a_min=$(stellar contract invoke --id "$contract_a" --network "$NETWORK" --function get_min_sources_required 2>/dev/null) || die "Failed to read min_sources from $contract_a"
  b_min=$(stellar contract invoke --id "$contract_b" --network "$NETWORK" --function get_min_sources_required 2>/dev/null) || die "Failed to read min_sources from $contract_b"

  if [[ "$format" == "json" ]]; then
    printf '{"a_admin":"%s","b_admin":"%s","a_description":"%s","b_description":"%s","a_min_sources":%s,"b_min_sources":%s}\n' \
      "$a_admin" "$b_admin" "$a_desc" "$b_desc" "$a_min" "$b_min"
  else
    echo "contract_a: $contract_a"
    echo "contract_b: $contract_b"
    echo "admin_a:    $a_admin"
    echo "admin_b:    $b_admin"
    echo "desc_a:     $a_desc"
    echo "desc_b:     $b_desc"
    echo "min_sources_a: $a_min"
    echo "min_sources_b: $b_min"
  fi
}

cmd_soroswap_price() {
  # oracle-cli soroswap-price --asset <address> [--pool <address>]
  local asset="" pool=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --asset)    asset="$2";    shift 2 ;;
      --pool)     pool="$2";     shift 2 ;;
      --contract) CONTRACT_ID="$2"; shift 2 ;;
      --network)  NETWORK="$2";   shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$asset" ]] || die "--asset is required"
  need_contract

  info "Fetching Soroswap pool price for asset $asset"
  invoke get_soroswap_price --asset "$asset" ${pool:+--pool "$pool"}
}

cmd_dex_price() {
  # oracle-cli dex-price --asset <address> [--pair <address>]
  local asset="" pair=""
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --asset)    asset="$2";    shift 2 ;;
      --pair)     pair="$2";     shift 2 ;;
      --contract) CONTRACT_ID="$2"; shift 2 ;;
      --network)  NETWORK="$2";   shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$asset" ]] || die "--asset is required"
  need_contract

  info "Fetching Stellar DEX price for asset $asset"
  invoke get_dex_price --asset "$asset" ${pair:+--pair "$pair"}
}

cmd_init() {
  # oracle-cli init --admin <address> --admin-key <identity> [--min-sources 1] [--max-history 100] [--decimals 18] [--description "..."]
  local admin="" admin_key="" min_sources="1" max_history="100" decimals="18" description="Stellar Price Oracle Aggregator"
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --admin)       admin="$2";       shift 2 ;;
      --admin-key)   admin_key="$2";   shift 2 ;;
      --min-sources) min_sources="$2"; shift 2 ;;
      --max-history) max_history="$2"; shift 2 ;;
      --decimals)    decimals="$2";    shift 2 ;;
      --description) description="$2"; shift 2 ;;
      --contract)    CONTRACT_ID="$2"; shift 2 ;;
      --network)     NETWORK="$2";     shift 2 ;;
      *) die "Unknown flag: $1" ;;
    esac
  done
  [[ -n "$admin" ]]     || die "--admin is required"
  [[ -n "$admin_key" ]] || die "--admin-key is required"
  need_contract

  info "Initializing contract $CONTRACT_ID"
  invoke initialize \
    --admin "$admin" \
    --min_sources_required "$min_sources" \
    --max_history_length "$max_history" \
    --decimals "$decimals" \
    --description "$description" \
    --source-account "$admin_key"
  info "Contract initialized"
}

# ── usage ─────────────────────────────────────────────────────────────────────
usage() {
  cat <<'EOF'
oracle-cli — Stellar Price Oracle operator CLI

USAGE:
  oracle-cli <command> [options]

ENVIRONMENT:
  ORACLE_CONTRACT_ID   Contract address (override with --contract)
  ORACLE_NETWORK       Network name, default: testnet (override with --network)
  ORACLE_SOURCE_KEY    Stellar identity for price submissions (override with --source-key)

COMMANDS:
  init              Initialize the oracle contract
  submit-price      Submit a price from a registered source
  get-price         Get the latest aggregated price for an asset
  add-source        Register a new oracle source (admin)
  register-asset    Register a new asset (admin)
  health-check      Display contract configuration and status
  oracle-state-dump Dump contract state (json|text)
  oracle-state-analyze Analyze state statistics (avg sources, history depth, TTL health)
  oracle-state-diff Compare two contract states
  soroswap-price    Get Soroswap pool price for an asset
  dex-price         Get Stellar DEX price for an asset

EXAMPLES:
  export ORACLE_CONTRACT_ID=CAAAA...
  oracle-cli health-check

  oracle-cli init \
    --admin GAAA... --admin-key my-admin-identity \
    --description "My Oracle" --decimals 18

  oracle-cli add-source \
    --address GBBB... --name "Chainlink" --admin-key my-admin-identity

  oracle-cli register-asset \
    --asset GCCC... --admin-key my-admin-identity

  oracle-cli submit-price \
    --source GBBB... --asset GCCC... --price 50000000000000000000 \
    --source-key my-source-identity

  oracle-cli get-price --asset GCCC...
EOF
}

# ── dispatch ──────────────────────────────────────────────────────────────────
main() {
  local cmd="${1:-}"
  [[ -n "$cmd" ]] || { usage; exit 0; }
  shift

  case "$cmd" in
    submit-price)         cmd_submit_price         "$@" ;;
    get-price)            cmd_get_price            "$@" ;;
    add-source)           cmd_add_source           "$@" ;;
    register-asset)       cmd_register_asset       "$@" ;;
    health-check)         cmd_health_check         "$@" ;;
    init)                 cmd_init                 "$@" ;;
    oracle-state-dump)    cmd_oracle_state_dump    "$@" ;;
    oracle-state-analyze) cmd_oracle_state_analyze "$@" ;;
    oracle-state-diff)    cmd_oracle_state_diff    "$@" ;;
    soroswap-price)       cmd_soroswap_price       "$@" ;;
    dex-price)            cmd_dex_price            "$@" ;;
    -h|--help|help)       usage ;;
    *) die "Unknown command: $cmd. Run 'oracle-cli help' for usage." ;;
  esac
}

main "$@"
