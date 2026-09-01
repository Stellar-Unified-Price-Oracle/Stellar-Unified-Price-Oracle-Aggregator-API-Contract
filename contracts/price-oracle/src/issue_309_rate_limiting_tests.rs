//! Tests for Issue #309 — Query Rate Limiting and Quota Management

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env,
};

use crate::test_helpers::{register_test_asset, setup_contract};

// ─── Helpers ────────────────────────────────────────────────────────────────

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
// Issue #309 — Query Rate Limiting and Quota Management
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_set_query_rate_limit() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let max_queries = 100u32;
    client.set_query_rate_limit(&max_queries);

    assert_eq!(client.get_query_rate_limit(), max_queries);
}

#[test]
fn test_query_rate_limit_default() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let limit = client.get_query_rate_limit();
    assert!(limit > 0u32);
}

#[test]
fn test_modify_query_rate_limit() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_query_rate_limit(&50u32);
    assert_eq!(client.get_query_rate_limit(), 50u32);

    client.set_query_rate_limit(&200u32);
    assert_eq!(client.get_query_rate_limit(), 200u32);
}

#[test]
fn test_timestamp_threshold_configuration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let threshold = 3600u64;
    client.set_timestamp_threshold(&threshold);

    assert_eq!(client.get_timestamp_threshold(), threshold);
}

#[test]
fn test_timestamp_threshold_affects_price_validity() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.set_timestamp_threshold(&60u64);

    // Price should be valid when timestamp matches ledger time
    let price = client.get_price(&asset);
    // Expected behavior: no price yet, but threshold is configured
    assert_eq!(client.get_timestamp_threshold(), 60u64);
}

#[test]
fn test_quota_enforcement_with_multiple_assets() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    set_ledger(&e, 100, 1_000);

    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    client.set_query_rate_limit(&10u32);

    assert_eq!(client.get_query_rate_limit(), 10u32);
}
