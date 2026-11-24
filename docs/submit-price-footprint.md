# Performance: `submit_price` Write Footprint Optimization (#875)

## Context & Problem

Each call to `submit_price` previously modified multiple persistent storage entries:
1. `SourceNonce(source)`: Monotonic nonce for replay prevention.
2. `Submission(asset, source)`: Raw price observation.
3. `SrcLastPriceLedger(source)`: Timestamp of last submission for reactivation checks.
4. `TotalSubmissionCount`: Global counter incremented on every submission across all sources and assets.
5. `SourceSubmissionCount(source)`: Per-source submission count.
6. `LastGasRecord`: Written on every aggregation round.

### Concurrency Bottleneck
`TotalSubmissionCount` in particular created a critical ledger contention point: concurrent transactions submitted by different oracle sources in the same ledger collided on this single persistent key, limiting overall submission throughput per ledger.

## Changes & Optimizations

1. **Moved Non-Essential Bookkeeping to Events:**
   - Offloaded `TotalSubmissionCount` and `SourceSubmissionCount` increments from `submit_price` to off-chain indexer derivation.
   - Indexers consume `PriceSubmittedEvent` to track total and per-source submission volumes with zero on-chain persistent storage overhead.
2. **Eliminated Intermediate Gas Telemetry Storage:**
   - Removed persistent writes to `LastGasRecord` on every aggregation pass.
3. **Fixed Persistent Write Footprint:**
   - A non-aggregating `submit_price` touches only:
     - `DataKey::SourceNonce` (1 write)
     - `DataKey::Submission` (1 write)
     - `DataKey::SrcLastPriceLedger` (1 write)
   - When aggregation is triggered, it writes the aggregated result:
     - `DataKey::Aggregate` (1 write)
     - Price history entry (in temporary or configured tier)

## Acceptance Criteria

- Write footprint profiled and reduced to a fixed minimal set of essential persistent keys.
- Write contention on global counters completely eliminated.
