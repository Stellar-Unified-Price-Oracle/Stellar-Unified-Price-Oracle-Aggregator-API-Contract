#!/usr/bin/env bash
# source-lifecycle.sh — Source onboarding / offboarding workflow (#402).
#
# Usage:
#   ./scripts/source-lifecycle.sh onboard   --contract <ID> --admin <IDENTITY> --source <ADDR> --source-key <IDENTITY> --name <NAME> --identity <HEX32>
#   ./scripts/source-lifecycle.sh status    --contract <ID> --admin <IDENTITY> --source <ADDR>
#   ./scripts/source-lifecycle.sh graduate  --contract <ID> --admin <IDENTITY> --source <ADDR>
#   ./scripts/source-lifecycle.sh offboard  --contract <ID> --admin <IDENTITY> --source <ADDR> [--evidence-asset <ADDR>]
#
# `onboard` runs the identity → bond steps and prints the checklist, which is
# the pass/fail evidence for each step. `graduate` fails on-chain unless the
# probation period has elapsed and the bond is posted. `offboard` is a single
# transaction: slashing (only with --evidence-asset, verified on-chain),
# revocation of derived state, tombstoning and recomputation. See
# docs/source-lifecycle.md.

set -euo pipefail

CMD="${1:?'onboard | status | graduate | offboard required'}"; shift
NETWORK="testnet"; EVIDENCE_ASSET=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --contract)       CONTRACT_ID="$2"; shift 2 ;;
        --admin)          ADMIN_IDENTITY="$2"; shift 2 ;;
        --source)         SOURCE="$2"; shift 2 ;;
        --source-key)     SOURCE_IDENTITY="$2"; shift 2 ;;
        --name)           NAME="$2"; shift 2 ;;
        --identity)       IDENTITY="$2"; shift 2 ;;
        --evidence-asset) EVIDENCE_ASSET="$2"; shift 2 ;;
        --network)        NETWORK="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done
CONTRACT_ID="${CONTRACT_ID:?'--contract is required'}"
ADMIN_IDENTITY="${ADMIN_IDENTITY:?'--admin is required'}"
SOURCE="${SOURCE:?'--source is required'}"

invoke() {
    local signer="$1"; shift
    stellar contract invoke --id "$CONTRACT_ID" --source "$signer" \
        --network "$NETWORK" -- "$@"
}

status() {
    invoke "$ADMIN_IDENTITY" get_onboarding_checklist --source "$SOURCE"
}

case "$CMD" in
    onboard)
        echo "[1/3] identity: binding $SOURCE to ${IDENTITY:?'--identity is required'}"
        invoke "$ADMIN_IDENTITY" onboard_source --source "$SOURCE" \
            --name "${NAME:?'--name is required'}" --identity "$IDENTITY"
        echo "[2/3] bond: depositing configured bond"
        invoke "${SOURCE_IDENTITY:?'--source-key is required'}" deposit_source_bond --source "$SOURCE"
        echo "[3/3] probation started; checklist:"
        status
        ;;
    status)
        status
        ;;
    graduate)
        invoke "$ADMIN_IDENTITY" graduate_source --source "$SOURCE"
        status
        ;;
    offboard)
        if [[ -n "$EVIDENCE_ASSET" ]]; then
            invoke "$ADMIN_IDENTITY" offboard_source --source "$SOURCE" --evidence_asset "$EVIDENCE_ASSET"
        else
            invoke "$ADMIN_IDENTITY" offboard_source --source "$SOURCE"
        fi
        ;;
    *) echo "Unknown command: $CMD" >&2; exit 1 ;;
esac
