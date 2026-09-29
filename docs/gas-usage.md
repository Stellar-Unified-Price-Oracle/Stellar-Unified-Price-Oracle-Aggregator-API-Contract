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

## Load Test v2 — Adversarial Patterns (#413)

Reproduce everything below with one command:

```bash
make load-test   # cargo test -p price-oracle --lib load_v2 -- --nocapture --test-threads=1
```

Source: `contracts/price-oracle/src/load_v2_tests.rs`. Each scenario prints
`LOADV2 ...` lines; the numbers below are from that output (soroban-env-host
26.1.3, CPU instructions are deterministic).

### Survived

| Scenario | Result |
|---|---|
| Byzantine minority, 9 sources, f = 0…4 all pushing +5% | Median stays inside the honest range for every f < n/2 (asserted). Deviation from honest centre: f=0: 2 bps, f=1: 2 bps, f=2: 6 bps, f=3: 11 bps, f=4: 15 bps. **Bound:** for f < n/2 the aggregate is always within `[min honest, max honest]`, i.e. at most the honest spread (here 36 bps). |
| Governance race: `set_min_sources_required` flipped mid-round over 8 rounds | 8 aggregate writes, 0 torn — every aggregate satisfied the quorum rule in force for the transaction that wrote it (asserted). Soroban executes each invocation atomically, so a parameter is never half-applied. |
| Diurnal + thundering-herd bursts, 5 assets × 60 rounds, 1 080 submissions | Quiet avg 10 295 605 CPU/submit, herd avg 10 909 202 (+6%), herd max 16 249 883. History length capped at `max_history_length` (50) — no unbounded storage growth. |

### Requires mitigation

| Finding | Numbers | Follow-up |
|---|---|---|
| Byzantine **majority** captures the median | f = 5/9 → median 1 050 000, 500 bps off | Expected for a median; mitigation is source-set governance / BFT filtering, not aggregation. |
| **Hostile load degrades honest throughput**: once assets are fully populated, `submit_prices` can carry only one asset per transaction | 10 sources/asset: batch of 1 = 6 261 240 CPU; batch of 2 needs **152 footprint entries (> 100)**; batch of 20 needs 1 232 entries, 325 writes (> 50), 64 MB memory (> 40 MB). Reproduce: `gas_budget_tests` with `BATCH_LEN = 2`. | File: reduce per-asset footprint of `submit_prices` (shared reads, fewer per-source keys). |
| `submit_price` becomes uncallable at 11 sources per asset | 11th submission needs 102 footprint entries (> 100). The "10–20 sources" guidance above is therefore unsafe above 10. | File: cap `max_sources` at 10 or bound the aggregation scan. |
| No per-source submission rate limit | 50/50 same-source submissions accepted over 50 ledgers with `min_submission_interval = 5` (that setting is a staleness window, not a rate limit). | File: add a per-source submission rate limit. |
| `query_rate_limit` is stored but not enforced | 250/250 `get_price` calls accepted over 50 ledgers with `query_rate_limit = 1`. | File: enforce the query rate limit or remove the setting. |

---

## Further Reading

- [Source Onboarding Guide](./source-onboarding.md)
- [Monitoring Setup Guide](./monitoring-setup.md)
- [Soroban Budget Docs](https://developers.stellar.org/docs/learn/smart-contract-internals/fees-and-metering)
