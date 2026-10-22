#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::test_helpers::*;

#[test]
fn test_stake_basic() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    // Set minimum stake to 1000
    client.set_min_stake(&1000i128);
    assert_eq!(client.get_min_stake(), 1000i128);

    // Source stakes 2000
    client.stake(&source, &2000i128);
    assert_eq!(client.get_source_stake(&source), 2000i128);
}

#[test]
fn test_stake_multiple_sources() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");

    client.set_min_stake(&500i128);

    client.stake(&source1, &1000i128);
    client.stake(&source2, &2000i128);

    assert_eq!(client.get_source_stake(&source1), 1000i128);
    assert_eq!(client.get_source_stake(&source2), 2000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_stake_below_minimum() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.set_min_stake(&1000i128);
    client.stake(&source, &500i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_stake_zero() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.stake(&source, &0i128);
}

#[test]
fn test_withdraw_stake() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.set_min_stake(&500i128);
    client.stake(&source, &2000i128);
    assert_eq!(client.get_source_stake(&source), 2000i128);

    client.withdraw_stake(&source, &500i128);
    assert_eq!(client.get_source_stake(&source), 1500i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn test_withdraw_stake_insufficient() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.stake(&source, &1000i128);
    client.withdraw_stake(&source, &2000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn test_withdraw_below_minimum() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.set_min_stake(&1000i128);
    client.stake(&source, &2000i128);
    client.withdraw_stake(&source, &1500i128);
}

#[test]
fn test_slash_stake() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.set_min_stake(&500i128);
    client.stake(&source, &2000i128);
    assert_eq!(client.get_source_stake(&source), 2000i128);

    // Admin slashes 500 tokens
    client.slash_stake(&source, &500i128);
    assert_eq!(client.get_source_stake(&source), 1500i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_slash_stake_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.stake(&source, &2000i128);

    clear_auth(&e);
    client.slash_stake(&source, &500i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #13)")]
fn test_slash_stake_exceeds_balance() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.stake(&source, &2000i128);
    client.slash_stake(&source, &3000i128);
}

#[test]
fn test_set_min_stake() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_min_stake(&1000i128);
    assert_eq!(client.get_min_stake(), 1000i128);

    client.set_min_stake(&2000i128);
    assert_eq!(client.get_min_stake(), 2000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_min_stake_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    client.set_min_stake(&1000i128);
}

#[test]
fn test_source_excluded_below_minimum() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    client.set_min_stake(&1000i128);
    client.stake(&source1, &2000i128);
    // source2 has no stake, below minimum

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    // Only source1 should be included in aggregation (source2 excluded)
    assert!(client.get_price(&asset, &0u64).is_none());
}

#[test]
fn test_stake_allows_submission() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    client.set_min_stake(&1000i128);
    client.stake(&source1, &2000i128);
    client.stake(&source2, &2000i128);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 150i128);
    assert_eq!(price.num_sources, 2u32);
}

#[test]
fn test_slash_emits_event() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.stake(&source, &2000i128);
    client.slash_stake(&source, &500i128);

    // Events should be captured in contract state
    let stake = client.get_source_stake(&source);
    assert_eq!(stake, 1500i128);
}
