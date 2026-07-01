# Gas Usage Reference

This document describes the gas (CPU instructions + memory) cost profile for each public function in the Stellar Unified Price Oracle Aggregator contract, along with how to run the gas tracking tooling yourself.

---

## How to Run the Gas Benchmarks

The gas tracking module lives in the contract's test suite and uses the Soroban test environment's built-in budget tracking API.

```bash
cargo test -p price-oracle --lib gas_tracking::gas_report -- --nocapture
```

This prints a formatted table to stdout showing CPU instruction counts and memory bytes for each function across different input sizes.

The source for the benchmarks is at:
- [`contracts/price-oracle/src/gas_tracking.rs`](../contracts/price-oracle/src/gas_tracking.rs) — the test module (runs in the contract crate)
- [`scripts/gas-tracking.rs`](../scripts/gas-tracking.rs) — standalone reference copy with usage comments

---

## What Is Measured

The Soroban VM tracks two budget dimensions per transaction:

| Dimension | Unit | Mainnet limit (approx.) |
|---|---|---|
| **CPU instructions** | abstract instruction count | 100,000,000 |
| **Memory** | bytes | 40,000,000 |

The benchmarks call `env.budget().reset_default()` before each invocation and read `cpu_instruction_count()` and `memory_bytes_count()` afterwards. All measurements are taken inside the Soroban test environment with `mock_all_auths()`.

---

## Benchmark Results (Representative)

> **Note:** Exact numbers vary by SDK version, host platform, and input data. Run the benchmarks yourself for authoritative numbers against your deployment version. The table below shows representative relative costs and scaling trends.

### initialize

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| default params | ~2,000,000 | ~50,000 |

One-time operation. Cost is fixed regardless of future state.

---

### add_source

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| 0 existing sources | ~1,500,000 | ~40,000 |
| 10 existing sources | ~1,600,000 | ~45,000 |
| 49 existing sources | ~2,200,000 | ~60,000 |

Cost scales sub-linearly with the number of existing sources (storage read of source list + append).

---

### register_asset

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| single asset | ~1,200,000 | ~35,000 |

Fixed cost. No dependency on existing asset count.

---

### submit_price

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| 1 source | ~3,000,000 | ~80,000 |
| 10 sources | ~5,500,000 | ~130,000 |
| 50 sources | ~18,000,000 | ~400,000 |

This is the most frequently called function. Cost scales with the number of sources because the aggregator reads all source prices and recomputes the median on every submission that crosses the `min_sources_required` threshold.

**Recommendation:** Keep active source count under 20 for comfortable headroom within the mainnet CPU limit.

---

### get_price

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| 1 source | ~1,800,000 | ~45,000 |
| 10 sources | ~3,000,000 | ~75,000 |
| 50 sources | ~10,000,000 | ~250,000 |

Read-only. Scales with source count due to median computation on retrieval.

---

### get_all_prices

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| 1 source | ~1,500,000 | ~40,000 |
| 10 sources | ~4,000,000 | ~100,000 |
| 50 sources | ~16,000,000 | ~370,000 |

Returns the full list of per-source prices. Cost scales linearly with source count — each source price is a separate storage read.

---

### get_historical_price

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| 10 history entries | ~1,200,000 | ~30,000 |
| 50 history entries | ~1,250,000 | ~32,000 |
| 100 history entries | ~1,300,000 | ~35,000 |

Near-constant cost. History is indexed by ledger number, so lookup is O(1) regardless of history depth.

---

### upgrade

| Variant | CPU (instr.) | Mem (bytes) |
|---|---|---|
| same wasm | ~2,500,000 | ~60,000 |

One-time admin operation. Cost is dominated by WASM hash lookup and storage update.

---

## Scaling Summary

| Function | Scales with | Direction |
|---|---|---|
| `initialize` | — | Fixed |
| `add_source` | Source count | Sub-linear |
| `register_asset` | — | Fixed |
| `submit_price` | Source count | Linear |
| `get_price` | Source count | Linear |
| `get_all_prices` | Source count | Linear |
| `get_historical_price` | History depth | ~Fixed (indexed) |
| `upgrade` | — | Fixed |

---

## Optimization Notes

- The dominant cost driver is **source count** in `submit_price`, `get_price`, and `get_all_prices`. If gas costs are a concern, keep the registered source count low (10–20).
- `get_historical_price` is cheap because history is keyed by ledger number in persistent storage.
- `submit_price` triggers median recomputation only when `min_sources_required` is met; submissions that don't trigger aggregation are cheaper.
- Consumer contracts that call `get_price` or `lastprice` (SEP-40) should account for source-count-dependent cost when estimating fees.

---

## Further Reading

- [Source Onboarding Guide](./source-onboarding.md)
- [Monitoring Setup Guide](./monitoring-setup.md)
- [Soroban Budget Docs](https://developers.stellar.org/docs/learn/smart-contract-internals/fees-and-metering)
