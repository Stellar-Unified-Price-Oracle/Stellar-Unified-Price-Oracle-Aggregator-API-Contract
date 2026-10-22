#![cfg(test)]

//! # #693 — Worst-case configured source-count benchmark
//!
//! Benchmarks the aggregation path (`submit_price` from the last source,
//! `trigger_aggregation`) and the query paths (`get_price`, `get_all_prices`,
//! `lastprice`, `prices`) with the source count configured at its maximum
//! supported value and every source contributing, under a set of adversarial
//! value distributions.
//!
//! Every measurement must stay inside the documented per-transaction network
//! limits (`docs/gas-usage.md`), and the whole benchmark is run twice in fresh
//! environments to prove the baseline is exactly reproducible. Run with
//! `make source-count-bench`; results are described in
//! `docs/source-count-benchmark.md`.

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::types::Asset;
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Maximum supported source count: 10 is the largest value for which
/// `submit_price` still fits the network's 100-entry footprint limit
/// (see `gas_budget_tests::N_SOURCES` and `docs/gas-budget.md`, Ceilings).
const MAX_SOURCES: u32 = 10;

/// Documented per-transaction network limits (`docs/gas-usage.md`).
const TX_CPU_LIMIT: u64 = 100_000_000;
const TX_MEM_LIMIT: u64 = 40_000_000;

const BASE: i128 = 1_000_000;

/// Adversarial value distributions. Every value stays within the default 5%
/// deviation bound so each submission is accepted and lands in the median
/// window — nothing is skipped, so the full-scan path is exercised.
const DISTRIBUTIONS: &[(&str, fn(u32) -> i128)] = &[
    ("all_equal", all_equal),
    ("ascending", ascending),
    ("reverse_sorted_near_miss", reverse_sorted_near_miss),
    ("alternating_extremes", alternating_extremes),
    ("minority_outlier_cluster", minority_outlier_cluster),
];

fn all_equal(_i: u32) -> i128 {
    BASE
}

fn ascending(i: u32) -> i128 {
    BASE + i as i128
}

/// Reverse-sorted, alternating ±1 near-miss values (the `gas_budget_tests` corpus).
fn reverse_sorted_near_miss(i: u32) -> i128 {
    let base = BASE + (MAX_SOURCES - i) as i128;
    if i % 2 == 0 {
        base + 1
    } else {
        base - 1
    }
}

/// Values alternate between the top and bottom of the accepted band (±4%).
fn alternating_extremes(i: u32) -> i128 {
    if i % 2 == 0 {
        BASE + BASE * 4 / 100
    } else {
        BASE - BASE * 4 / 100
    }
}

/// A minority (just under half) clustered at +4%, the rest at the base value.
fn minority_outlier_cluster(i: u32) -> i128 {
    if i < (MAX_SOURCES - 1) / 2 {
        BASE + BASE * 4 / 100
    } else {
        BASE
    }
}

/// (cpu, mem) of a single top-level invocation.
fn measure<F: FnOnce()>(e: &Env, f: F) -> (u64, u64) {
    e.cost_estimate().budget().reset_unlimited();
    f();
    let budget = e.cost_estimate().budget();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

fn oracle(e: &Env) -> (PriceOracleContractClient<'_>, Vec<Address>) {
    e.mock_all_auths();
    e.cost_estimate().budget().reset_unlimited();
    let client = PriceOracleContractClient::new(e, &e.register(PriceOracleContract, ()));
    client.initialize(
        &Address::generate(e),
        &2u32,
        &10u32,
        &7u32,
        &String::from_str(e, "source-count-bench"),
    );
    client.set_max_sources(&MAX_SOURCES);
    let mut sources = Vec::new(e);
    for _ in 0..MAX_SOURCES {
        let s = Address::generate(e);
        client.add_source(&s, &String::from_str(e, "S"));
        sources.push_back(s);
    }
    (client, sources)
}

/// One measured row: (distribution, path, cpu, mem).
type Row = (&'static str, &'static str, u64, u64);

/// Runs every distribution at the maximum source count and returns all rows.
fn run_benchmark() -> std::vec::Vec<Row> {
    let mut rows = std::vec::Vec::new();
    for (name, price_of) in DISTRIBUTIONS {
        let e = Env::default();
        let (client, sources) = oracle(&e);
        let asset = Address::generate(&e);
        client.register_asset(&asset);
        let ts = e.ledger().timestamp();
        for i in 0..MAX_SOURCES - 1 {
            client.submit_price(&sources.get_unchecked(i), &asset, &price_of(i), &ts);
        }

        let last = sources.get_unchecked(MAX_SOURCES - 1);
        let last_price = price_of(MAX_SOURCES - 1);
        let (cpu, mem) = measure(&e, || client.submit_price(&last, &asset, &last_price, &ts));
        rows.push((*name, "submit_price_last_source", cpu, mem));

        let (cpu, mem) = measure(&e, || client.trigger_aggregation(&asset));
        rows.push((*name, "trigger_aggregation", cpu, mem));

        let (cpu, mem) = measure(&e, || {
            client.get_price(&asset, &0u64);
        });
        rows.push((*name, "get_price", cpu, mem));

        let (cpu, mem) = measure(&e, || {
            client.get_all_prices(&asset);
        });
        rows.push((*name, "get_all_prices", cpu, mem));

        let sep40 = Asset::Stellar(asset.clone());
        let (cpu, mem) = measure(&e, || {
            client.lastprice(&sep40);
        });
        rows.push((*name, "lastprice", cpu, mem));

        let (cpu, mem) = measure(&e, || {
            client.prices(&sep40, &1u32);
        });
        rows.push((*name, "prices", cpu, mem));

        assert_eq!(
            client.get_all_prices(&asset).len(),
            MAX_SOURCES,
            "{name}: every source must contribute at the maximum source count"
        );
    }
    rows
}

/// Every path at the maximum source count, under every adversarial
/// distribution, completes inside the documented per-transaction CPU and
/// memory limits.
#[test]
fn source_count_bench_within_documented_limits() {
    for (dist, path, cpu, mem) in run_benchmark() {
        std::println!("SRCBENCH sources={MAX_SOURCES} dist={dist} path={path} cpu={cpu} mem={mem}");
        assert!(
            cpu <= TX_CPU_LIMIT,
            "SRCBENCH LIMIT EXCEEDED path={path} dist={dist} cpu={cpu} limit={TX_CPU_LIMIT}"
        );
        assert!(
            mem <= TX_MEM_LIMIT,
            "SRCBENCH LIMIT EXCEEDED path={path} dist={dist} mem={mem} limit={TX_MEM_LIMIT}"
        );
    }
}

/// Soroban metering is deterministic: two independent runs of the benchmark
/// must produce identical numbers, so the baseline is reproducible on any machine.
#[test]
fn source_count_bench_baseline_is_reproducible() {
    let first = run_benchmark();
    let second = run_benchmark();
    assert_eq!(first.len(), second.len());
    for (a, b) in first.iter().zip(second.iter()) {
        assert_eq!(a, b, "non-reproducible measurement for {}/{}", a.0, a.1);
    }
}

/// The benchmark really runs at the configured maximum: one more source is rejected.
#[test]
fn source_count_bench_runs_at_configured_maximum() {
    let e = Env::default();
    let (client, sources) = oracle(&e);
    assert_eq!(sources.len(), MAX_SOURCES);
    assert_eq!(client.get_max_sources(), MAX_SOURCES);
    let extra = Address::generate(&e);
    assert!(client
        .try_add_source(&extra, &String::from_str(&e, "S"))
        .is_err());
}
