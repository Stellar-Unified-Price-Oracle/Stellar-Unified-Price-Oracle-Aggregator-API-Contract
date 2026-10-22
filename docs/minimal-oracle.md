# Minimal Oracle Contract

## What is the Minimal Oracle?

The minimal oracle is a standalone, stripped-down Soroban price oracle contract that provides
only the core functionality needed to aggregate prices from multiple permissioned sources.
It has no history, no SEP-40 interface, no governance timelocks, and no events — just the
essential mechanics: register sources, submit prices, return the median.

## When to Use Minimal vs Full Oracle

| Concern | Minimal Oracle | Full Oracle (`price-oracle`) |
|---|---|---|
| Use case | Simple integrations, prototypes, PoCs | Production-grade aggregation |
| Gas cost | ~60% lower (fewer storage ops) | Full feature set |
| Audit surface | ~170 lines | ~1 500+ lines across 10 modules |
| SEP-40 | No | Yes |
| Price history | No | Yes (configurable retention) |
| Governance | No | Admin controls, timelocks |
| Events | No | 9 event types |
| Interpolation | No | Yes |

Choose the minimal oracle when:
- You need a quick integration and don't require historical data
- You want the smallest possible audit surface
- Gas cost is a primary constraint
- You are building a prototype or internal tooling

Choose the full oracle when:
- Your consumers need historical price lookups
- You need SEP-40 compliance for interoperability
- You need on-chain events for indexers and monitoring
- You need governance controls (pausing, timelocks, admin transfers)

## Endpoints

### `initialize(admin: Address, decimals: u32)`

Initialises the contract. Must be called exactly once immediately after deployment.

- `admin` — the address that will control source registration
- `decimals` — the number of decimal places prices are expressed in (e.g. `18`)
- Panics with `AlreadyInitialized` (code 1) if called a second time

### `add_source(source: Address)`

Registers a new oracle source that is permitted to submit prices. Admin-only.

- `source` — the address of the new oracle source
- Panics with `SourceAlreadyExists` (code 3) if already registered

### `remove_source(source: Address)`

Removes a previously registered oracle source. Admin-only. Existing price entries from the
removed source are not deleted but will no longer be included in future `get_price` calls
because the source list drives aggregation.

- `source` — the address to remove
- Panics with `SourceNotFound` (code 4) if not registered

### `submit_price(source: Address, asset: Address, price: i128)`

Submits the current price for an asset. The `source` address must sign the transaction
(`source.require_auth()`). Only registered sources may submit.

- `source` — the signing oracle source
- `asset` — the asset being priced (any `Address`, e.g. a token contract address)
- `price` — the price value scaled by `10^decimals` (must be > 0)
- Panics with `SourceNotFound` (code 4) if source is not registered
- Panics with `InvalidPrice` (code 6) if price ≤ 0

### `get_price(asset: Address) -> i128`

Returns the **median** of all prices submitted by registered sources for the given asset.
Sources that have not submitted a price for this asset are excluded from the median computation.

- Panics with `NoData` (code 7) if no registered source has submitted a price for the asset

## Gas Comparison

The minimal oracle uses roughly **60% fewer persistent storage operations** per request than
the full oracle:

| Operation | Minimal Oracle | Full Oracle |
|---|---|---|
| `submit_price` | 1 read + 1 write | 5+ reads/writes (history, ledger, aggregate) |
| `get_price` | 1 read (sources) + N reads (prices) | Same aggregation + 1 history write |
| Contract size | ~6–8 KB WASM | ~25–30 KB WASM |

Fewer storage operations translate directly to lower ledger fees and faster execution.

## Error Codes

| Code | Name | Cause |
|---|---|---|
| 1 | `AlreadyInitialized` | `initialize` called more than once |
| 2 | `NotAuthorized` | Reserved (caller is not admin) |
| 3 | `SourceAlreadyExists` | `add_source` with an already-registered address |
| 4 | `SourceNotFound` | `remove_source` or `submit_price` with an unregistered address |
| 5 | `InsufficientSources` | Reserved for future use |
| 6 | `InvalidPrice` | `submit_price` with price ≤ 0 |
| 7 | `NoData` | `get_price` with no submitted prices |

## How to Deploy

### 1. Build

```bash
cargo build -p minimal-oracle --target wasm32v1-none --release
```

The WASM artifact will be written to:
```
target/wasm32v1-none/release/minimal_oracle.wasm
```

### 2. Deploy to Testnet

```bash
stellar contract deploy \
  --wasm target/wasm32v1-none/release/minimal_oracle.wasm \
  --source <your-identity> \
  --network testnet
```

Save the returned contract ID (e.g. `CAAAA...`).

### 3. Initialize

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source <admin-identity> \
  --network testnet \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --decimals 18
```

### 4. Add a Source

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source <admin-identity> \
  --network testnet \
  -- add_source \
  --source <SOURCE_ADDRESS>
```

### 5. Submit a Price

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --source <source-identity> \
  --network testnet \
  -- submit_price \
  --source <SOURCE_ADDRESS> \
  --asset <ASSET_ADDRESS> \
  --price 50000000000000000000
```

### 6. Query the Price

```bash
stellar contract invoke \
  --id <CONTRACT_ID> \
  --network testnet \
  -- get_price \
  --asset <ASSET_ADDRESS>
```
