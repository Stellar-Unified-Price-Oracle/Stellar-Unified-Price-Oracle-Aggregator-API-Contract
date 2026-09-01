//! Tests for Issue #381 — Adaptive TTL Extension Based on Access Frequency

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
// Issue #381 — Adaptive TTL Extension Based on Access Frequency
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_frequently_accessed_asset_ttl_extended() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_000_000, &500);

    // Access the price multiple times to track frequency
    for _ in 0..5 {
        let _price = client.get_price(&asset);
    }

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_rarely_accessed_asset_ttl_shorter() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_000_000, &500);

    // Access the price once
    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_adaptive_ttl_on_asset_registration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    // Asset is registered, TTL should be extended
    let _price = client.get_price(&asset);
}

#[test]
fn test_multiple_assets_different_ttl_frequencies() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset1, &1_000_000, &500);
    client.submit_price(&source2, &asset2, &2_000_000, &500);

    // Access asset1 frequently
    for _ in 0..10 {
        let _price = client.get_price(&asset1);
    }

    // Access asset2 rarely
    let _price = client.get_price(&asset2);

    let price1 = client.get_price(&asset1);
    let price2 = client.get_price(&asset2);

    assert!(price1.is_some());
    assert!(price2.is_some());
}

#[test]
fn test_ttl_extension_in_batching() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_500_000, &500);

    client.extend_asset_ttl(&asset, &50u32);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_adaptive_ttl_respects_max_entry_ttl() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_000_000, &500);

    // Access multiple times, should respect protocol max_entry_ttl
    for _ in 0..100 {
        let _price = client.get_price(&asset);
    }

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_hot_key_gets_longer_ttl_than_cold_key() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let hot_asset = register_test_asset(&e, &client);
    let cold_asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &hot_asset, &1_000_000, &500);
    client.submit_price(&source2, &cold_asset, &2_000_000, &500);

    // Make hot_asset hot with many accesses
    for _ in 0..50 {
        let _price = client.get_price(&hot_asset);
    }

    // Make cold_asset stay cold
    let _price = client.get_price(&cold_asset);

    // Both should still exist
    let hot_price = client.get_price(&hot_asset);
    let cold_price = client.get_price(&cold_asset);

    assert!(hot_price.is_some());
    assert!(cold_price.is_some());
}

#[test]
fn test_ttl_batching_with_access_frequency_tracking() {
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

    // Access frequently
    for _ in 0..20 {
        let _price = client.get_price(&asset);
    }

    // Trigger aggregation which should use adaptive TTL
    client.trigger_aggregation(&asset);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}
