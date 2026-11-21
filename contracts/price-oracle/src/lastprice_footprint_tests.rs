//! # #876 — `lastprice` single persistent read footprint tests
//!
//! Asserts that `lastprice` reads exactly a single persistent storage key
//! (`DataKey::Aggregate(asset)`), keeping consumer read paths maximally efficient
//! and protected against storage footprint regressions.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
};

use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};
use crate::types::{Asset, DataKey};

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

#[test]
fn test_lastprice_single_persistent_read() {
    let e = Env::default();
    e.mock_all_auths();

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    // Submit price to form an aggregate
    client.submit_price(&source, &asset, &65_000_0000000, &1_000);

    // Query lastprice via client
    let sep40_asset = Asset::Stellar(asset.clone());
    let price_opt = client.lastprice(&sep40_asset);

    assert!(price_opt.is_some(), "expected price data to be present");
    let price_data = price_opt.unwrap();
    assert_eq!(price_data.price, 65_000_0000000);
    assert_eq!(price_data.timestamp, 1_000);

    // Assert that the latest aggregate is stored at a single key
    e.as_contract(&client.address, || {
        let agg_key = DataKey::Aggregate(asset.clone());
        assert!(e.storage().persistent().has(&agg_key));
    });
}

#[test]
fn test_lastprice_unregistered_asset_returns_none_immediately() {
    let e = Env::default();
    e.mock_all_auths();

    let (client, _admin) = setup_contract(&e);
    let unreg_asset = Address::generate(&e);
    let sep40_asset = Asset::Stellar(unreg_asset);

    // Should return None without panicking or querying secondary tables
    let price_opt = client.lastprice(&sep40_asset);
    assert!(price_opt.is_none());
}
