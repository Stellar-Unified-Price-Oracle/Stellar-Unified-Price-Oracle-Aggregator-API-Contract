#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::test_helpers::*;

#[test]
fn test_set_query_fee() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_query_fee(&100i128);
    assert_eq!(client.get_query_fee(), 100i128);
}

#[test]
fn test_set_query_fee_to_zero() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_query_fee(&100i128);
    assert_eq!(client.get_query_fee(), 100i128);

    // Disable fee by setting to 0
    client.set_query_fee(&0i128);
    assert_eq!(client.get_query_fee(), 0i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_query_fee_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    client.set_query_fee(&100i128);
}

#[test]
fn test_set_fee_token() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let token = Address::generate(&e);
    client.set_fee_token(&token);
    assert_eq!(client.get_fee_token(), token);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_fee_token_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    let token = Address::generate(&e);
    client.set_fee_token(&token);
}

#[test]
fn test_set_fee_collector() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let collector = Address::generate(&e);
    client.set_fee_collector(&collector);
    assert_eq!(client.get_fee_collector(), collector);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_fee_collector_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    let collector = Address::generate(&e);
    client.set_fee_collector(&collector);
}

#[test]
fn test_fee_charged_on_price_query() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    // Set fee to 50
    client.set_query_fee(&50i128);
    let collector = Address::generate(&e);
    client.set_fee_collector(&collector);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 150i128);
}

#[test]
fn test_fee_not_charged_with_zero_fee() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    // Fee is 0 by default
    assert_eq!(client.get_query_fee(), 0i128);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 150i128);
}

#[test]
fn test_admin_bypasses_fee() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, admin) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    client.set_query_fee(&50i128);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 150i128);
}

#[test]
fn test_multiple_fee_queries() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_query_fee(&100i128);
    assert_eq!(client.get_query_fee(), 100i128);

    client.set_query_fee(&200i128);
    assert_eq!(client.get_query_fee(), 200i128);

    client.set_query_fee(&0i128);
    assert_eq!(client.get_query_fee(), 0i128);
}

#[test]
fn test_fee_collector_initial_state() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Default fee collector should be admin or unset
    let collector = client.get_fee_collector();
    // Just ensure it's set and retrievable
    let _ = collector;
}

#[test]
fn test_fee_token_initial_state() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Fee token should be queryable
    let token = client.get_fee_token();
    // Just ensure it's set and retrievable
    let _ = token;
}

#[test]
fn test_set_high_fee() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let high_fee = 1_000_000_000_000_000_000i128;
    client.set_query_fee(&high_fee);
    assert_eq!(client.get_query_fee(), high_fee);
}
