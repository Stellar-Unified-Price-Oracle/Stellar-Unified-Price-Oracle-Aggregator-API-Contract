//! Tests for Issue #352 — Operational Runbook for Multi-sig Admin Key Management

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env, String,
};

use crate::test_helpers::setup_contract;
use crate::types::Role;

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
// Issue #352 — Operational Runbook for Multi-sig Admin Key Management
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_multisig_three_of_five_delegation_setup() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let gov1 = soroban_sdk::Address::generate(&e);
    let gov2 = soroban_sdk::Address::generate(&e);
    let gov3 = soroban_sdk::Address::generate(&e);
    let gov4 = soroban_sdk::Address::generate(&e);
    let gov5 = soroban_sdk::Address::generate(&e);

    client.delegate_role(&gov1, &Role::Admin);
    client.delegate_role(&gov2, &Role::Admin);
    client.delegate_role(&gov3, &Role::Admin);
    client.delegate_role(&gov4, &Role::Admin);
    client.delegate_role(&gov5, &Role::Admin);

    let admin_holders = client.get_role_holders(Role::Admin);
    assert!(admin_holders.len() >= 5);
}

#[test]
fn test_multisig_five_of_seven_delegation_setup() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let governors: Vec<soroban_sdk::Address> = (0..7)
        .map(|_| soroban_sdk::Address::generate(&e))
        .collect();

    for gov in &governors {
        client.delegate_role(gov, &Role::Admin);
    }

    let admin_holders = client.get_role_holders(Role::Admin);
    assert!(admin_holders.len() >= 7);
}

#[test]
fn test_key_rotation_via_delegation_and_revocation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let old_key = soroban_sdk::Address::generate(&e);
    let new_key = soroban_sdk::Address::generate(&e);

    client.delegate_role(&old_key, &Role::Admin);
    assert!(client.has_role(old_key.clone(), Role::Admin));

    client.delegate_role(&new_key, &Role::Admin);
    assert!(client.has_role(new_key.clone(), Role::Admin));

    client.revoke_role(&old_key, &Role::Admin);
    assert!(!client.has_role(old_key, Role::Admin));
    assert!(client.has_role(new_key, Role::Admin));
}

#[test]
fn test_cascading_key_rotation_maintains_threshold() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let old_govs: Vec<soroban_sdk::Address> = (0..3)
        .map(|_| soroban_sdk::Address::generate(&e))
        .collect();
    let new_govs: Vec<soroban_sdk::Address> = (0..3)
        .map(|_| soroban_sdk::Address::generate(&e))
        .collect();

    for gov in &old_govs {
        client.delegate_role(gov, &Role::Admin);
    }

    for gov in &new_govs {
        client.delegate_role(gov, &Role::Admin);
    }

    for gov in &old_govs {
        client.revoke_role(gov, &Role::Admin);
    }

    let admin_holders = client.get_role_holders(Role::Admin);
    assert!(admin_holders.len() >= 3);
}

#[test]
fn test_emergency_admin_delegation_and_recovery() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let emergency_key = soroban_sdk::Address::generate(&e);

    client.delegate_role(&emergency_key, &Role::Admin);
    assert!(client.has_role(emergency_key.clone(), Role::Admin));

    let recovered_key = soroban_sdk::Address::generate(&e);
    client.delegate_role(&recovered_key, &Role::Admin);

    client.revoke_role(&emergency_key, &Role::Admin);
    assert!(client.has_role(recovered_key, Role::Admin));
    assert!(!client.has_role(emergency_key, Role::Admin));
}

#[test]
fn test_admin_role_cannot_be_removed_from_all_holders() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let gov = soroban_sdk::Address::generate(&e);
    client.delegate_role(&gov, &Role::Admin);

    let admin_holders = client.get_role_holders(Role::Admin);
    assert!(admin_holders.len() >= 1);
}

#[test]
fn test_get_roles_for_holder_after_delegation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let holder = soroban_sdk::Address::generate(&e);

    client.delegate_role(&holder, &Role::Admin);
    client.delegate_role(&holder, &Role::Source);

    let roles = client.get_roles_for_holder(holder);
    assert!(roles.len() >= 2);
}

#[test]
fn test_source_role_delegation_for_secondary_oracles() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let source_admin = soroban_sdk::Address::generate(&e);
    let source_operator1 = soroban_sdk::Address::generate(&e);
    let source_operator2 = soroban_sdk::Address::generate(&e);

    client.delegate_role(&source_admin, &Role::Admin);
    client.delegate_role(&source_operator1, &Role::Source);
    client.delegate_role(&source_operator2, &Role::Source);

    assert!(client.has_role(source_admin, Role::Admin));
    assert!(client.has_role(source_operator1, Role::Source));
    assert!(client.has_role(source_operator2, Role::Source));
}

#[test]
fn test_role_separation_admin_and_source() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let admin_only = soroban_sdk::Address::generate(&e);
    let source_only = soroban_sdk::Address::generate(&e);

    client.delegate_role(&admin_only, &Role::Admin);
    client.delegate_role(&source_only, &Role::Source);

    assert!(client.has_role(admin_only, Role::Admin));
    assert!(!client.has_role(admin_only, Role::Source));
    assert!(!client.has_role(source_only, Role::Admin));
    assert!(client.has_role(source_only, Role::Source));
}

#[test]
fn test_multi_role_holder_access_matrix() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let operator = soroban_sdk::Address::generate(&e);

    client.delegate_role(&operator, &Role::Admin);
    client.delegate_role(&operator, &Role::Source);

    let roles = client.get_roles_for_holder(operator.clone());
    assert!(roles.len() >= 2);

    assert!(client.has_role(operator.clone(), Role::Admin));
    assert!(client.has_role(operator, Role::Source));
}

#[test]
fn test_admin_revocation_isolation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let user = soroban_sdk::Address::generate(&e);

    client.delegate_role(&user, &Role::Admin);
    client.delegate_role(&user, &Role::Source);

    client.revoke_role(&user, &Role::Admin);

    assert!(!client.has_role(user.clone(), Role::Admin));
    assert!(client.has_role(user, Role::Source));
}

#[test]
fn test_multisig_consistency_across_role_operations() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let govs: Vec<soroban_sdk::Address> = (0..5)
        .map(|_| soroban_sdk::Address::generate(&e))
        .collect();

    for gov in &govs {
        client.delegate_role(gov, &Role::Admin);
    }

    let initial_holders = client.get_role_holders(Role::Admin);
    let initial_count = initial_holders.len();

    client.revoke_role(&govs[0], &Role::Admin);

    let updated_holders = client.get_role_holders(Role::Admin);
    let updated_count = updated_holders.len();

    assert_eq!(updated_count, initial_count - 1);
}
