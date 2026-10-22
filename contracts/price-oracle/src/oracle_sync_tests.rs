#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String, Vec,
};

use crate::test_helpers::*;

#[test]
fn test_secondary_oracle_registration() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Primary Source");

    let secondary_oracle = Address::generate(&e);

    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 1, 1000);

    client.set_min_sources_required(&1u32);
    submit_test_price(&client, &source, &asset, 15000, 1000);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 15000);
}

#[test]
fn test_price_push_to_secondaries_on_aggregation() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&2u32);

    submit_test_price_n(&client, &source1, &asset, 50000, 1000, 1);
    submit_test_price_n(&client, &source2, &asset, 50200, 1000, 1);

    let aggregated_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(aggregated_price.price, 50100);
}

#[test]
fn test_sync_verification_secondary_confirms_receipt() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Sync Source");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 75000, 1000, 1);

    let price1 = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price1.price, 75000);

    ledger_default(&e, 5, 2000);

    submit_test_price_n(&client, &source, &asset, 76000, 2000, 2);

    let price2 = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price2.price, 76000);
}

#[test]
fn test_multiple_secondary_oracle_sync() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Multi Sync 1");
    let source2 = register_test_source(&e, &client, "Multi Sync 2");
    let source3 = register_test_source(&e, &client, "Multi Sync 3");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&3u32);

    submit_test_price_n(&client, &source1, &asset, 30000, 1000, 1);
    submit_test_price_n(&client, &source2, &asset, 30100, 1000, 1);
    submit_test_price_n(&client, &source3, &asset, 30050, 1000, 1);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 30050);
}

#[test]
fn test_cross_contract_sync_mechanism() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Cross Contract");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 45000, 1000, 1);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 45000);

    ledger_default(&e, 10, 5000);

    submit_test_price_n(&client, &source, &asset, 46000, 5000, 2);

    let new_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(new_price.price, 46000);
}
