# Contract ABI

The Price Oracle contract's ABI (function signatures, parameter types, and
struct/enum definitions) is generated directly from the contract's compiled
WASM, so it can never drift from the deployed interface.

## Machine-readable ABI

Run:

```bash
./scripts/generate-abi.sh
```

This builds the contract (if needed) and writes the full Soroban contract
spec to [`docs/abi.json`](./abi.json) via
`stellar contract info interface --output json-formatted`. The
JSON includes every exported function with its parameter and return types,
plus all `#[contracttype]` structs and enums used in the public interface.

## Human-readable reference

* **Function reference** — see `docs/abi.json` for the authoritative,
  up-to-date list of all exported functions. Key entry points:
  * SEP-40 interface: `base`, `decimals`, `resolution`, `assets`,
    `lastprice`, `price`, `prices`
  * Price submission: `submit_price`, `submit_prices`,
    `submit_price_with_volume`, `submit_price_with_proof`
  * Price queries: `get_price`, `get_price_with_confidence`,
    `get_source_price`, `get_historical_price`
  * Source/asset management: `add_source`, `remove_source`,
    `register_asset`, `unregister_asset`
* **Error codes** — see [`docs/error-codes.md`](./error-codes.md) for the
  full error code registry (all `#[contracterror]` variants with
  descriptions).

## Keeping the ABI current

The `abi-check` job in `.github/workflows/ci.yml` regenerates the ABI from
the built WASM on every push/PR and publishes it as a build artifact, so
consumers always have access to an ABI generated from the exact commit
being tested. Run `./scripts/generate-abi.sh` locally to produce the same
file for local integration work.
