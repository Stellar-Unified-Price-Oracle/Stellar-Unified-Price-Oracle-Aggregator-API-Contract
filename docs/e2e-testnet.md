# End-to-End Testnet Tests

`scripts/e2e-testnet.sh` deploys the oracle contract to the Stellar **testnet**, runs a full lifecycle test, and verifies correctness.

## What It Tests

| Step | Action |
|------|--------|
| 1 | Create / reuse three testnet identities (admin, source-a, source-b) |
| 2 | Fund all identities via Friendbot |
| 3 | Deploy contract WASM to testnet |
| 4 | `initialize()` with `min_sources=2`, `decimals=7`, `max_history=100` |
| 5 | `add_source()` for both oracle sources |
| 6 | `register_asset()` for a test asset |
| 7 | `submit_price()` from source A and source B |
| 8 | `get_price()` — verify median aggregate is correct |
| 9 | `get_source_price()` — verify per-source submissions |
| 10 | `health_check()` — verify health report |
| 11 | SEP-40 `lastprice()` smoke-test |
| 12 | Cleanup: unregister asset and sources (optional) |

## Prerequisites

- **Stellar CLI** (`stellar`) — [install guide](https://developers.stellar.org/docs/tools/stellar-cli)
  - Also works with the older `soroban` CLI: `export STELLAR_CLI=soroban`
- **jq** — for parsing JSON responses (`apt install jq` or `brew install jq`)
- Built contract WASM:
  ```bash
  cargo build -p price-oracle --target wasm32v1-none --release
  ```
- Internet access to the Stellar testnet RPC and Friendbot

## Running

```bash
./scripts/e2e-testnet.sh
```

### Environment Variable Overrides

| Variable | Default | Description |
|----------|---------|-------------|
| `NETWORK` | `testnet` | Stellar network alias |
| `RPC_URL` | `https://soroban-testnet.stellar.org` | Soroban RPC endpoint |
| `NETWORK_PASSPHRASE` | `Test SDF Network ; September 2015` | Network passphrase |
| `ADMIN_ID` | `oracle-admin-e2e` | CLI identity name for admin |
| `SOURCE_A_ID` | `oracle-source-a-e2e` | CLI identity name for source A |
| `SOURCE_B_ID` | `oracle-source-b-e2e` | CLI identity name for source B |
| `STELLAR_CLI` | `stellar` | CLI binary name (`stellar` or `soroban`) |

Example with custom identities:

```bash
ADMIN_ID=my-admin SOURCE_A_ID=my-source-1 SOURCE_B_ID=my-source-2 \
  ./scripts/e2e-testnet.sh
```

## Expected Output

```
[e2e] Setting up identities...
[e2e]   Reusing existing identity: oracle-admin-e2e
...
[e2e] Contract deployed: C...
[e2e] Contract initialized.
[e2e] Sources registered.
[e2e] Source registration verified.
[e2e] Asset registration verified.
[e2e] Prices submitted.
[e2e] PASS: Aggregate price is correct (median = 10100000)
...
[e2e] ==========================================
[e2e]  E2E test PASSED
[e2e]  Contract ID: C...
[e2e]  Network:     testnet
[e2e] ==========================================
```

## Price Verification

The script submits:
- Source A: `10_000_000` (= 1.0000000 at 7 decimals)
- Source B: `10_200_000` (= 1.0200000 at 7 decimals)

Expected median aggregate: `10_100_000` (= 1.0100000).
