#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};
use crate::test_helpers::*;

#[test]
fn test_external_governor_delegation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);

    let stored_governor = client.get_external_governor();
    assert_eq!(stored_governor, governor);
}

#[test]
fn test_governor_allow_list_configuration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);

    let allowed_ops = vec!["update_sources", "update_assets"];
    for op in allowed_ops {
        client.allow_governor_op(&String::from_str(&e, op));
    }

    assert!(client.is_governor_op_allowed(&String::from_str(&e, "update_sources")));
    assert!(client.is_governor_op_allowed(&String::from_str(&e, "update_assets")));
}

#[test]
fn test_governor_disallowed_operation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);

    client.allow_governor_op(&String::from_str(&e, "update_sources"));

    assert!(!client.is_governor_op_allowed(&String::from_str(&e, "update_admin")));
}

#[test]
fn test_governor_authentication_on_call() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);
    client.allow_governor_op(&String::from_str(&e, "update_sources"));

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "GovernorSource"));

    let sources = client.list_sources();
    assert!(sources.len() > 0);
}

#[test]
fn test_governor_remove_from_allow_list() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);

    let op = String::from_str(&e, "update_sources");
    client.allow_governor_op(&op);
    assert!(client.is_governor_op_allowed(&op));

    client.disallow_governor_op(&op);
    assert!(!client.is_governor_op_allowed(&op));
}

#[test]
fn test_governor_multiple_allowed_operations() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);

    let ops = vec![
        "update_sources",
        "update_assets",
        "set_min_sources",
    ];

    for op_str in ops.iter() {
        client.allow_governor_op(&String::from_str(&e, op_str));
    }

    for op_str in ops.iter() {
        assert!(client.is_governor_op_allowed(&String::from_str(&e, op_str)));
    }
}

#[test]
fn test_admin_still_has_full_access() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);

    let new_admin = Address::generate(&e);
    client.set_admin(&new_admin);

    let stored_admin = client.get_admin();
    assert_eq!(stored_admin, new_admin);
}

#[test]
fn test_governor_clear_delegation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);
    assert_eq!(client.get_external_governor(), governor);

    let zero_addr = Address::generate(&e);
    client.clear_external_governor();

    let stored = client.get_external_governor();
    assert_ne!(stored, governor);
}

#[test]
fn test_governor_operation_auth_failure() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor = Address::generate(&e);
    client.set_external_governor(&governor);
    client.allow_governor_op(&String::from_str(&e, "update_sources"));

    clear_auth(&e);

    let source = Address::generate(&e);
    let result = std::panic::catch_unwind(|| {
        client.add_source(&source, &String::from_str(&e, "UnauthorizedSource"));
    });

    assert!(result.is_err());
}

#[test]
fn test_governor_multiple_governors_delegation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let governor1 = Address::generate(&e);
    client.set_external_governor(&governor1);
    assert_eq!(client.get_external_governor(), governor1);

    let governor2 = Address::generate(&e);
    client.set_external_governor(&governor2);
    assert_eq!(client.get_external_governor(), governor2);
}
