# Stellar Unified Price Oracle

[![CI](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/actions/workflows/ci.yml/badge.svg)](https://github.com/Stellar-Unified-Price-Oracle/Stellar-Unified-Price-Oracle-Aggregator-API-Contract/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

A Soroban smart contract that collects prices from multiple permissioned oracle
sources, aggregates them by **median**, and serves current and historical prices
to other Stellar contracts through the standard **SEP-40** oracle interface.

DeFi protocols on Stellar (lending, perps, stablecoins) need a price they can
trust. A single feed is a single point of failure; this contract combines
several independent sources so that one bad or compromised feed cannot move
the price.

> **Security notice (November 2026):** two pull requests merged on 2026-10-22
> contained a third-party malware payload (the "PolinRider" campaign). It has
> been removed and CI now blocks it. If you cloned this repo between
> 2026-10-22 and the cleanup, read
> [docs/security/malware-incident-2026-10.md](docs/security/malware-incident-2026-10.md).

## Features

- **Median aggregation** across N permissioned sources, with a configurable
  minimum quorum (`min_sources_required`).
- **SEP-40 consumer interface**: `lastprice`, `price`, `prices`, `assets`,
  `decimals`, `resolution`, `base`.
- **Price history** per asset with ranged and cursor-paginated reads and
  optional gap interpolation.
- **Source enrollment with evidence**: sources submit identity, endpoint
  ownership, contact and reliability evidence before an admin activates them.
- **Asset lifecycle**: register, temporarily disable, re-enable, delist.
- **Admin-gated upgrades and configuration**, with events for every change.
- **Client SDKs** (TypeScript, Python, Rust, Go) and runnable consumer
  [examples](examples/).
- **Security gates in CI**: unit, property, model-based and Kani-bounded tests,
  attack-regression corpus, fuzzing, secret scanning and a supply-chain
  malware scan.

## How it works

```
 source A ─┐
 source B ─┼─ submit_price ─▶ [ Price Oracle contract ] ─▶ median ─▶ AggregatePrice + history
 source C ─┘   (source auth)        │                                     │
                                    └── admin: sources, assets, config    └─▶ SEP-40 reads by
                                                                               consumer contracts
```

1. The admin initializes the contract and registers assets.
2. Sources enroll, submit evidence, and are activated by the admin.
3. Each source signs and submits prices with `submit_price`.
4. Once `min_sources_required` fresh prices exist, the contract publishes the
   median and records it in history.
5. Consumer contracts read it with `lastprice(asset)` or other SEP-40 calls.

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) and
[docs/aggregation-semantics.md](docs/aggregation-semantics.md) for details.

## Quick start

### Prerequisites

- Rust (toolchain pinned in `rust-toolchain.toml`, target `wasm32v1-none`)
- [Stellar CLI](https://developers.stellar.org/docs/tools/developer-tools) for deployment

### Build and test

```bash
make build    # cargo build -p price-oracle --target wasm32v1-none --release
make test     # cargo test -p price-oracle --lib
make lint     # clippy -D warnings
make check    # rustfmt --check
```

### Deploy to testnet

```bash
stellar keys generate admin --network testnet --fund
stellar contract deploy \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --source admin --network testnet
# => C... (contract ID)

stellar contract invoke --id <CONTRACT_ID> --source admin --network testnet -- \
  initialize --admin $(stellar keys address admin) \
  --min_sources_required 1 --max_history_length 100 --decimals 14 \
  --description "Stellar Unified Price Oracle (testnet)"
```

Or use the guided deployer: `./scripts/deploy-oracle.sh`. Record every
deployment in [docs/deployment.md](docs/deployment.md).

### Operate with oracle-cli

```bash
./scripts/oracle-cli.sh add-source --address GBBB... --name "Source A" --admin-key admin
./scripts/oracle-cli.sh register-asset --asset GCCC... --admin-key admin
./scripts/oracle-cli.sh submit-price --source GBBB... --asset GCCC... \
  --price 50000000000000000000 --source-key source-a
./scripts/oracle-cli.sh get-price --asset GCCC...
./scripts/oracle-cli.sh health-check
```

## Deployments

| Network | Contract ID | Status |
|---|---|---|
| Testnet | see [docs/deployment.md](docs/deployment.md) | |
| Mainnet | — | not yet deployed |

## Contract API

### Sources

| Function | Description |
|----------|-------------|
| `add_source(address, name)` | Create a pending source enrollment (admin) |
| `submit_source_evidence(address, identity, endpoint_ownership, operational_contact, reliability)` | Submit required evidence as the source address |
| `activate_source(address)` | Activate a fully evidenced source (admin) |
| `remove_source(address)` | Remove a pending enrollment or an active source (admin) |
| `is_source(address) -> bool` | Check if address is an active source |
| `get_source_enrollment(address) -> Option<SourceEnrollment>` | Review submitted enrollment evidence |
| `get_oracle_sources() -> OracleSources` | Get all active sources |

Enrollment evidence is public on-chain: use public references and non-secret
contact details, never credentials or private keys.

### Submission

| Function | Description |
|----------|-------------|
| `submit_price(source, asset, price, timestamp)` | Submit a price (source auth) |

### Price queries

| Function | Description |
|----------|-------------|
| `get_price(asset) -> AggregatePrice` | Latest aggregated price for an asset |
| `get_source_price(asset, source) -> PriceEntry` | Latest price from a specific source |
| `get_all_prices(asset) -> Vec<PriceEntry>` | Latest prices from all sources |
| `get_latest_ledger() -> u32` | Latest ledger with price data |

### History

| Function | Description |
|----------|-------------|
| `get_historical_price(asset, ledger) -> PriceHistoryEntry` | Price at a ledger (interpolated if enabled) |
| `get_historical_prices(asset, start, end) -> Vec<PriceHistoryEntry>` | Inclusive range of up to 50 ledgers |
| `get_historical_prices_paginated(asset, cursor, limit)` | Up to 50 entries in ascending order; pass `next_cursor` to continue |
| `has_historical_price(asset, ledger) -> bool` | Whether a historical price exists |
| `set_interpolation_enabled(bool)` | Enable/disable gap interpolation (admin) |
| `get_interpolation_enabled() -> bool` | Whether interpolation is enabled |

Start paginated reads with `cursor = 0` and pass each returned `next_cursor`
unchanged. Gaps are skipped.

### Asset lifecycle

| Function | Description |
|----------|-------------|
| `disable_asset(asset)` | Temporarily disable an asset (admin; blocks submissions and aggregation) |
| `enable_asset(asset)` | Re-enable a disabled asset (admin) |
| `is_asset_disabled(asset) -> bool` | Whether an asset is disabled |
| `get_asset_disabled_at(asset) -> u32` | Ledger at which an asset was disabled |
| `unregister_asset(asset)` | Permanently delist an asset (admin; history preserved) |

Disabled assets return `ErrorCode::AssetDisabled` (190) and can be re-enabled.
Delisted assets return `ErrorCode::AssetNotRegistered`. History stays readable
in both cases. See [docs/ASSET_LIFECYCLE_DESIGN.md](docs/ASSET_LIFECYCLE_DESIGN.md).

### SEP-40 consumer interface

| Function | Description |
|----------|-------------|
| `base() -> Asset` | Base asset (USD) |
| `assets() -> Vec<Asset>` | Registered assets (max 100; use pagination beyond that) |
| `assets_paginated(cursor, limit)` | Up to 50 assets in registration order |
| `decimals() -> u32` | Price decimals |
| `resolution() -> u32` | Staleness window in seconds (0 = no expiry) |
| `price(asset, timestamp) -> Option<PriceData>` | Price at or before a timestamp |
| `prices(asset, records) -> Option<Vec<PriceData>>` | Up to 50 latest records |
| `lastprice(asset) -> Option<PriceData>` | Latest aggregated price |

For `assets_paginated`, start with `cursor = None` and pass the last returned
asset as the next cursor. If that asset was unregistered the contract returns
`InvalidPageCursor`; restart from the first page. Page limits are 1–50.

The full list of endpoints is in
[contracts/price-oracle/src/lib.rs](contracts/price-oracle/src/lib.rs) and
[docs/abi.md](docs/abi.md). Error codes: [docs/error-codes.md](docs/error-codes.md).

## Repository layout

| Path | Contents |
|---|---|
| `contracts/price-oracle/` | The Soroban contract and its tests |
| `sdk/` | TypeScript, Python, Rust and Go client SDKs |
| `examples/` | Consumer quickstarts: web dApp, Python submission bot, indexer |
| `scripts/` | Deployment, operations and CI gate scripts |
| `services/` | Off-chain tooling (monitoring, secret scan, release checks) |
| `fuzz/` | Fuzz targets and corpora |
| `docs/` | Design docs, runbooks and policies |

## Operations and policy docs

- [Deployment record](docs/deployment.md) and [runbook](docs/runbook.md)
- [Incident severity and decision authority matrix](docs/incident-severity-matrix.md)
- [Initialization parameter policy](docs/init-parameter-policy.md)
- [Asset listing policy](docs/asset-listing-policy.md)
- [Price scaling helpers (JS / Python)](docs/price-scaling.md)
- [SLA](docs/SLA.md), [treasury budget](docs/treasury-budget.md) and [runway](docs/treasury-runway.md)
- [Security policy](SECURITY.md) and [audit checklist](docs/security-audit-checklist.md)

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md). Every PR must
pass `make build test lint check` and the supply-chain scan
(`make malware-scan`). Changes to `.github/`, `.husky/`, `.devcontainer/`,
`scripts/` and any `package.json` require maintainer review.

## License

[MIT](LICENSE)
