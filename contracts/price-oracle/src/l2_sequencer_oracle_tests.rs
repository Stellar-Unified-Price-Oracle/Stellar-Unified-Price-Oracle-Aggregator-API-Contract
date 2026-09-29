#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
};

use crate::test_helpers::*;

#[test]
fn test_l2_sequencer_health_check_integration() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Sequencer Monitor");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price(&client, &source, &asset, 10000, 1000);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 10000);
}

#[test]
fn test_l2_sequencer_downtime_price_freeze() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Sequencer Monitor");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 50000, 1000, 1);

    let first_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(first_price.price, 50000);

    ledger_default(&e, 10, 2000);

    let frozen_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(frozen_price.price, 50000);
}

#[test]
fn test_l2_sequencer_recovery_and_unfreeze() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Sequencer Monitor");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 30000, 1000, 1);
    let initial_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(initial_price.price, 30000);

    ledger_default(&e, 5, 1500);

    submit_test_price_n(&client, &source, &asset, 31000, 1500, 2);
    let updated_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(updated_price.price, 31000);
}

#[test]
fn test_l2_sequencer_status_transitions() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Monitor 1");
    let source2 = register_test_source(&e, &client, "Monitor 2");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&2u32);

    submit_test_price_n(&client, &source1, &asset, 25000, 1000, 1);
    submit_test_price_n(&client, &source2, &asset, 25100, 1000, 1);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 25050);

    ledger_default(&e, 10, 2000);

    submit_test_price_n(&client, &source1, &asset, 26000, 2000, 2);
    submit_test_price_n(&client, &source2, &asset, 26100, 2000, 2);

    let new_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(new_price.price, 26050);
}
