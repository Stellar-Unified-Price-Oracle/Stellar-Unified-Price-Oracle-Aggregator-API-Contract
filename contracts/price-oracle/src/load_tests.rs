//! Load testing suite for the price oracle contract (#102).
//!
//! Simulates high-throughput scenarios with configurable numbers of sources,
//! assets, and submission counts and reports per-run statistics.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
};

use crate::{PriceOracleContract, PriceOracleContractClient};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

struct LoadTestConfig {
    num_sources: u32,
    num_assets: u32,
    submissions_per_source: u32,
}

struct LoadTestResult {
    total_submissions: u32,
    successful_submissions: u32,
    aggregations_triggered: u32,
}

fn run_load_test(config: &LoadTestConfig) -> LoadTestResult {
    let e = Env::default();
    e.mock_all_auths();

    // Set a generous budget for large-scale tests
    e.cost_estimate().budget().reset_unlimited();

    let contract_id = e.register(PriceOracleContract, ());
    let client = PriceOracleContractClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    client.initialize(
        &admin,
        &config.num_sources,
        &200u32,
        &18u32,
        &String::from_str(&e, "Load Test Oracle"),
    );

    // Register sources
    let mut sources: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&e);
    for i in 0..config.num_sources {
        let src = Address::generate(&e);
        let name = String::from_str(&e, "Source");
        client.add_source(&src, &name);
        sources.push_back(src);
        let _ = i;
    }

    // Register assets
    let mut assets: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&e);
    for _ in 0..config.num_assets {
        let asset = Address::generate(&e);
        client.register_asset(&asset);
        assets.push_back(asset);
    }

    let mut total_submissions: u32 = 0;
    let mut successful_submissions: u32 = 0;
    let mut aggregations_triggered: u32 = 0;

    let base_price: i128 = 100_000;

    for round in 0..config.submissions_per_source {
        // Advance ledger so history entries land on distinct ledgers
        let seq = 100u32 + round;
        let ts = 1_000_000u64 + round as u64 * 1000;
        e.ledger().set(LedgerInfo {
            timestamp: ts,
            protocol_version: 26,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 4096,
        });

        for ai in 0..config.num_assets {
            let asset = assets.get_unchecked(ai);

            for si in 0..config.num_sources {
                let source = sources.get_unchecked(si);
                // Vary price slightly per source to exercise median
                let price = base_price + (si as i128) * 10 + (round as i128);
                total_submissions += 1;
                client.submit_price(&source, &asset, &price, &ts);
                successful_submissions += 1;
            }

            // After all sources submit, check if an aggregate was produced
            if client.get_price(&asset, &0u64).is_some() {
                aggregations_triggered += 1;
            }
        }
    }

    LoadTestResult {
        total_submissions,
        successful_submissions,
        aggregations_triggered,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Baseline: 1 source, 1 asset, 10 submissions.
#[test]
fn load_test_single_source_single_asset() {
    let config = LoadTestConfig {
        num_sources: 1,
        num_assets: 1,
        submissions_per_source: 10,
    };
    let result = run_load_test(&config);
    assert_eq!(result.total_submissions, 10);
    assert_eq!(result.successful_submissions, 10);
}

/// 5 sources, 5 assets, 10 rounds.
#[test]
fn load_test_medium_scale() {
    let config = LoadTestConfig {
        num_sources: 5,
        num_assets: 5,
        submissions_per_source: 10,
    };
    let result = run_load_test(&config);
    let expected = config.num_sources * config.num_assets * config.submissions_per_source;
    assert_eq!(result.total_submissions, expected);
    assert_eq!(result.successful_submissions, expected);
    // After all sources submit in each round, aggregation should be triggered
    assert!(result.aggregations_triggered > 0);
}

/// 10 sources, 10 assets, 5 rounds — stress test.
#[test]
fn load_test_high_source_count() {
    let config = LoadTestConfig {
        num_sources: 10,
        num_assets: 10,
        submissions_per_source: 5,
    };
    let result = run_load_test(&config);
    let expected = config.num_sources * config.num_assets * config.submissions_per_source;
    assert_eq!(result.total_submissions, expected);
    assert_eq!(result.successful_submissions, expected);
}

/// 3 sources, 20 assets, 5 rounds — max asset range test.
#[test]
fn load_test_max_assets() {
    let config = LoadTestConfig {
        num_sources: 3,
        num_assets: 20,
        submissions_per_source: 5,
    };
    let result = run_load_test(&config);
    let expected = config.num_sources * config.num_assets * config.submissions_per_source;
    assert_eq!(result.total_submissions, expected);
    assert_eq!(result.successful_submissions, expected);
}

/// Verify aggregate price correctness under load (median of N sources).
#[test]
fn load_test_aggregate_correctness() {
    let e = Env::default();
    e.mock_all_auths();
    e.cost_estimate().budget().reset_unlimited();

    e.ledger().set(LedgerInfo {
        timestamp: 1_000_000,
        protocol_version: 26,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 4096,
    });

    let contract_id = e.register(PriceOracleContract, ());
    let client = PriceOracleContractClient::new(&e, &contract_id);
    let admin = Address::generate(&e);

    // 5 sources, prices: 100, 110, 120, 130, 140  → median = 120
    let num_sources = 5u32;
    client.initialize(
        &admin,
        &num_sources,
        &100u32,
        &18u32,
        &String::from_str(&e, "Load Test Oracle"),
    );

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    let prices = [100i128, 110, 120, 130, 140];
    for p in &prices {
        let src = Address::generate(&e);
        client.add_source(&src, &String::from_str(&e, "S"));
        client.submit_price(&src, &asset, p, &1_000_000u64);
    }

    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(
        agg.price, 120i128,
        "median of [100,110,120,130,140] should be 120"
    );
    assert_eq!(agg.num_sources, 5u32);
}

/// Storage growth test: ensure history is pruned to max_history_length.
#[test]
fn load_test_history_pruning() {
    let e = Env::default();
    e.mock_all_auths();
    e.cost_estimate().budget().reset_unlimited();

    let contract_id = e.register(PriceOracleContract, ());
    let client = PriceOracleContractClient::new(&e, &contract_id);
    let admin = Address::generate(&e);
    let max_history = 5u32;

    client.initialize(
        &admin,
        &1u32,
        &max_history,
        &18u32,
        &String::from_str(&e, "Load Test Oracle"),
    );

    let src = Address::generate(&e);
    client.add_source(&src, &String::from_str(&e, "S"));
    let asset = Address::generate(&e);
    client.register_asset(&asset);

    // Submit 10 prices at 10 different ledgers — history should be capped at 5
    for seq in 100u32..110 {
        e.ledger().set(LedgerInfo {
            timestamp: seq as u64 * 1000,
            protocol_version: 26,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 4096,
        });
        client.submit_price(&src, &asset, &(seq as i128 * 100), &(seq as u64 * 1000));
    }

    // The ledgers that should be absent (pruned): 100..=104
    for seq in 100u32..105 {
        assert!(
            !client.has_historical_price(&asset, &seq),
            "ledger {} should have been pruned",
            seq
        );
    }
    // The 5 most recent ledgers should still be present
    for seq in 105u32..110 {
        assert!(
            client.has_historical_price(&asset, &seq),
            "ledger {} should be present",
            seq
        );
    }
}
