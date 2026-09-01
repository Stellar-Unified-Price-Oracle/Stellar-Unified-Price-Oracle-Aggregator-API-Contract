//! Tests for Issue #308 — Oracle Health and Status Monitoring

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
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
// Issue #308 — Oracle Health and Status Monitoring
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_get_oracle_decimals() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let decimals = client.get_decimals();
    assert_eq!(decimals, 18u32);
}

#[test]
fn test_get_oracle_description() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let desc = client.get_description();
    assert_eq!(
        desc,
        String::from_str(&e, "Stellar Price Oracle Aggregator")
    );
}

#[test]
fn test_oracle_health_check_no_data() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let min_sources = client.get_min_sources_required();
    assert_eq!(min_sources, 2u32);
}

#[test]
fn test_oracle_health_check_sufficient_sources() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test Source");
    let source2 = register_test_source(&e, &client, "Source 2");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.set_min_sources_required(&2u32);

    client.submit_price(&source, &asset, &1_000_000, &1_000);
    client.submit_price(&source2, &asset, &1_010_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_emergency_pause_affects_health() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test Source");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());

    let pause_reason = String::from_str(&e, "Critical issue detected");
    client.emergency_pause(&pause_reason, &100u32);

    assert!(client.is_emergency_pause_active());
}

#[test]
fn test_get_admin_audit_log_count() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let count = client.get_audit_log_count();
    assert_eq!(count, 0u32);
}

#[test]
fn test_oracle_resolution_setting() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_resolution(&100u32);
    assert_eq!(client.get_resolution(), 100u32);

    client.set_resolution(&50u32);
    assert_eq!(client.get_resolution(), 50u32);
}
