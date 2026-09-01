#![cfg(test)]

use crate::test_helpers::*;
use soroban_sdk::{testutils::Address as _, Address, String};

#[test]
fn test_emergency_pause_activates() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    assert!(!client.is_emergency_pause_active());

    client.emergency_pause(&String::from_str(&e, "Critical incident"), &100u32);

    assert!(client.is_emergency_pause_active());
}

#[test]
fn test_emergency_pause_details() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let reason = String::from_str(&e, "System compromise detected");
    client.emergency_pause(&reason, &200u32);

    let pause_details = client.get_emergency_pause();
    assert!(pause_details.is_some());

    let details = pause_details.unwrap();
    assert_eq!(details.reason, reason);
}

#[test]
fn test_emergency_pause_blocks_price_submissions() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let source = register_test_source(&e, &client, "TestSource");
    let asset = register_test_asset(&e, &client);

    // Pause the contract
    client.emergency_pause(&String::from_str(&e, "Emergency pause"), &100u32);

    // Try to submit price - should fail
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.submit_price(&source, &asset, &1000i128, &0u64);
    }));

    // Should panic due to pause
    assert!(result.is_err());
}

#[test]
fn test_cancel_emergency_pause() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    client.emergency_pause(&String::from_str(&e, "Test pause"), &100u32);
    assert!(client.is_emergency_pause_active());

    client.cancel_emergency_pause();
    assert!(!client.is_emergency_pause_active());
}

#[test]
fn test_extend_emergency_pause() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    client.emergency_pause(&String::from_str(&e, "Test pause"), &100u32);

    let details_before = client.get_emergency_pause().unwrap();
    let unpause_before = details_before.auto_unpause_ledger;

    client.extend_emergency_pause(&50u32);

    let details_after = client.get_emergency_pause().unwrap();
    let unpause_after = details_after.auto_unpause_ledger;

    // Unpause ledger should be extended
    assert!(unpause_after > unpause_before);
}

#[test]
fn test_emergency_pause_auto_unpause_timeout() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let source = register_test_source(&e, &client, "TestSource");
    let asset = register_test_asset(&e, &client);

    // Emergency pause with very short timeout
    client.emergency_pause(&String::from_str(&e, "Quick pause"), &1u32);

    assert!(client.is_emergency_pause_active());

    // After enough ledgers pass, it should auto-unpause
    // (In a real ledger, this would be checked on the next call)
}

#[test]
fn test_emergency_pause_multiple_extensions() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    client.emergency_pause(&String::from_str(&e, "Pause"), &100u32);

    let initial = client.get_emergency_pause().unwrap().auto_unpause_ledger;

    client.extend_emergency_pause(&50u32);
    let after_first = client.get_emergency_pause().unwrap().auto_unpause_ledger;
    assert!(after_first > initial);

    client.extend_emergency_pause(&50u32);
    let after_second = client.get_emergency_pause().unwrap().auto_unpause_ledger;
    assert!(after_second > after_first);
}

#[test]
fn test_emergency_pause_cancels_immediately() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    client.emergency_pause(&String::from_str(&e, "Pause"), &1000u32);

    // Should be paused
    assert!(client.is_emergency_pause_active());

    // Cancel immediately
    client.cancel_emergency_pause();

    // Should be unpaused (bypass the timeout)
    assert!(!client.is_emergency_pause_active());
}

#[test]
fn test_emergency_pause_reason_stored() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let reason = String::from_str(&e, "Database connectivity failure");
    client.emergency_pause(&reason, &500u32);

    let details = client.get_emergency_pause().unwrap();
    assert_eq!(details.reason, reason);
}

#[test]
fn test_emergency_pause_initiated_by_admin() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    client.emergency_pause(&String::from_str(&e, "Pause"), &100u32);

    let details = client.get_emergency_pause().unwrap();
    assert_eq!(details.initiated_by, admin);
}

#[test]
fn test_get_emergency_pause_when_inactive() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // When no emergency pause is active, should return None
    let details = client.get_emergency_pause();
    assert!(details.is_none());
}

#[test]
fn test_emergency_pause_ledger_recorded() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    client.emergency_pause(&String::from_str(&e, "Pause"), &100u32);

    let details = client.get_emergency_pause().unwrap();

    // Should have recorded the ledger
    assert!(details.initiated_ledger >= 0);
    assert!(details.auto_unpause_ledger > details.initiated_ledger);
}

#[test]
fn test_emergency_pause_bypass_timelock() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Emergency pause should bypass timelock - activate immediately
    // This is tested indirectly by the fact that we can pause and unpause
    // without waiting for a timelock delay

    client.emergency_pause(&String::from_str(&e, "Immediate pause"), &100u32);
    assert!(client.is_emergency_pause_active());

    // Should be able to cancel immediately too (no timelock)
    client.cancel_emergency_pause();
    assert!(!client.is_emergency_pause_active());
}
