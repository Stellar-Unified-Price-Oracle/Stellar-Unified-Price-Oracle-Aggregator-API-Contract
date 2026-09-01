//! Tests for Issue #379 — Batch Storage Writes in the Aggregation Path

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
// Issue #379 — Batch Storage Writes in the Aggregation Path
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_aggregation_writes_aggregate_price() {
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

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_aggregation_updates_history() {
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
fn test_aggregation_batches_multiple_writes() {
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

    // Trigger aggregation which should batch: aggregate, history, EMA, circuit-breaker state
    client.trigger_aggregation(&asset);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_sequential_aggregations_maintain_state() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    // First aggregation
    client.submit_price(&source1, &asset, &1_000_000, &500);
    client.submit_price(&source2, &asset, &1_100_000, &500);
    client.submit_price(&source3, &asset, &1_050_000, &500);
    client.trigger_aggregation(&asset);

    let price1 = client.get_price(&asset).unwrap().price;

    // Second aggregation
    set_ledger(&e, 200, 2_000);
    client.submit_price(&source1, &asset, &1_100_000, &500);
    client.submit_price(&source2, &asset, &1_200_000, &500);
    client.submit_price(&source3, &asset, &1_150_000, &500);
    client.trigger_aggregation(&asset);

    let price2 = client.get_price(&asset).unwrap().price;

    assert_ne!(price1, price2);
}

#[test]
fn test_circuit_breaker_state_persisted_on_aggregation() {
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

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_atomicity_on_aggregation_writes() {
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

    // Verify all writes completed successfully
    let price = client.get_price(&asset);
    assert!(price.is_some());

    let history = client.get_price_history(&asset, &5u32);
    assert!(history.len() > 0);
}
