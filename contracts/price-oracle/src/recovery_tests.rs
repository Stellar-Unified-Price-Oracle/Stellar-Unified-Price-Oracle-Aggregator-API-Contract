#![cfg(test)]

//! # #245 — Admin Key Social Recovery Tests
//!
//! Tests for:
//! - Guardian registration by the current admin
//! - Recovery initiation and N-of-M approval by guardians
//! - Cancellation window (admin can cancel before execution)
//! - Auto-execution after the cancellation-window delay elapses
//! - Misuse: non-guardians, double approval, conflicting candidates, premature execution

use soroban_sdk::{Vec, 
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, Vec,
};

use crate::test_helpers::*;

fn advance_ledger(e: &Env, seq: u32) {
    e.ledger().set(LedgerInfo {
        timestamp: (seq as u64) * 5,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 4_000_000,
    });
}

fn guardians3(e: &Env) -> (Address, Address, Address) {
    (Address::generate(e), Address::generate(e), Address::generate(e))
}

// ---------------------------------------------------------------------------
// Guardian registration
// ---------------------------------------------------------------------------

#[test]
fn test_set_and_get_guardians() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, g3) = guardians3(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    guardians.push_back(g3.clone());

    client.recovery_set_guardians(&guardians, &2u32);

    assert_eq!(client.recovery_get_guardians().len(), 3);
    assert_eq!(client.recovery_get_threshold(), 2u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #108)")]
fn test_set_guardians_zero_threshold_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1);
    guardians.push_back(g2);

    client.recovery_set_guardians(&guardians, &0u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #108)")]
fn test_set_guardians_threshold_exceeds_count_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1);
    guardians.push_back(g2);

    client.recovery_set_guardians(&guardians, &3u32);
}

// ---------------------------------------------------------------------------
// Recovery initiation & N-of-M approval
// ---------------------------------------------------------------------------

#[test]
fn test_recovery_not_ready_before_threshold() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, g3) = guardians3(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    guardians.push_back(g3.clone());
    client.recovery_set_guardians(&guardians, &2u32);

    client.recovery_approve(&g1, &new_admin);

    let pending = client.recovery_get_pending().unwrap();
    assert_eq!(pending.new_admin, new_admin);
    assert_eq!(pending.approvals.len(), 1);
    assert_eq!(pending.ready_ledger, 0);
}

#[test]
fn test_recovery_ready_at_threshold() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, g3) = guardians3(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    guardians.push_back(g3.clone());
    client.recovery_set_guardians(&guardians, &2u32);

    client.recovery_approve(&g1, &new_admin);
    client.recovery_approve(&g2, &new_admin);

    let pending = client.recovery_get_pending().unwrap();
    assert_eq!(pending.approvals.len(), 2);
    assert!(pending.ready_ledger > 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #103)")]
fn test_non_guardian_cannot_approve() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);
    let outsider = Address::generate(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1);
    guardians.push_back(g2);
    client.recovery_set_guardians(&guardians, &2u32);

    client.recovery_approve(&outsider, &new_admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #106)")]
fn test_double_approval_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2);
    client.recovery_set_guardians(&guardians, &2u32);

    client.recovery_approve(&g1, &new_admin);
    client.recovery_approve(&g1, &new_admin);
}

#[test]
#[should_panic(expected = "Error(Contract, #105)")]
fn test_conflicting_candidate_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);
    let candidate_a = Address::generate(&e);
    let candidate_b = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);

    client.recovery_approve(&g1, &candidate_a);
    // g2 proposes a different candidate while one is already pending.
    client.recovery_approve(&g2, &candidate_b);
}

// ---------------------------------------------------------------------------
// Cancellation window
// ---------------------------------------------------------------------------

#[test]
fn test_admin_can_cancel_before_execution() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);

    client.recovery_approve(&g1, &new_admin);
    client.recovery_approve(&g2, &new_admin);
    assert!(client.recovery_get_pending().is_some());

    client.recovery_cancel();
    assert!(client.recovery_get_pending().is_none());
}

#[test]
#[should_panic(expected = "Error(Contract, #104)")]
fn test_cancel_with_nothing_pending_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.recovery_cancel();
}

#[test]
fn test_cancelled_recovery_cannot_be_executed() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);
    client.recovery_set_delay(&10u32);

    advance_ledger(&e, 100);
    client.recovery_approve(&g1, &new_admin);
    client.recovery_approve(&g2, &new_admin);
    client.recovery_cancel();

    advance_ledger(&e, 200);
    let result = client.try_recovery_execute();
    assert!(result.is_err());
}

#[test]
fn test_new_recovery_can_start_after_cancellation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);
    let candidate_a = Address::generate(&e);
    let candidate_b = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);
    client.recovery_set_delay(&10u32);

    client.recovery_approve(&g1, &candidate_a);
    client.recovery_cancel();

    // A fresh recovery for a different candidate should now succeed.
    client.recovery_approve(&g1, &candidate_b);
    let pending = client.recovery_get_pending().unwrap();
    assert_eq!(pending.new_admin, candidate_b);
    assert_ne!(candidate_b, admin);
}

// ---------------------------------------------------------------------------
// Auto-execution after delay
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #107)")]
fn test_execute_before_delay_elapsed_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let (g1, g2, _g3) = guardians3(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);
    client.recovery_set_delay(&100u32);

    advance_ledger(&e, 10);
    client.recovery_approve(&g1, &new_admin);
    client.recovery_approve(&g2, &new_admin);

    // Threshold just reached; delay has not elapsed yet.
    client.recovery_execute();
}

#[test]
#[should_panic(expected = "Error(Contract, #104)")]
fn test_execute_with_nothing_pending_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.recovery_execute();
}

#[test]
fn test_recovery_auto_executes_after_delay() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let (g1, g2, g3) = guardians3(&e);
    let new_admin = Address::generate(&e);

    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    guardians.push_back(g3.clone());
    client.recovery_set_guardians(&guardians, &2u32);
    client.recovery_set_delay(&50u32);

    advance_ledger(&e, 10);
    client.recovery_approve(&g1, &new_admin);
    client.recovery_approve(&g2, &new_admin);

    let pending = client.recovery_get_pending().unwrap();
    let execute_after = pending.ready_ledger + 50;

    // Not yet ready.
    advance_ledger(&e, execute_after - 1);
    assert!(client.try_recovery_execute().is_err());

    // Delay has now elapsed — anyone (a third-party, not admin or a guardian) can
    // trigger execution.
    advance_ledger(&e, execute_after);
    client.recovery_execute();

    assert_eq!(client.get_admin_address(), new_admin);
    assert_ne!(client.get_admin_address(), admin);
    assert!(client.recovery_get_pending().is_none());
}

#[test]
fn test_recovery_delay_default_and_configurable() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    assert_eq!(client.recovery_get_delay(), 17_280u32);

    client.recovery_set_delay(&500u32);
    assert_eq!(client.recovery_get_delay(), 500u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_recovery_delay_zero_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.recovery_set_delay(&0u32);
}
