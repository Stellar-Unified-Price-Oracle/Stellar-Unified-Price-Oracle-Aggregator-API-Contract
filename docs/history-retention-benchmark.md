# Worst-Case History Retention Benchmark (#694)

Writes, reads and pruning are measured when each asset's history has reached
the configured retention limit (`max_history_length`).

* Benchmark: `contracts/price-oracle/src/history_retention_benchmark_tests.rs`
* Run: `make retention-bench` (also part of `make test`, so CI runs it)

```bash
make retention-bench   # cargo test -p price-oracle --lib retention_bench -- --nocapture --test-threads=1
```

## Configuration

| Setting | Value | Why |
|---|---|---|
| `max_history_length` | 500 | top of the recommended range in `docs/deployment.md` (100–500) |
| Assets | 2 | each asset is filled to the limit independently |
| Writes past the limit | 5 per asset | every one evicts the oldest entry in the same call |
| Sources / `min_sources_required` | 1 / 1 | every write aggregates and records exactly one history entry per ledger |

One history entry is written per asset per ledger, so after 500 ledgers both
assets hold exactly 500 entries.

## What is measured

Each measurement prints a `RETENTION ...` line with CPU instructions and memory
bytes of the single invocation.

| Operation | When | Line |
|---|---|---|
| `submit_price` write | first entry, the entry that reaches the limit, and each of the 5 evicting writes | `write=first`, `write=at_limit`, `write=evicting_<k>` |
| `get_historical_price` | lookup of the oldest retained entry | `read=get_historical_price_oldest` |
| `get_historical_prices_paginated` | one full page (50) from the start | `read=get_historical_prices_paginated_page` |
| `prices` (SEP-40) | 50 records | `read=prices_page` |
| `prune_history` | prune exactly one entry at the limit | `prune=prune_history_one_entry` |

## Boundary assertions

* **Bounded write cost.** At the limit every write adds one entry and evicts
  one, so the steady state is flat: across the 5 evicting writes, CPU and
  memory may vary by at most 10% (`STEADY_STATE_TOLERANCE_PCT`). Cost that
  grew with every write past the limit would fail this.
* **Bounded read cost.** Each read is measured at the limit and again after
  the evicting writes; the two must be within 10%.
* **Oldest-entry eviction.** After every evicting write, each asset retains
  exactly 500 entries, the oldest retained ledger is the oldest one not yet
  evicted, the retained ledgers are contiguous and end at the current ledger,
  and `get_storage_budget(asset).entry_count` is 500.
* **Explicit pruning.** `prune_history(asset, 499)` at the limit removes
  exactly one entry, and it is the oldest.

All costs are Soroban-metered and therefore deterministic; re-running the
benchmark reproduces the same `RETENTION` lines.
