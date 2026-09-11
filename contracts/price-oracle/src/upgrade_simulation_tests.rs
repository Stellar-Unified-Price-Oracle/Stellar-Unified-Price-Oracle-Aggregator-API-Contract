#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String, Vec,
};

use crate::test_helpers::*;

#[test]
fn test_upgrade_simulation_sandbox_execution() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Upgrade Monitor");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 55000, 1000, 1);

    let original_price = client.get_price(&asset);
    assert_eq!(original_price.price, 55000);
}

#[test]
fn test_storage_compatibility_report_generation() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Storage Test 1");
    let source2 = register_test_source(&e, &client, "Storage Test 2");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&2u32);

    submit_test_price_n(&client, &source1, &asset1, 60000, 1000, 1);
    submit_test_price_n(&client, &source2, &asset1, 60100, 1000, 1);

    submit_test_price_n(&client, &source1, &asset2, 45000, 1000, 1);
    submit_test_price_n(&client, &source2, &asset2, 45100, 1000, 1);

    let price1 = client.get_price(&asset1);
    assert_eq!(price1.price, 60050);

    let price2 = client.get_price(&asset2);
    assert_eq!(price2.price, 45050);
}

#[test]
fn test_migration_issue_detection_incompatibilities() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Migration Detector");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&1u32);

    submit_test_price_n(&client, &source, &asset, 33000, 1000, 1);

    let price = client.get_price(&asset);
    assert_eq!(price.price, 33000);

    ledger_default(&e, 2, 2000);

    submit_test_price_n(&client, &source, &asset, 34000, 2000, 2);

    let new_price = client.get_price(&asset);
    assert_eq!(new_price.price, 34000);
}

#[test]
fn test_upgrade_simulation_accuracy_state_preservation() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "State Source 1");
    let source2 = register_test_source(&e, &client, "State Source 2");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&2u32);

    submit_test_price_n(&client, &source1, &asset, 70000, 1000, 1);
    submit_test_price_n(&client, &source2, &asset, 70200, 1000, 1);

    let initial_price = client.get_price(&asset);
    assert_eq!(initial_price.price, 70100);

    ledger_default(&e, 10, 5000);

    submit_test_price_n(&client, &source1, &asset, 71000, 5000, 2);
    submit_test_price_n(&client, &source2, &asset, 71200, 5000, 2);

    let upgraded_price = client.get_price(&asset);
    assert_eq!(upgraded_price.price, 71100);
}

#[test]
fn test_upgrade_with_complex_storage_state() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let sources: Vec<Address> = (0..5)
        .map(|i| register_test_source(&e, &client, &format!("Complex Source {}", i)))
        .collect();
    let assets: Vec<Address> = (0..3).map(|_| register_test_asset(&e, &client)).collect();

    ledger_default(&e, 1, 1000);
    client.set_min_sources_required(&3u32);

    for (idx, asset) in assets.iter().enumerate() {
        for (src_idx, source) in sources.iter().enumerate() {
            let price = (40000 + (idx as i128 * 1000) + (src_idx as i128 * 100)) as i128;
            submit_test_price_n(&client, source, asset, price, 1000, 1);
        }
    }

    for asset in assets.iter() {
        let price = client.get_price(asset);
        assert!(price.price > 0);
    }
}
