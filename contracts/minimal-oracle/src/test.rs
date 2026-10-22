#![cfg(test)]
use super::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::Env;

fn setup() -> (Env, MinimalOracleClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(MinimalOracle, ());
    let client = MinimalOracleClient::new(&env, &contract_id);
    (env, client)
}

#[test]
fn test_initialize() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin, &18u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn test_double_initialize_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.initialize(&admin, &18u32);
}

#[test]
fn test_add_and_remove_source() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let source = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.add_source(&source);
    client.remove_source(&source);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_add_duplicate_source_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let source = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.add_source(&source);
    client.add_source(&source);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_remove_nonexistent_source_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let source = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.remove_source(&source);
}

#[test]
fn test_submit_and_get_price_single_source() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let source = Address::generate(&env);
    let asset = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.add_source(&source);
    client.submit_price(&source, &asset, &1_000_000i128);
    assert_eq!(client.get_price(&asset), 1_000_000i128);
}

#[test]
fn test_get_price_median_odd() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let asset = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.add_source(&s1);
    client.add_source(&s2);
    client.add_source(&s3);
    client.submit_price(&s1, &asset, &100i128);
    client.submit_price(&s2, &asset, &300i128);
    client.submit_price(&s3, &asset, &200i128);
    // median of [100, 200, 300] = 200
    assert_eq!(client.get_price(&asset), 200i128);
}

#[test]
fn test_get_price_median_even() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let asset = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.add_source(&s1);
    client.add_source(&s2);
    client.submit_price(&s1, &asset, &100i128);
    client.submit_price(&s2, &asset, &200i128);
    // median of [100, 200] = 150
    assert_eq!(client.get_price(&asset), 150i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_get_price_no_data_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let asset = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.get_price(&asset);
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_submit_zero_price_panics() {
    let (env, client) = setup();
    let admin = Address::generate(&env);
    let source = Address::generate(&env);
    let asset = Address::generate(&env);
    client.initialize(&admin, &18u32);
    client.add_source(&source);
    client.submit_price(&source, &asset, &0i128);
}
