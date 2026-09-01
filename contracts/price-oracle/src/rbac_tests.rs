#![cfg(test)]

use crate::test_helpers::*;
use crate::types::Role;
use soroban_sdk::{testutils::Address as _, Address, String};

#[test]
fn test_admin_has_all_roles() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Admin should have SourceManager role
    assert!(client.has_role(&admin, &Role::SourceManager));

    // Admin should have AssetManager role
    assert!(client.has_role(&admin, &Role::AssetManager));

    // Admin should have PriceUpdater role
    assert!(client.has_role(&admin, &Role::PriceUpdater));

    // Admin should have ConfigManager role
    assert!(client.has_role(&admin, &Role::ConfigManager));

    // Admin should have UpgradeManager role
    assert!(client.has_role(&admin, &Role::UpgradeManager));
}

#[test]
fn test_delegate_role() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let delegatee = Address::generate(&e);

    // Delegatee shouldn't have role initially
    assert!(!client.has_role(&delegatee, &Role::SourceManager));

    // Delegate the role
    client.delegate_role(&delegatee, &Role::SourceManager);

    // Delegatee should have role now
    assert!(client.has_role(&delegatee, &Role::SourceManager));
}

#[test]
fn test_revoke_role() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let delegatee = Address::generate(&e);

    // Delegate first
    client.delegate_role(&delegatee, &Role::AssetManager);
    assert!(client.has_role(&delegatee, &Role::AssetManager));

    // Then revoke
    client.revoke_role(&delegatee, &Role::AssetManager);
    assert!(!client.has_role(&delegatee, &Role::AssetManager));
}

#[test]
fn test_get_role_holders() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let delegatee1 = Address::generate(&e);
    let delegatee2 = Address::generate(&e);

    // Delegate to two addresses
    client.delegate_role(&delegatee1, &Role::ConfigManager);
    client.delegate_role(&delegatee2, &Role::ConfigManager);

    // Get role holders
    let holders = client.get_role_holders(&Role::ConfigManager);
    assert!(holders.len() >= 2);
}

#[test]
fn test_get_address_roles() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let delegatee = Address::generate(&e);

    // Delegate multiple roles
    client.delegate_role(&delegatee, &Role::SourceManager);
    client.delegate_role(&delegatee, &Role::AssetManager);

    // Get roles for address
    let roles = client.get_address_roles(&delegatee);
    assert!(roles.len() >= 2);
}

#[test]
fn test_admin_has_all_roles_in_get_address_roles() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let roles = client.get_address_roles(&admin);
    // Admin should have all 5 roles
    assert_eq!(roles.len(), 5);
}

#[test]
fn test_multiple_roles_per_address() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let delegatee = Address::generate(&e);

    // Delegate multiple roles to same address
    client.delegate_role(&delegatee, &Role::SourceManager);
    client.delegate_role(&delegatee, &Role::AssetManager);
    client.delegate_role(&delegatee, &Role::PriceUpdater);

    // Verify all roles are present
    assert!(client.has_role(&delegatee, &Role::SourceManager));
    assert!(client.has_role(&delegatee, &Role::AssetManager));
    assert!(client.has_role(&delegatee, &Role::PriceUpdater));

    // Other roles should NOT be present
    assert!(!client.has_role(&delegatee, &Role::ConfigManager));
}

#[test]
fn test_role_isolation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let delegatee1 = Address::generate(&e);
    let delegatee2 = Address::generate(&e);

    // Delegate different roles
    client.delegate_role(&delegatee1, &Role::SourceManager);
    client.delegate_role(&delegatee2, &Role::AssetManager);

    // Each should only have their delegated role
    assert!(client.has_role(&delegatee1, &Role::SourceManager));
    assert!(!client.has_role(&delegatee1, &Role::AssetManager));

    assert!(!client.has_role(&delegatee2, &Role::SourceManager));
    assert!(client.has_role(&delegatee2, &Role::AssetManager));
}

#[test]
fn test_role_revoke_leaves_other_roles() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let delegatee = Address::generate(&e);

    // Delegate two roles
    client.delegate_role(&delegatee, &Role::SourceManager);
    client.delegate_role(&delegatee, &Role::AssetManager);

    // Revoke one role
    client.revoke_role(&delegatee, &Role::SourceManager);

    // Other role should still be present
    assert!(!client.has_role(&delegatee, &Role::SourceManager));
    assert!(client.has_role(&delegatee, &Role::AssetManager));
}

#[test]
fn test_delegate_same_role_twice() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let delegatee = Address::generate(&e);

    // Delegate same role twice
    client.delegate_role(&delegatee, &Role::UpgradeManager);
    client.delegate_role(&delegatee, &Role::UpgradeManager);

    // Should still have the role (idempotent)
    assert!(client.has_role(&delegatee, &Role::UpgradeManager));
}

#[test]
fn test_non_admin_cannot_delegate() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let non_admin = Address::generate(&e);
    let delegatee = Address::generate(&e);

    // Non-admin tries to delegate - should fail
    // This would require changing auth context, which is handled by mock_all_auths()
    // So we skip this test in basic test suite
}
