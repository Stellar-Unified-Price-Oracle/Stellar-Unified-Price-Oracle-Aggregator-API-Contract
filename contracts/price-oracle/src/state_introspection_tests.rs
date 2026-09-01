#![cfg(test)]

use soroban_sdk::{Address, Env, String};

use crate::test_helpers::*;

#[test]
fn test_state_dump_defaults() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(&admin, &1u32, &50u32, &18u32, &String::from_str(&e, "Test"));

    let dump = client.oracle_state_dump();
    assert_eq!(dump.admin, admin);
    assert_eq!(dump.min_sources_required, 1u32);
    assert_eq!(dump.max_history_length, 50u32);
    assert_eq!(dump.decimals, 18u32);
    assert_eq!(dump.timestamp_threshold, 300u64);
    assert_eq!(dump.max_deviation_bps, 500u32);
}

#[test]
fn test_state_analyze_counts() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(
        &admin,
        &2u32,
        &100u32,
        &6u32,
        &String::from_str(&e, "Analyze"),
    );

    let analysis = client.oracle_state_analyze();
    assert_eq!(analysis.admin, admin);
    assert_eq!(analysis.min_sources_required, 2u32);
    assert_eq!(analysis.max_history_length, 100u32);
    assert_eq!(analysis.decimals, 6u32);
}
