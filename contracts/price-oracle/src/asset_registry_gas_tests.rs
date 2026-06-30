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
    let cpu = e.budget().cpu_instruction_count();
    let mem = e.budget().memory_bytes_count();

    println!("bench_asset_registry_lookup_50_assets: cpu={cpu} mem={mem}");
}
