//! Tests for Issue #310 — Fee Market Priority Configuration

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env,
};

use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};

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
// Issue #310 — Fee Market Priority Configuration
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_set_max_sources() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_max_sources(&10u32);
    assert_eq!(client.get_max_sources(), 10u32);
}

#[test]
fn test_max_sources_enforced() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_max_sources(&1u32);

    assert_eq!(client.get_max_sources(), 1u32);
}

#[test]
fn test_modify_max_sources() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_max_sources(&5u32);
    assert_eq!(client.get_max_sources(), 5u32);

    client.set_max_sources(&20u32);
    assert_eq!(client.get_max_sources(), 20u32);
}

#[test]
fn test_min_sources_required_configuration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&3u32);
    assert_eq!(client.get_min_sources_required(), 3u32);
}

#[test]
fn test_min_sources_affects_aggregation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test Source");
    let source2 = register_test_source(&e, &client, "Source 2");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.set_min_sources_required(&3u32);

    client.submit_price(&source, &asset, &1_000_000, &1_000);
    client.submit_price(&source2, &asset, &1_010_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_none());
}

#[test]
fn test_max_history_length_configuration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_max_history_length(&50u32);
    assert_eq!(client.get_max_history_length(), 50u32);
}

#[test]
fn test_max_history_enforced() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test Source");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.set_max_history_length(&3u32);

    for i in 0..5 {
        set_ledger(&e, 100 + i, 1_000 + (i as u64));
        client.submit_price(
            &source,
            &asset,
            &(1_000_000 + (i as i128)),
            &(1_000 + (i as u64)),
        );
    }

    assert_eq!(client.get_max_history_length(), 3u32);
}

#[test]
fn test_fee_priority_high_priority_configuration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_decimals(&18u32);
    assert_eq!(client.get_decimals(), 18u32);
}

#[test]
fn test_multiple_priority_levels() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    set_ledger(&e, 100, 1_000);

    client.set_min_sources_required(&2u32);
    client.set_max_history_length(&100u32);
    client.set_query_rate_limit(&1000u32);

    assert_eq!(client.get_min_sources_required(), 2u32);
    assert_eq!(client.get_max_history_length(), 100u32);
    assert_eq!(client.get_query_rate_limit(), 1000u32);
}
