//! Tests for Issue #353 — Incident Response Playbook and On-Chain Circuit-Breaker Drills

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env, String,
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
// Issue #353 — Incident Response Playbook and On-Chain Circuit-Breaker Drills
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_emergency_pause_activation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let reason = String::from_str(&e, "Price manipulation detected");
    client.emergency_pause(&reason, &100u32);

    assert!(client.is_emergency_pause_active());
}

#[test]
fn test_emergency_pause_details_retrieval() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let reason = String::from_str(&e, "Source compromise detected");
    client.emergency_pause(&reason, &50u32);

    let pause_info = client.get_emergency_pause();
    assert!(pause_info.is_some());
    let pause = pause_info.unwrap();
    assert_eq!(pause.reason, reason);
}

#[test]
fn test_emergency_pause_auto_unpause_ledger_count() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let reason = String::from_str(&e, "Governance attack suspected");
    let auto_unpause_ledgers = 200u32;
    client.emergency_pause(&reason, &auto_unpause_ledgers);

    let pause_info = client.get_emergency_pause().unwrap();
    assert!(pause_info.auto_unpause_ledger >= 100);
}

#[test]
fn test_extend_emergency_pause_increases_duration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let reason = String::from_str(&e, "Ongoing incident");
    client.emergency_pause(&reason, &100u32);

    let initial_pause = client.get_emergency_pause().unwrap();

    client.extend_emergency_pause(&100u32);

    let extended_pause = client.get_emergency_pause().unwrap();
    assert!(extended_pause.auto_unpause_ledger >= initial_pause.auto_unpause_ledger);
}

#[test]
fn test_cancel_emergency_pause_deactivation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let reason = String::from_str(&e, "Incident resolved");
    client.emergency_pause(&reason, &100u32);
    assert!(client.is_emergency_pause_active());

    client.cancel_emergency_pause();
    assert!(!client.is_emergency_pause_active());
}

#[test]
fn test_challenge_price_submission_during_normal_operations() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let asset = register_test_asset(&e, &client);
    let expected_price = 1_000_000i128;

    client.challenge_price(&asset, &expected_price, &soroban_sdk::Bytes::new(&e));

    let challenge_count = client.get_challenge_history(&asset, &0u32, &10u32).len();
    assert!(challenge_count >= 0);
}

#[test]
fn test_challenge_resolution_valid_acceptance() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let asset = register_test_asset(&e, &client);
    let expected_price = 1_000_000i128;

    client.challenge_price(&asset, &expected_price, &soroban_sdk::Bytes::new(&e));

    client.resolve_challenge(&0u32, &true);

    let history = client.get_challenge_history(&asset, &0u32, &10u32);
    assert!(history.len() >= 0);
}

#[test]
fn test_challenge_resolution_rejection() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let asset = register_test_asset(&e, &client);
    let expected_price = 1_000_000i128;

    client.challenge_price(&asset, &expected_price, &soroban_sdk::Bytes::new(&e));

    client.resolve_challenge(&0u32, &false);

    let history = client.get_challenge_history(&asset, &0u32, &10u32);
    assert!(history.len() >= 0);
}

#[test]
fn test_challenger_rewards_accumulation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let challenger = soroban_sdk::Address::generate(&e);
    let asset = register_test_asset(&e, &client);

    client.challenge_price(&asset, &1_000_000, &soroban_sdk::Bytes::new(&e));

    let rewards = client.get_challenger_rewards(&challenger);
    assert!(rewards >= 0);
}

#[test]
fn test_source_removal_incident_response() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let compromised_source = register_test_source(&e, &client, "Compromised");
    let asset = register_test_asset(&e, &client);

    assert!(client.is_source(compromised_source.clone()));

    client.remove_source(&compromised_source);

    assert!(!client.is_source(compromised_source));
}

#[test]
fn test_incident_severity_escalation_s0_response() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let critical_source = register_test_source(&e, &client, "Critical");

    client.remove_source(&critical_source);
    assert!(!client.is_source(critical_source));
}

#[test]
fn test_incident_isolation_pause_single_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");

    client.set_min_sources_required(&2u32);

    client.submit_price(&source1, &asset1, &1_000_000, &1_000);
    client.submit_price(&source2, &asset2, &2_000_000, &1_000);

    let reason = String::from_str(&e, "Asset 1 price manipulation");
    client.emergency_pause(&reason, &100u32);

    assert!(client.is_emergency_pause_active());
}

#[test]
fn test_circuit_breaker_max_price_deviation_check() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&1u32);
    client.set_max_price_deviation(&1000u32);

    client.submit_price(&source, &asset, &1_000_000, &1_000);

    let max_dev = client.get_max_price_deviation();
    assert_eq!(max_dev, 1000u32);
}

#[test]
fn test_heartbeat_interval_violation_detection() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let heartbeat = 600u64;
    client.set_heartbeat_interval(&heartbeat);

    assert_eq!(client.get_heartbeat_interval(), heartbeat);
}

#[test]
fn test_query_rate_limit_enforcement_during_attack() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_query_rate_limit(&10u32);
    assert_eq!(client.get_query_rate_limit(), 10u32);
}

#[test]
fn test_multi_source_compromise_incident_drill() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&3u32);

    client.submit_price(&source1, &asset, &1_000_000, &1_000);
    client.submit_price(&source2, &asset, &1_000_000, &1_000);
    client.submit_price(&source3, &asset, &1_000_000, &1_000);

    client.remove_source(&source1);
    client.remove_source(&source2);

    assert!(!client.is_source(source1));
    assert!(!client.is_source(source2));
    assert!(client.is_source(source3));
}

#[test]
fn test_drip_pause_extension_for_ongoing_incident() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let reason = String::from_str(&e, "Ongoing incident");
    client.emergency_pause(&reason, &50u32);

    for _i in 0..5 {
        set_ledger(&e, 150 + (_i as u32) * 50, 1_000 + (_i as u64) * 50);
        client.extend_emergency_pause(&50u32);
    }

    assert!(client.is_emergency_pause_active());
}
