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
    submit-price)    cmd_submit_price    "$@" ;;
    get-price)       cmd_get_price       "$@" ;;
    add-source)      cmd_add_source      "$@" ;;
    register-asset)  cmd_register_asset  "$@" ;;
    health-check)    cmd_health_check    "$@" ;;
    init)            cmd_init            "$@" ;;
    -h|--help|help)  usage ;;
    *) die "Unknown command: $cmd. Run 'oracle-cli help' for usage." ;;
  esac
}

main "$@"
