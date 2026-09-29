#!/usr/bin/env bash
# generate-abi.sh — Regenerate the machine-readable contract ABI from the
# built WASM's on-chain spec (source of truth: contracts/price-oracle/src).
#
# Usage: ./scripts/generate-abi.sh
#
# Outputs:
#   docs/abi.json — machine-readable Soroban contract spec (function
#                   signatures, parameter types, struct/enum definitions)
#
# Note: `stellar contract inspect` only accepts xdr-base64/xdr-base64-array/
# docs and is deprecated in favour of `stellar contract info interface`, whose
# `--output json-formatted` is what emits the JSON spec.
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WASM_PATH="$ROOT_DIR/target/wasm32v1-none/release/price_oracle.wasm"
OUT_PATH="$ROOT_DIR/docs/abi.json"

if [[ ! -f "$WASM_PATH" ]]; then
    echo "Building contract wasm..."
    cargo build -p price-oracle --target wasm32v1-none --release --manifest-path "$ROOT_DIR/Cargo.toml"
fi

echo "Extracting ABI from $WASM_PATH ..."
stellar contract info interface --wasm "$WASM_PATH" --output json-formatted > "$OUT_PATH"

echo "ABI written to $OUT_PATH"
