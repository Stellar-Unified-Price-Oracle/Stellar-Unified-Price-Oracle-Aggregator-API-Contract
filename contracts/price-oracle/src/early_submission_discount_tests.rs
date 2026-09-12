#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
};

use crate::test_helpers::*;

#[test]
fn test_early_submission_detection_first_quartile() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Early Submitter");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 12000, 1000, 1);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 12000);
}

#[test]
fn test_discount_calculation_and_application() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let early_source = register_test_source(&e, &client, "Early Source");
    let late_source = register_test_source(&e, &client, "Late Source");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&2u32);

    submit_test_price_n(&client, &early_source, &asset, 20000, 1000, 1);
    submit_test_price_n(&client, &late_source, &asset, 20100, 1050, 1);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 20050);
}

#[test]
fn test_discount_tracking_per_source() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Tracked Source");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 18000, 1000, 1);

    let price1 = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price1.price, 18000);

    ledger_default(&e, 2, 2000);

    submit_test_price_n(&client, &source, &asset, 19000, 2000, 2);

    let price2 = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price2.price, 19000);
}

#[test]
fn test_earliness_based_discount_scaling() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Very Early");
    let source2 = register_test_source(&e, &client, "Moderately Early");
    let source3 = register_test_source(&e, &client, "On Time");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&3u32);

    submit_test_price_n(&client, &source1, &asset, 22000, 1000, 1);
    submit_test_price_n(&client, &source2, &asset, 22050, 1025, 1);
    submit_test_price_n(&client, &source3, &asset, 22100, 1100, 1);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 22050);
}

#[test]
fn test_multiple_window_submissions_with_discount() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Window Tracker");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 35000, 1000, 1);
    let price1 = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price1.price, 35000);

    ledger_default(&e, 2, 2000);

    submit_test_price_n(&client, &source, &asset, 35500, 2000, 2);
    let price2 = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price2.price, 35500);

    ledger_default(&e, 3, 3000);

    submit_test_price_n(&client, &source, &asset, 36000, 3000, 3);
    let price3 = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price3.price, 36000);
}
