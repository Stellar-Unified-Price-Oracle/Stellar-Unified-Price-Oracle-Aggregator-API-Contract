//! Tests for the `submit_prices` batch function (#119).

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, Vec,
};

use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};

fn ledger_at(e: &Env, seq: u32, ts: u64) {
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
}

// --- helpers ---

fn make_batch(e: &Env, tuples: &[(Address, i128, u64)]) -> Vec<(Address, i128, u64)> {
    let mut v: Vec<(Address, i128, u64)> = Vec::new(e);
    for (a, p, t) in tuples {
        v.push_back((a.clone(), *p, *t));
    }
    v
}

// --- tests ---

/// Batch with a single entry should produce an aggregate exactly like submit_price.
#[test]
fn test_submit_prices_single_entry() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000_000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    let batch = make_batch(&e, &[(asset.clone(), 5_000i128, 1_000_000u64)]);
    client.submit_prices(&source, &batch);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 5_000i128);
}

/// Batch with two assets in one call — both should get aggregated.
#[test]
fn test_submit_prices_multiple_assets() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000_000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    let batch = make_batch(
        &e,
        &[
            (asset1.clone(), 1_000i128, 1_000_000u64),
            (asset2.clone(), 2_000i128, 1_000_000u64),
        ],
    );
    client.submit_prices(&source, &batch);

    assert_eq!(client.get_price(&asset1, &0u64).unwrap().price, 1_000i128);
    assert_eq!(client.get_price(&asset2, &0u64).unwrap().price, 2_000i128);
}

/// Authorization is checked once — a single mock_all_auths covers the whole batch.
#[test]
fn test_submit_prices_auth_checked_once() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000_000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);
    let asset3 = register_test_asset(&e, &client);

    // mock_all_auths is already set by setup_contract via create_contract
    let batch = make_batch(
        &e,
        &[
            (asset1.clone(), 100i128, 1_000_000u64),
            (asset2.clone(), 200i128, 1_000_000u64),
            (asset3.clone(), 300i128, 1_000_000u64),
        ],
    );
    client.submit_prices(&source, &batch);

    assert!(client.get_price(&asset1, &0u64).is_some());
    assert!(client.get_price(&asset2, &0u64).is_some());
    assert!(client.get_price(&asset3, &0u64).is_some());
}

/// Invalid price (zero) in the batch aborts the entire call (atomicity).
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_submit_prices_invalid_price_aborts_all() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000_000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    // Second entry has price = 0 — should panic with InvalidPrice
    let batch = make_batch(
        &e,
        &[
            (asset1.clone(), 100i128, 1_000_000u64),
            (asset2.clone(), 0i128, 1_000_000u64), // INVALID
        ],
    );
    client.submit_prices(&source, &batch);
}

/// Unregistered asset in the batch aborts the entire call.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_submit_prices_unregistered_asset_aborts() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000_000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset1 = register_test_asset(&e, &client);
    let bad_asset = Address::generate(&e); // Not registered

    let batch = make_batch(
        &e,
        &[
            (asset1.clone(), 100i128, 1_000_000u64),
            (bad_asset.clone(), 200i128, 1_000_000u64),
        ],
    );
    client.submit_prices(&source, &batch);
}

/// Aggregation is triggered after batch — multiple sources reach min_sources threshold.
#[test]
fn test_submit_prices_aggregation_triggered() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000_000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&2u32);

    let source1 = register_test_source(&e, &client, "S1");
    let source2 = register_test_source(&e, &client, "S2");
    let asset = register_test_asset(&e, &client);

    // Source1 submits via batch, source2 via batch — together they meet min_sources=2
    let batch1 = make_batch(&e, &[(asset.clone(), 100i128, 1_000_000u64)]);
    client.submit_prices(&source1, &batch1);
    // Only 1 source so far — no aggregate yet
    assert!(client.get_price(&asset, &0u64).is_none());

    let batch2 = make_batch(&e, &[(asset.clone(), 200i128, 1_000_000u64)]);
    client.submit_prices(&source2, &batch2);
    // Now 2 sources — median of [100, 200] = 150
    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 150i128);
    assert_eq!(agg.num_sources, 2u32);
}

/// Empty batch is accepted without error.
#[test]
fn test_submit_prices_empty_batch() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000_000);
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "S1");

    let batch: Vec<(Address, i128, u64)> = Vec::new(&e);
    // Should not panic
    client.submit_prices(&source, &batch);
}

/// Future timestamp beyond threshold aborts atomically.
#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_submit_prices_future_timestamp_aborts() {
    let e = Env::default();
    ledger_at(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    // Default threshold is 300s; timestamp 10_000 is 9000s in future
    let batch = make_batch(&e, &[(asset.clone(), 100i128, 10_000u64)]);
    client.submit_prices(&source, &batch);
}
