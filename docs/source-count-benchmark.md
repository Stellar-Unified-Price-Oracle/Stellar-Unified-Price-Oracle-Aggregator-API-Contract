# Worst-Case Source-Count Benchmark (#693)

Aggregation and query paths are benchmarked with the source count configured
at its **maximum supported value** and every source contributing, under
adversarial value distributions.

* Benchmark: `contracts/price-oracle/src/source_count_benchmark_tests.rs`
* Run: `make source-count-bench` (also part of `make test`, so CI runs it)

```bash
make source-count-bench   # cargo test -p price-oracle --lib source_count_bench -- --nocapture --test-threads=1
```

Each measurement prints one line:

```text
SRCBENCH sources=10 dist=<distribution> path=<path> cpu=<instructions> mem=<bytes>
```

## Configuration

| Setting | Value | Why |
|---|---|---|
| `max_sources` | 10 | Largest source count for which `submit_price` still fits the 100-entry footprint limit (`docs/gas-budget.md`, Ceilings). The benchmark sets it explicitly and asserts an 11th source is rejected. |
| Contributing sources | 10 | Every source submits, so the median scan covers the full set. |
| `min_sources_required` | 2 | Aggregation runs on every submission after the second. |

## Paths

| Path | Measured call |
|---|---|
| `submit_price_last_source` | the 10th source's `submit_price` after the other 9 have submitted — full scan, sort and history write |
| `trigger_aggregation` | permissionless re-aggregation over all 10 submissions |
| `get_price` | aggregate read |
| `get_all_prices` | per-source read of all 10 submissions |
| `lastprice` | SEP-40 latest price |
| `prices` | SEP-40 history read, 1 record |

## Distributions

All values stay inside the default 5% deviation bound, so every submission is
accepted and lands in the median window — nothing is skipped.

| Distribution | Shape |
|---|---|
| `all_equal` | every source at the same value (friendly control) |
| `ascending` | strictly increasing, already sorted |
| `reverse_sorted_near_miss` | reverse-sorted, alternating ±1 near-miss values (the `gas_budget_tests` corpus) |
| `alternating_extremes` | values alternate between +4% and −4% |
| `minority_outlier_cluster` | 4 of 10 sources clustered at +4%, the rest at the base value |

## Limits

Every path, under every distribution, must complete within the documented
per-transaction network limits (`docs/gas-usage.md`):

| Dimension | Limit |
|---|---|
| CPU instructions | 100,000,000 |
| Memory | 40,000,000 bytes |

A breach fails the test with
`SRCBENCH LIMIT EXCEEDED path=… dist=… cpu=… limit=…` (or `mem=…`).

## Reproducible baseline

Soroban metering is deterministic. `source_count_bench_baseline_is_reproducible`
runs the full benchmark twice in fresh environments and asserts every
`(distribution, path, cpu, mem)` row is identical, so the `SRCBENCH` output is
the baseline on any machine. The `reverse_sorted_near_miss` /
`submit_price_last_source` row is the same input as the committed
`submit_price` budget in `docs/gas-budget.md`.
