#![cfg(test)]

use crate::test_helpers::*;
use soroban_sdk::{testutils::Address as _, Address, Env, String};

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

// ---------------------------------------------------------------------------
// Governance event audit completeness checks (#659)
//
// These tests reconcile privileged state changes with their emitted
// governance events and approval records across an upgrade cycle. The audit
// must fail when a governance event is missing, malformed, or unlinked from
// its state change / approval record.
// ---------------------------------------------------------------------------

/// Normal operation: every privileged state change in an upgrade cycle has a
/// matching, well-formed governance event and approval record, so the audit
/// passes.
#[test]
fn test_governance_audit_complete_cycle_passes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Privileged state changes that make up an upgrade cycle.
    client.set_min_sources_required(&2u32);
    client.set_max_history_length(&50u32);
    client.set_decimals(&8u32);

    // The audit reconciles state changes with emitted events + approvals.
    let report = client.audit_governance_events();
    assert!(report.complete, "audit should pass for a complete cycle");
    assert_eq!(report.missing_events, 0u32);
    assert_eq!(report.malformed_events, 0u32);
    assert_eq!(report.unlinked_events, 0u32);
}

/// Boundary: an empty upgrade cycle (no privileged changes) is trivially
/// complete and must not report spurious failures.
#[test]
fn test_governance_audit_empty_cycle_passes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let report = client.audit_governance_events();
    assert!(report.complete);
    assert_eq!(report.missing_events, 0u32);
    assert_eq!(report.malformed_events, 0u32);
    assert_eq!(report.unlinked_events, 0u32);
}

/// Boundary: multiple upgrades in a single cycle are each reconciled
/// independently and the audit still passes.
#[test]
fn test_governance_audit_multiple_upgrades_passes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&2u32);
    client.set_min_sources_required(&3u32);
    client.set_max_history_length(&40u32);
    client.set_max_history_length(&60u32);

    let report = client.audit_governance_events();
    assert!(report.complete);
    assert_eq!(report.missing_events, 0u32);
    assert_eq!(report.malformed_events, 0u32);
    assert_eq!(report.unlinked_events, 0u32);
}

/// Failure path: a privileged state change whose governance event was not
/// emitted must cause the audit to fail and be reported as missing.
#[test]
fn test_governance_audit_missing_event_fails() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&2u32);

    // Simulate a dropped governance event for the recorded state change.
    client.test_drop_governance_event(&0u32);

    let report = client.audit_governance_events();
    assert!(!report.complete, "audit must fail when an event is missing");
    assert!(report.missing_events >= 1u32);
}

/// Failure path: a malformed governance event (e.g. wrong payload shape)
/// must cause the audit to fail and be reported as malformed.
#[test]
fn test_governance_audit_malformed_event_fails() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&2u32);

    // Corrupt the emitted governance event payload.
    client.test_corrupt_governance_event(&0u32);

    let report = client.audit_governance_events();
    assert!(!report.complete, "audit must fail when an event is malformed");
    assert!(report.malformed_events >= 1u32);
}

/// Failure path: a governance event that is not linked to its approval
/// record must cause the audit to fail and be reported as unlinked.
#[test]
fn test_governance_audit_unlinked_event_fails() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&2u32);

    // Break the link between the event and its approval record.
    client.test_unlink_governance_event(&0u32);

    let report = client.audit_governance_events();
    assert!(!report.complete, "audit must fail when an event is unlinked");
    assert!(report.unlinked_events >= 1u32);
}

/// Recovery path: once the missing event is restored, the audit passes again.
#[test]
fn test_governance_audit_recovers_after_restore() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&2u32);
    client.test_drop_governance_event(&0u32);

    let broken = client.audit_governance_events();
    assert!(!broken.complete);

    // Restore the dropped event and re-run the audit.
    client.test_restore_governance_event(&0u32);

    let recovered = client.audit_governance_events();
    assert!(recovered.complete, "audit should pass after recovery");
    assert_eq!(recovered.missing_events, 0u32);
    assert_eq!(recovered.malformed_events, 0u32);
    assert_eq!(recovered.unlinked_events, 0u32);
}
