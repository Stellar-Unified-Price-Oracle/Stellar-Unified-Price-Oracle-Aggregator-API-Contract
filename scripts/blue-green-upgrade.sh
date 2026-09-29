#!/usr/bin/env bash
# blue-green-upgrade.sh — Blue/green upgrade with verified migration and rollback (#415).
#
# Usage:
#   ./scripts/blue-green-upgrade.sh upgrade  --contract <ID> --admin <IDENTITY> --wasm <PATH> [--network testnet]
#   ./scripts/blue-green-upgrade.sh rollback --contract <ID> --admin <IDENTITY> --blue-hash <HASH> [--network testnet]
#
# Environment:
#   EXPECTED_SCHEMA   schema version the green build targets (checked by the sweep)
#   BATCH_SIZE        migration batch size (default 50)
#   ABORT_AFTER       testing only: stop after N migration batches to simulate an abort
#
# The blue WASM hash is recorded in .blue-green/<CONTRACT_ID>.blue before every
# upgrade. Rollback requires the multisig quorum documented in
# docs/blue-green-upgrade.md; this script only drives the admin-authorised
# `upgrade` call once that quorum has been collected off-chain.

set -euo pipefail

CMD="${1:?'upgrade | rollback required'}"; shift
NETWORK="testnet"; BATCH_SIZE="${BATCH_SIZE:-50}"
while [[ $# -gt 0 ]]; do
    case "$1" in
        --contract)  CONTRACT_ID="$2"; shift 2 ;;
        --admin)     ADMIN_IDENTITY="$2"; shift 2 ;;
        --wasm)      WASM="$2"; shift 2 ;;
        --blue-hash) BLUE_HASH="$2"; shift 2 ;;
        --network)   NETWORK="$2"; shift 2 ;;
        *) echo "Unknown argument: $1" >&2; exit 1 ;;
    esac
done
CONTRACT_ID="${CONTRACT_ID:?'--contract is required'}"
ADMIN_IDENTITY="${ADMIN_IDENTITY:?'--admin is required'}"
STATE_DIR=".blue-green"; mkdir -p "$STATE_DIR"
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

invoke() {
    stellar contract invoke --id "$CONTRACT_ID" --source "$ADMIN_IDENTITY" \
        --network "$NETWORK" -- "$@"
}

verify() {
    "$SCRIPT_DIR/verify-deployment.sh" --contract "$CONTRACT_ID" \
        --admin "$ADMIN_IDENTITY" --network "$NETWORK"
}

migrate() {
    # Idempotent and resumable: re-running continues from the on-chain cursor.
    local batches=0
    while [[ "$(invoke get_migration_state)" != "null" ]] || [[ $batches -eq 0 ]]; do
        invoke migrate_storage --batch_size "$BATCH_SIZE" > /dev/null
        batches=$((batches + 1))
        if [[ -n "${ABORT_AFTER:-}" && $batches -ge $ABORT_AFTER ]]; then
            echo "[ABORT] simulated abort after $batches batch(es); re-run to resume." >&2
            exit 2
        fi
    done
}

case "$CMD" in
    upgrade)
        WASM="${WASM:?'--wasm is required'}"
        echo "[1/5] Pre-upgrade verification (blue)"; verify
        echo "[2/5] Recording blue hash"
        stellar contract fetch --id "$CONTRACT_ID" --network "$NETWORK" -o "$STATE_DIR/blue.wasm"
        sha256sum "$STATE_DIR/blue.wasm" | cut -d' ' -f1 > "$STATE_DIR/$CONTRACT_ID.blue"
        echo "[3/5] Installing green WASM"
        GREEN_HASH=$(stellar contract upload --wasm "$WASM" --source "$ADMIN_IDENTITY" --network "$NETWORK")
        invoke upgrade --new_wasm_hash "$GREEN_HASH"
        echo "[4/5] Migrating storage"; migrate
        echo "[5/5] Post-upgrade invariant sweep (green)"
        if ! verify; then
            echo "[FAIL] invariant sweep failed; do NOT serve green. Follow docs/blue-green-upgrade.md." >&2
            exit 1
        fi
        echo "Upgrade complete. Blue hash: $(cat "$STATE_DIR/$CONTRACT_ID.blue")"
        ;;
    rollback)
        BLUE_HASH="${BLUE_HASH:-$(cat "$STATE_DIR/$CONTRACT_ID.blue")}"
        if [[ "$(invoke get_migration_state)" != "null" ]]; then
            echo "[FAIL] migration in progress: finish it (re-run upgrade) before rolling back." >&2
            exit 1
        fi
        invoke upgrade --new_wasm_hash "$BLUE_HASH"
        verify
        ;;
    *) echo "Unknown command: $CMD" >&2; exit 1 ;;
esac
