#!/usr/bin/env bash
# generate-abi.sh — Regenerate the machine-readable contract ABI from the
# built WASM's on-chain spec (source of truth: contracts/price-oracle/src).
#
# Usage: ./scripts/generate-abi.sh
#
# Outputs:
#   docs/abi.json — machine-readable Soroban contract spec (function
#                   signatures, parameter types, struct/enum definitions)
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_PATH="$ROOT_DIR/target/wasm32v1-none/release/price_oracle.wasm"
OUT_PATH="$ROOT_DIR/docs/abi.json"

if [[ ! -f "$WASM_PATH" ]]; then
    echo "Building contract wasm..."
    cargo build -p price-oracle --target wasm32v1-none --release --manifest-path "$ROOT_DIR/Cargo.toml"
fi

echo "Extracting ABI from $WASM_PATH ..."
stellar contract inspect --wasm "$WASM_PATH" --output json > "$OUT_PATH"

echo "ABI written to $OUT_PATH"
