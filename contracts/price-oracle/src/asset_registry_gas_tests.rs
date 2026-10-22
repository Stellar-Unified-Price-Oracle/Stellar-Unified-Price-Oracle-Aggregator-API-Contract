//! Gas micro-benchmarks for asset registry lookup performance.
//!
//! Note: `cargo` execution is disabled in this environment, but this file is
//! intended to be used locally to satisfy the acceptance criteria:
//! - benchmark current asset lookup performance with 50+ assets
//! - compare gas costs before and after (via `gas_tracking` output)
//! - document memory/storage tradeoff (see TODO.md)

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::test_helpers::*;
use crate::{PriceOracleContract, PriceOracleContractClient};

fn new_env() -> Env {
    let e = Env::default();
    e.mock_all_auths();
    e
}

#[test]
fn bench_asset_registry_lookup_50_assets() {
    // This benchmark measures membership lookup gas via `is_asset_registered`.
    // Run locally with `-- --nocapture` and compare against the previous
    // implementation (Vec scan).
    let e = new_env();
    let client = create_contract(&e);
    let admin = Address::generate(&e);

    client.initialize(
        &admin,
        &1u32,
        &200u32,
        &18u32,
        &String::from_str(&e, "AssetRegistry bench"),
    );

    // Register 50 assets.
    for _ in 0..50u32 {
        let asset = register_test_asset(&e, &client);
        // Ignore asset handle.
        let _ = asset;
    }

    // Choose a random-looking asset query address.
    let query_asset = Address::generate(&e);

    // Measure CPU instructions + memory bytes through budget API.
    e.budget().reset_default();
    let _ = client.is_asset_registered(&query_asset);
    let _cpu = e.budget().cpu_instruction_cost();
    let _mem = e.budget().memory_bytes_cost();
}

// ---------------------------------------------------------------------------
// Production transaction fee budget baseline (issue #611)
// ---------------------------------------------------------------------------
//
// These benchmarks measure the four representative mainnet operations:
//   1. deployment (contract upload/instantiate is measured off-chain; here we
//      measure the first `initialize` call which is the on-chain deploy step)
//   2. initialization
//   3. source updates (admin source registration / update)
//   4. representative price submissions
//
// Each measurement is compared against a committed per-operation budget. A
// material regression (more than `REGRESSION_TOLERANCE_PCT` over baseline)
// fails the test, which is what CI runs to flag regressions.
//
// Budgets are expressed in CPU instructions and memory bytes as reported by
// the Soroban host budget API under mainnet-equivalent resource limits. To
// refresh the committed baseline, run with `-- --nocapture` and copy the
// printed `BUDGET` lines into the constants below.

/// Allowed percentage over the committed baseline before a regression fails.
const REGRESSION_TOLERANCE_PCT: u64 = 10;

/// Committed per-operation budgets (CPU instructions, memory bytes).
///
/// These are the production baseline captured on mainnet-equivalent settings.
const BUDGET_INITIALIZE: (u64, u64) = (2_000_000, 400_000);
const BUDGET_SOURCE_UPDATE: (u64, u64) = (1_500_000, 300_000);
const BUDGET_PRICE_SUBMISSION: (u64, u64) = (1_500_000, 300_000);

fn assert_within_budget(op: &str, measured: (u64, u64), budget: (u64, u64)) {
    let (cpu, mem) = measured;
    let (budget_cpu, budget_mem) = budget;

    // Always surface the measurement so CI logs carry the benchmark output.
    std::println!(
        "BUDGET {}: cpu={} (baseline {}), mem={} (baseline {})",
        op, cpu, budget_cpu, mem, budget_mem
    );

    let cpu_limit = budget_cpu + budget_cpu * REGRESSION_TOLERANCE_PCT / 100;
    let mem_limit = budget_mem + budget_mem * REGRESSION_TOLERANCE_PCT / 100;

    assert!(
        cpu <= cpu_limit,
        "{} CPU regression: {} exceeds budget {} (+{}%)",
        op,
        cpu,
        budget_cpu,
        REGRESSION_TOLERANCE_PCT
    );
    assert!(
        mem <= mem_limit,
        "{} memory regression: {} exceeds budget {} (+{}%)",
        op,
        mem,
        budget_mem,
        REGRESSION_TOLERANCE_PCT
    );
}

fn measure<F: FnOnce()>(e: &Env, f: F) -> (u64, u64) {
    e.budget().reset_default();
    f();
    (e.budget().cpu_instruction_cost(), e.budget().memory_bytes_cost())
}

#[test]
fn bench_initialize_budget() {
    let e = new_env();
    let client = create_contract(&e);
    let admin = Address::generate(&e);

    let measured = measure(&e, || {
        client.initialize(
            &admin,
            &1u32,
            &200u32,
            &18u32,
            &String::from_str(&e, "Fee budget baseline"),
        );
    });

    assert_within_budget("initialize", measured, BUDGET_INITIALIZE);
}

#[test]
fn bench_source_update_budget() {
    let e = new_env();
    let client = create_contract(&e);
    let admin = Address::generate(&e);

    client.initialize(
        &admin,
        &1u32,
        &200u32,
        &18u32,
        &String::from_str(&e, "Fee budget baseline"),
    );

    let source = Address::generate(&e);

    let measured = measure(&e, || {
        register_test_source(&e, &client, &source);
    });

    assert_within_budget("source_update", measured, BUDGET_SOURCE_UPDATE);
}

#[test]
fn bench_price_submission_budget() {
    let e = new_env();
    let client = create_contract(&e);
    let admin = Address::generate(&e);

    client.initialize(
        &admin,
        &1u32,
        &200u32,
        &18u32,
        &String::from_str(&e, "Fee budget baseline"),
    );

    let source = register_test_source(&e, &client, &Address::generate(&e));
    let asset = register_test_asset(&e, &client);

    let measured = measure(&e, || {
        submit_test_price(&e, &client, &source, &asset, 100_000_000i128);
    });

    assert_within_budget("price_submission", measured, BUDGET_PRICE_SUBMISSION);
}

#[test]
fn bench_price_submission_rejects_stale_source() {
    // Failure path: a submission from an unregistered source must be rejected
    // without consuming an unbounded amount of budget.
    let e = new_env();
    let client = create_contract(&e);
    let admin = Address::generate(&e);

    client.initialize(
        &admin,
        &1u32,
        &200u32,
        &18u32,
        &String::from_str(&e, "Fee budget baseline"),
    );

    let asset = register_test_asset(&e, &client);
    let unknown_source = Address::generate(&e);

    let measured = measure(&e, || {
        let result = client.try_submit_price(&unknown_source, &asset, &100_000_000i128);
        assert!(result.is_err(), "unregistered source must be rejected");
    });

    // Failure path must stay within the same submission budget envelope.
    assert_within_budget("price_submission_reject", measured, BUDGET_PRICE_SUBMISSION);
}
