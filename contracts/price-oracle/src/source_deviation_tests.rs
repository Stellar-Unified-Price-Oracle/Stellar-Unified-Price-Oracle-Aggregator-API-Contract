#![cfg(test)]
use crate::test_helpers::*;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

#[test]
fn test_set_and_get_per_source_tolerance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, source, _asset) = setup_basic(&env);
    // Set per-source tolerance
    client.set_source_deviation_tolerance(&source, &200u32);
    assert_eq!(client.get_source_deviation_tolerance(&source), Some(200u32));
}

#[test]
fn test_clear_per_source_tolerance() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, source, _asset) = setup_basic(&env);
    client.set_source_deviation_tolerance(&source, &300u32);
    // Clear it
    client.set_source_deviation_tolerance(&source, &0u32);
    assert_eq!(client.get_source_deviation_tolerance(&source), None);
}

#[test]
fn test_effective_tolerance_uses_per_source_when_set() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, source, _asset) = setup_basic(&env);
    client.set_source_deviation_tolerance(&source, &800u32);
    assert_eq!(client.get_source_deviation_tolerance(&source), Some(800u32));
}

#[test]
fn test_effective_tolerance_falls_back_to_global() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, source, _asset) = setup_basic(&env);
    // No per-source tolerance set → None
    assert_eq!(client.get_source_deviation_tolerance(&source), None);
}

#[test]
#[should_panic]
fn test_set_tolerance_invalid_source_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, _source, _asset) = setup_basic(&env);
    let unknown = Address::generate(&env);
    client.set_source_deviation_tolerance(&unknown, &200u32);
}

#[test]
#[should_panic]
fn test_set_tolerance_over_10000_bps_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let (client, _admin, source, _asset) = setup_basic(&env);
    client.set_source_deviation_tolerance(&source, &10_001u32);
}
