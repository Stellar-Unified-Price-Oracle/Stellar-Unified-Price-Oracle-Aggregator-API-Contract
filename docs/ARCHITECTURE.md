# Architecture: Stellar Unified Price Oracle Aggregator

## Overview

The Stellar Unified Price Oracle Aggregator is a decentralized price oracle smart contract built on Soroban (Stellar's smart contract platform). It collects price data from multiple permissioned oracle sources, aggregates them using a median calculation, and exposes both current and historical price data to consumer contracts via the SEP-40 standard interface.

## Module Structure

```
contracts/price-oracle/src/
├── lib.rs           # Contract struct & public endpoints (26 public functions)
├── types.rs         # Data structures, storage keys, error codes
├── storage.rs       # Storage helpers, median computation, TTL management
├── admin.rs         # Admin configuration & management (decimals, thresholds, etc)
├── assets.rs        # Asset registration & management
├── sources.rs       # Oracle source registration & heartbeat mechanism
├── prices.rs        # Price submission, aggregation, querying
├── history.rs       # Historical price storage and retrieval
├── events.rs        # 18+ contract event definitions
├── errors.rs        # 12 error codes
├── test_helpers.rs  # Shared test utilities
└── test.rs          # 92+ unit tests
```

## Data Flow

### Price Submission Flow

```
Source calls submit_price()
    ↓
Verify source is registered & authenticated
Verify asset is registered
Validate price > 0 and timestamp within threshold
    ↓
Store price as latest submission for (asset, source) pair
Emit PriceSubmittedEvent
    ↓
Collect all source prices for asset
    ↓
If sufficient sources (≥ min_sources_required):
  - Compute median of all prices
  - Update aggregate price storage
  - Store price in history (temporary storage)
  - Emit PriceAggregatedEvent
    ↓
Else: Emit SourcesInsufficientEvent
```

### Price Query Flow

```
Consumer calls get_price(asset)
    ↓
Check asset is registered
Retrieve aggregate price from persistent storage
    ↓
Apply staleness checks (resolution window)
    ↓
Return AggregatePrice or None
```

## Storage Architecture

### Storage Keys (DataKey enum)

| Key | Type | Purpose |
|-----|------|---------|
| `Admin` | Address | Current admin address |
| `Source(address)` | bool | Registered oracle source flag |
| `AssetRegistered(address)` | bool | Registered asset flag |
| `Submission(asset, source)` | PriceEntry | Latest price from source for asset |
| `Aggregate(asset)` | AggregatePrice | Latest median-aggregated price |
| `PriceHistory(asset, ledger)` | PriceHistoryEntry | Historical price at ledger (temporary) |
| `PriceHistoryLedgers(asset)` | Vec<u32> | Index of history ledgers (persistent) |
| `MinSourcesRequired` | u32 | Minimum sources for aggregation |
| `MaxHistoryLength` | u32 | Max history records per asset |
| `Decimals` | u32 | Price decimal precision |
| `Description` | String | Contract description |
| `Resolution` | u32 | Staleness window in seconds |
| `TimestampThreshold` | u64 | Max future timestamp allowed (seconds) |
| `MaxPriceDeviation` | u32 | Max allowed deviation in basis points (5% = 500) |
| `SourceHeartbeat(source)` | u64 | Last heartbeat timestamp |
| `HeartbeatInterval` | u64 | Required heartbeat interval (seconds) |
| `InactiveSource(source)` | bool | Marked as inactive flag |

### Storage Tiers

- **Persistent Storage**: Admin config, asset/source registries, current aggregate prices
  - TTL: 1000 ledgers + 4000 ledger bump
  - Use case: Long-lived state

- **Temporary Storage**: Historical prices
  - TTL: Configured but typically shorter
  - Use case: Audit trail & back-testing

## Median Computation

The contract uses a **quicksort-based median algorithm**:

```rust
fn compute_median(prices: &Vec<i128>) -> i128 {
    // Sort prices in-place
    sort_prices(&mut sorted);
    
    if even_count {
        // Return average of two middle values
        (sorted[n/2 - 1] + sorted[n/2]) / 2
    } else {
        // Return middle value
        sorted[n/2]
    }
}
```

**Properties**:
- Resistant to outliers (up to 50% of sources can be corrupted)
- Requires minimum sources before aggregation occurs
- Latest timestamp of all contributing sources is recorded

## Admin Functions

### Configuration Management

| Function | Purpose |
|----------|---------|
| `set_min_sources_required(u32)` | Minimum sources for valid aggregation |
| `set_max_history_length(u32)` | Maximum historical records retained |
| `set_decimals(u32)` | Price decimal precision (e.g., 18 for 1e-18) |
| `set_description(String)` | Human-readable contract description |
| `set_resolution(u32)` | Staleness window: prices older than this are stale |
| `set_timestamp_threshold(u64)` | Max allowed future timestamp offset (default: 300s) |
| `set_max_price_deviation(u32)` | Max deviation % in basis points (e.g., 500 = 5%) |
| `set_heartbeat_interval(u64)` | Required heartbeat interval for sources (seconds) |

## Source Heartbeat Mechanism

Sources must periodically call `submit_heartbeat()` to remain active:

```
submit_heartbeat(source) is called
    ↓
Update source.last_heartbeat = now()
If source was inactive:
  - Mark as active
  - Emit SourceActiveAgainEvent
    ↓
Emit SourceHeartbeatEvent
```

**Inactivity Tracking**:
- If `now() > last_heartbeat + heartbeat_interval`, source is marked inactive
- Inactive sources are excluded from aggregation
- Admin can query inactive sources: `get_inactive_sources()` returns count
- Individual check: `is_source_inactive(source)` returns bool

## Event System

Contract emits 18+ events for all state changes:

| Event | Trigger | Topics |
|-------|---------|--------|
| `PriceSubmittedEvent` | Price submission | asset, source |
| `PriceAggregatedEvent` | Aggregation success | asset |
| `SourcesInsufficientEvent` | Not enough sources | asset |
| `PriceDeviationFlaggedEvent` | Price exceeds deviation threshold | asset, source |
| `SourceHeartbeatEvent` | Heartbeat submitted | source |
| `SourceInactiveEvent` | Source marked inactive | source |
| `SourceActiveAgainEvent` | Inactive source reactivates | source |
| `SourceAddedEvent` | New source registered | source, admin |
| `SourceRemovedEvent` | Source removed | source, admin |
| `AssetRegisteredEvent` | New asset registered | asset, admin |
| `AssetUnregisteredEvent` | Asset removed | asset, admin |
| `AdminChangedEvent` | Admin role transferred | old_admin, new_admin |
| `ContractUpgradedEvent` | WASM upgraded | new_wasm_hash |

## SEP-40 Oracle Consumer Interface

The contract implements the full SEP-40 standard for oracle consumers:

```
base()              → Asset (USD)
assets()            → Vec<Asset> (all registered assets as Stellar)
decimals()          → u32 (price decimal places)
resolution()        → u32 (staleness window in seconds)
lastprice(asset)    → Option<PriceData> (latest aggregated price)
price(asset, ts)    → Option<PriceData> (price at or before timestamp)
prices(asset, n)    → Option<Vec<PriceData>> (last n prices)
```

## Error Handling

| Code | Name | Scenario |
|------|------|----------|
| 0 | `NotAuthorized` | Caller is not admin or source |
| 1 | `AlreadyInitialized` | Contract already initialized |
| 2 | `AssetNotRegistered` | Asset not in registry |
| 3 | `AssetAlreadyRegistered` | Asset already registered |
| 4 | `SourceAlreadyExists` | Source already registered |
| 5 | `SourceNotFound` | Source not found |
| 6 | `InsufficientSources` | Not enough sources for aggregation |
| 7 | `InvalidPrice` | Price ≤ 0 |
| 8 | `NoData` | No price data available |
| 9 | `InvalidTimestamp` | Timestamp too far in future |
| 10 | `InvalidConfiguration` | Invalid config parameter |
| 11 | `DescriptionTooLong` | Description exceeds 256 chars |

## Testing Strategy

- **92+ unit tests** covering:
  - Admin functions & authorization
  - Asset & source management
  - Price submission & aggregation
  - Median calculation (odd/even cases)
  - Historical price queries
  - SEP-40 interface compliance
  - Authorization & error cases
  - Heartbeat & inactivity tracking
  - Deviation threshold checks
- **5 property-based tests** (proptest) for median invariants
- All tests pass with zero warnings

## Security Considerations

1. **Authorization**: All admin functions require admin signature; all source functions require source signature
2. **Median Aggregation**: Naturally resistant to up to 50% malicious sources
3. **Timestamp Validation**: Prevents far-future prices that could mislead consumers
4. **TTL Management**: Automatic cleanup of stale data via Soroban's persistent/temporary storage
5. **History Pruning**: Automatic removal of oldest prices when max_history_length is exceeded

## Performance Characteristics

- **Median Computation**: O(n log n) quicksort on active source prices
- **Storage Operations**: O(1) for price lookups, O(n) for full history scan
- **Gas Cost**: Depends on number of sources; designed for ~10-20 sources per asset
