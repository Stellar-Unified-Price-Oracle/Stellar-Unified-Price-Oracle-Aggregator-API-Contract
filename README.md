# Stellar Unified Price Oracle Aggregator

A **decentralized price oracle aggregator** built on Soroban (Stellar smart contracts). Collects price data from multiple permissioned oracle sources, aggregates via **median**, and exposes historical price data for consumer contracts.

## Features

- **Multi-source aggregation** — register multiple oracle sources per asset, aggregate via median
- **Admin governance** — admin controls sources, assets, decimals, description, history limits
- **Median price** — robust single-statistic aggregation resistant to outliers and manipulation
- **Per-source prices** — inspect individual source submissions for transparency
- **Historical prices** — ledger-based price history with configurable retention
- **Contract upgradability** — WASM-based upgrade mechanism
- **SEP-40 compliant** — full implementation of the Stellar Oracle Consumer Interface standard
- **Contract events** — all state changes emit on-chain events for indexers and monitoring
- **27 public endpoints** — full admin, source, asset, submission, query, history, and SEP-40 interface

## Contract Interface

### Admin

| Function | Description |
|----------|-------------|
| `initialize(admin, min_sources, max_history, decimals, description)` | Initialize the contract |
| `set_admin(new_admin)` | Transfer admin rights |
| `get_admin_address()` | Get current admin |
| `set_min_sources_required(n)` | Set minimum sources for aggregation |
| `get_min_sources_required()` | Get minimum sources required |
| `set_max_history_length(n)` | Set max historical records per asset |
| `get_max_history_length()` | Get max history length |
| `set_decimals(n)` | Set price decimals |
| `get_decimals()` | Get decimals |
| `set_description(s)` | Set contract description |
| `get_description()` | Get description |
| `upgrade(new_wasm_hash)` | Upgrade contract WASM |

### Source Management

| Function | Description |
|----------|-------------|
| `add_source(address, name)` | Register an oracle source |
| `remove_source(address)` | Remove an oracle source |
| `is_source(address) -> bool` | Check if address is a registered source |
| `get_oracle_sources() -> OracleSources` | Get all registered sources |

### Asset Management

| Function | Description |
|----------|-------------|
| `register_asset(asset)` | Register a new asset |
| `unregister_asset(asset)` | Unregister an asset |
| `is_asset_registered(asset) -> bool` | Check if asset is registered |

### Price Submission

| Function | Description |
|----------|-------------|
| `submit_price(source, asset, price, timestamp)` | Submit a price (source only) |

### Price Queries

| Function | Description |
|----------|-------------|
| `get_price(asset) -> AggregatePrice` | Get latest aggregated price for an asset |
| `get_source_price(asset, source) -> PriceEntry` | Get latest price from a specific source |
| `get_all_prices(asset) -> Vec<PriceEntry>` | Get latest prices from all sources |
| `get_latest_ledger() -> u32` | Get the latest ledger with price data |

### Historical

| Function | Description |
|----------|-------------|
| `get_historical_price(asset, ledger) -> PriceHistoryEntry` | Get historical price at a specific ledger |
| `get_historical_prices(asset, start, end) -> Vec<PriceHistoryEntry>` | Get historical prices in a ledger range |
| `has_historical_price(asset, ledger) -> bool` | Check if historical price exists |

### SEP-40 Oracle Consumer Interface

| Function | Description |
|----------|-------------|
| `base() → Asset` | Returns the base asset (USD) |
| `assets() → Vec<Asset>` | Returns all registered assets as `Asset::Stellar` |
| `decimals() → u32` | Returns price decimals |
| `resolution() → u32` | Returns staleness window in seconds (0 = no expiry) |
| `price(asset, timestamp) → Option<PriceData>` | Get price at or before a given timestamp |
| `prices(asset, records) → Option<Vec<PriceData>>` | Get latest N historical price records |
| `lastprice(asset) → Option<PriceData>` | Get latest aggregated price |

### Contract Events

| Event | Trigger | Topics | Data |
|-------|---------|--------|------|
| `ContractInitializedEvent` | `initialize()` | admin | min_sources, max_history, decimals, description |
| `SourceAddedEvent` | `add_source()` | source, admin | name |
| `SourceRemovedEvent` | `remove_source()` | source, admin | — |
| `AssetRegisteredEvent` | `register_asset()` | asset, admin | — |
| `AssetUnregisteredEvent` | `unregister_asset()` | asset, admin | — |
| `PriceSubmittedEvent` | `submit_price()` | asset, source | price, timestamp |
| `PriceUpdatedEvent` | aggregate price changes | asset | new_price, old_price, timestamp |
| `AdminChangedEvent` | `set_admin()` | old_admin, new_admin | — |
| `ContractUpgradedEvent` | `upgrade()` | new_wasm_hash | — |

## Getting Started

### Prerequisites

- Rust (stable toolchain, see `rust-toolchain.toml`)
- Soroban CLI (optional, for deployment)

### Build

```bash
make build
```

### Test

```bash
make test
```

All **79 tests pass** with zero warnings.

### Deploy

```bash
soroban contract deploy \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --source <identity> \
  --network testnet
```

## Project Structure

```
contracts/price-oracle/
├── Cargo.toml
├── .cargo/config.toml
└── src/
    ├── lib.rs       # Contract entrypoint and endpoint implementations
    ├── types.rs     # Data types, storage keys, error codes
    ├── storage.rs   # Storage helpers and median computation
    ├── events.rs    # Contract event definitions
    └── test.rs      # Test suite (65 tests)
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Contract | Rust, Soroban SDK v26 |
| Target | `wasm32v1-none` (WebAssembly) |
| Aggregation | On-chain median (Rust) |
| Testing | `#[cfg(test)]` with `soroban-sdk/testutils` |

## Error Codes

| Code | Name | Description |
|------|------|-------------|
| 0 | `NotAuthorized` | Caller is not the admin |
| 1 | `AlreadyInitialized` | Contract already initialized |
| 2 | `AssetNotRegistered` | Asset not found |
| 3 | `AssetAlreadyRegistered` | Asset already registered |
| 4 | `SourceAlreadyExists` | Source already registered |
| 5 | `SourceNotFound` | Source not found |
| 6 | `InsufficientSources` | Not enough sources for aggregation |
| 7 | `InvalidPrice` | Price is zero or negative |
| 8 | `NoData` | No price data available |

## Documentation

| Document | Description |
|---|---|
| [Architecture](docs/ARCHITECTURE.md) | System design, data flow, module structure |
| [Deployment Record](docs/deployment.md) | Contract addresses, initialization parameters, admin addresses, deployment checklist |
| [Security Audit Checklist](docs/security-audit-checklist.md) | Pre-audit review items: access control, input validation, arithmetic safety, storage safety, upgrade mechanism, event integrity, known patterns |
| [Monitoring Dashboard](docs/monitoring/README.md) | Grafana dashboard setup and metrics reference |

## License

MIT
