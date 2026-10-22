#![cfg(test)]

use soroban_sdk::{Env, String};

use crate::test_helpers::*;

#[test]
fn test_override_price_basic() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Chainlink");
    let asset = register_test_asset(&e, &client);

    // Submit normal price
    submit_test_price(&client, &source, &asset, 1000i128, 1234567890);
    let normal_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(normal_price.price, 1000i128);
    assert!(!normal_price.is_override);

    // Set override
    client.override_price(
        &asset,
        &9999i128,
        &String::from_str(&e, "Emergency override"),
        &200u32,
    );

    // get_price now returns override
    let ovr_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(ovr_price.price, 9999i128);
    assert!(ovr_price.is_override);
    assert_eq!(ovr_price.num_sources, 0u32);
}

#[test]
fn test_get_price_override_query() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    // No override initially
    assert!(client.get_price_override(&asset).is_none());

    // Set override
    client.override_price(
        &asset,
        &5000i128,
        &String::from_str(&e, "Market halt"),
        &500u32,
    );

    let entry = client.get_price_override(&asset).unwrap();
    assert_eq!(entry.price, 5000i128);
    assert_eq!(entry.expiry_ledger, 500u32);
    assert_eq!(entry.set_ledger, 100u32);
    assert_eq!(entry.reason, String::from_str(&e, "Market halt"));
}

#[test]
fn test_remove_price_override() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Chainlink");
    let asset = register_test_asset(&e, &client);

    // Submit normal price
    submit_test_price(&client, &source, &asset, 1000i128, 1234567890);

    // Set then remove override
    client.override_price(
        &asset,
        &9999i128,
        &String::from_str(&e, "Temporary fix"),
        &300u32,
    );
    assert!(client.get_price(&asset, &0u64).unwrap().is_override);

    client.remove_price_override(&asset);

    // Override gone, normal price returns
    assert!(client.get_price_override(&asset).is_none());
    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 1000i128);
    assert!(!price.is_override);
}

#[test]
fn test_override_expires_after_ledger() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Chainlink");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 1000i128, 1234567890);

    // Set override expiring at ledger 150
    client.override_price(&asset, &9999i128, &String::from_str(&e, "Temp"), &150u32);
    assert!(client.get_price(&asset, &0u64).unwrap().is_override);

    // Advance ledger past expiry
    ledger_default(&e, 200, 1234567890);

    // Override expired, falls back to normal price
    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 1000i128);
    assert!(!price.is_override);

    // Override entry cleaned up
    assert!(client.get_price_override(&asset).is_none());
}

#[test]
fn test_original_submissions_preserved_during_override() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Chainlink");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 1000i128, 1234567890);

    client.override_price(
        &asset,
        &9999i128,
        &String::from_str(&e, "Override"),
        &300u32,
    );

    // Override active, but source submission still retrievable
    let entry = client.get_source_price(&asset, &source);
    assert_eq!(entry.price, 1000i128);

    // Source can still submit during override
    submit_test_price(&client, &source, &asset, 2000i128, 1234567890);
    let entry2 = client.get_source_price(&asset, &source);
    assert_eq!(entry2.price, 2000i128);

    // get_price still returns override
    assert!(client.get_price(&asset, &0u64).unwrap().is_override);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_override_price_invalid_zero() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    client.override_price(&asset, &0i128, &String::from_str(&e, "Bad"), &300u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_override_price_expiry_in_past() {
    let e = Env::default();
    ledger_default(&e, 200, 1234567890);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    // expiry_ledger must be > current_ledger (200), so 100 should fail
    client.override_price(&asset, &9999i128, &String::from_str(&e, "Bad"), &100u32);
}

#[test]
fn test_override_price_unauthorized() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    clear_auth(&e);
    assert!(client
        .try_override_price(
            &asset,
            &9999i128,
            &String::from_str(&e, "Unauthorized"),
            &300u32,
        )
        .is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_remove_nonexistent_override() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    // No override set — should panic with NoData (#8)
    client.remove_price_override(&asset);
}

#[test]
fn test_override_replaces_previous_override() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    client.override_price(&asset, &1111i128, &String::from_str(&e, "First"), &300u32);
    client.override_price(&asset, &2222i128, &String::from_str(&e, "Second"), &400u32);

    let entry = client.get_price_override(&asset).unwrap();
    assert_eq!(entry.price, 2222i128);
    assert_eq!(entry.expiry_ledger, 400u32);
}
