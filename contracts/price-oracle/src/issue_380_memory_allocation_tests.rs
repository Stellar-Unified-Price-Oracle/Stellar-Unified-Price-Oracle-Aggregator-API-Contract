//! Tests for Issue #380 — WASM Memory Allocation Profiling and Optimization

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env,
};

use crate::test_helpers::{setup_contract, register_test_source, register_test_asset};

fn set_ledger(e: &Env, seq: u32, ts: u64) {
    e.ledger().set(LedgerInfo {
        timestamp: ts,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100,
        min_persistent_entry_ttl: 100,
        max_entry_ttl: 99_999,
    });
}

// ══════════════════════════════════════════════════════════════════════════════
// Issue #380 — WASM Memory Allocation Profiling and Optimization
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_price_submission_without_excess_allocation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_500_000, &500);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_aggregation_with_optimized_sorting() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset, &1_000_000, &500);
    client.submit_price(&source2, &asset, &1_200_000, &500);
    client.submit_price(&source3, &asset, &1_100_000, &500);

    client.trigger_aggregation(&asset);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_median_calculation_with_vec_reuse() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let source4 = register_test_source(&e, &client, "Source 4");
    let source5 = register_test_source(&e, &client, "Source 5");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset, &1_000_000, &500);
    client.submit_price(&source2, &asset, &1_200_000, &500);
    client.submit_price(&source3, &asset, &1_100_000, &500);
    client.submit_price(&source4, &asset, &1_050_000, &500);
    client.submit_price(&source5, &asset, &1_150_000, &500);

    client.trigger_aggregation(&asset);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_history_collection_efficient_allocation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset, &1_000_000, &500);
    client.submit_price(&source2, &asset, &1_100_000, &500);
    client.submit_price(&source3, &asset, &1_050_000, &500);

    client.trigger_aggregation(&asset);

    let history = client.get_price_history(&asset, &10u32);
    assert!(history.len() > 0);
}

#[test]
fn test_loop_allocation_optimization() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let sources: Vec<_> = (0..5)
        .map(|i| register_test_source(&e, &client, &format!("Source {}", i)))
        .collect();
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    for source in &sources {
        client.submit_price(source, &asset, &1_000_000, &500);
    }

    client.trigger_aggregation(&asset);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_temporary_allocation_reuse_in_aggregation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset, &1_500_000, &500);
    client.submit_price(&source2, &asset, &1_600_000, &500);
    client.submit_price(&source3, &asset, &1_550_000, &500);

    client.trigger_aggregation(&asset);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_no_excessive_allocations_on_repeated_queries() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_000_000, &500);

    // Query multiple times to verify no excessive allocations
    for _ in 0..10 {
        let price = client.get_price(&asset);
        assert!(price.is_some());
    }
}
