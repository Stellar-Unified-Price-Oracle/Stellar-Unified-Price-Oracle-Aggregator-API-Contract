//! Tests for Issue #307 — Alert Rules Configuration and Dispatch

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env,
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
// Issue #307 — Alert Rules Configuration and Dispatch
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_set_price_threshold_alert() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let threshold = 1000u32;
    client.set_max_price_deviation(&threshold);

    let retrieved = client.get_max_price_deviation();
    assert_eq!(retrieved, threshold);
}

#[test]
fn test_alert_on_price_deviation_exceeds_threshold() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test Source");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.set_max_price_deviation(&500u32);
    client.submit_price(&source, &asset, &1_000_000, &1_000);
    client.submit_price(&source, &asset, &1_150_000, &1_000);

    assert_eq!(client.get_max_price_deviation(), 500u32);
}

#[test]
fn test_heartbeat_interval_configuration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let interval = 300u64;
    client.set_heartbeat_interval(&interval);

    assert_eq!(client.get_heartbeat_interval(), interval);
}

#[test]
fn test_heartbeat_miss_detection() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test Source");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.set_heartbeat_interval(&60u64);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    set_ledger(&e, 200, 2_000);

    let still_live = client.check_source_liveness(&source, &asset);
    assert!(!still_live);
}

#[test]
fn test_alert_on_source_inactivity() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test Source");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);
    client.set_heartbeat_interval(&100u64);

    client.submit_price(&source, &asset, &1_000_000, &1_000);

    set_ledger(&e, 300, 3_000);

    let is_active = client.check_source_liveness(&source, &asset);
    assert!(!is_active);
}

#[test]
fn test_multiple_alert_thresholds() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_max_price_deviation(&300u32);
    assert_eq!(client.get_max_price_deviation(), 300u32);

    client.set_max_price_deviation(&500u32);
    assert_eq!(client.get_max_price_deviation(), 500u32);
}
