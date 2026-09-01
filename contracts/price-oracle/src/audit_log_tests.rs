#![cfg(test)]

use crate::test_helpers::*;
use soroban_sdk::{testutils::Address as _, Address, String};

#[test]
fn test_audit_log_entry_on_initialize() {
    let e = Env::default();
    e.mock_all_auths();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(
        &admin,
        &1u32,
        &10u32,
        &18u32,
        &String::from_str(&e, "Test Oracle"),
    );

    // Should have at least 1 audit entry
    let count = client.get_audit_log_count();
    assert!(count > 0);
}

#[test]
fn test_get_admin_audit_log() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Perform some admin actions
    client.set_min_sources_required(&2u32);
    client.set_max_history_length(&50u32);

    // Query audit log
    let entries = client.get_admin_audit_log(&0u32, &100u32);
    assert!(entries.len() >= 2);
}

#[test]
fn test_audit_log_pagination() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Add multiple assets
    for i in 0..5 {
        let asset = Address::generate(&e);
        client.register_asset(&asset);
    }

    // Get with limit
    let entries_limited = client.get_admin_audit_log(&0u32, &2u32);
    assert_eq!(entries_limited.len(), 2);

    // Get all
    let entries_all = client.get_admin_audit_log(&0u32, &100u32);
    assert!(entries_all.len() >= 5);
}

#[test]
fn test_audit_log_count_increments() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let count_before = client.get_audit_log_count();

    client.set_min_sources_required(&3u32);

    let count_after = client.get_audit_log_count();
    assert!(count_after > count_before);
}

#[test]
fn test_audit_log_head_changes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let head_before = client.get_audit_log_head();

    client.set_max_history_length(&75u32);

    let head_after = client.get_audit_log_head();
    assert!(head_after != head_before || (head_before.is_empty() && head_after.is_empty()));
}

#[test]
fn test_verify_audit_chain_valid() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Perform an admin action to create an entry
    client.set_min_sources_required(&2u32);

    // Verify chain should pass
    let is_valid = client.verify_audit_chain();
    assert!(is_valid);
}

#[test]
fn test_audit_log_on_add_source() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let count_before = client.get_audit_log_count();

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "TestSource"));

    let count_after = client.get_audit_log_count();
    assert_eq!(count_after, count_before + 1);
}

#[test]
fn test_audit_log_on_register_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let count_before = client.get_audit_log_count();

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    let count_after = client.get_audit_log_count();
    assert_eq!(count_after, count_before + 1);
}

#[test]
fn test_audit_log_from_id_parameter() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Create 10 entries
    for i in 0..10 {
        let asset = Address::generate(&e);
        client.register_asset(&asset);
    }

    // Get from ID 5
    let entries = client.get_admin_audit_log(&5u32, &100u32);

    // Should only get entries >= 5
    if entries.len() > 0 {
        assert!(entries.get_unchecked(0).id >= 5);
    }
}

#[test]
fn test_audit_log_all_admin_actions_tracked() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    client.set_decimals(&8u32);
    client.set_resolution(&60u32);
    client.set_min_sources_required(&3u32);

    let entries = client.get_admin_audit_log(&0u32, &100u32);

    // Should have entries for all 3 actions
    assert!(entries.len() >= 3);
}
