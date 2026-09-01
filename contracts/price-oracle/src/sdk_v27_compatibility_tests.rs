#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};
use crate::test_helpers::*;

#[test]
fn test_sdk_v27_contract_initialization() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    assert_ne!(client.env.ledger_sequence(), 0);
}

#[test]
fn test_sdk_v27_price_submission() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 1000, 100);

    let price = client.get_price(&asset);
    assert_eq!(price.price, 1000);
}

#[test]
fn test_sdk_v27_source_management() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "TestSource"));

    let sources = client.list_sources();
    assert!(sources.len() > 0);
}

#[test]
fn test_sdk_v27_asset_registration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    let assets = client.list_assets();
    assert!(assets.len() > 0);
}

#[test]
fn test_sdk_v27_min_sources_configuration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&5u32);
    assert_eq!(client.get_min_sources(), 5u32);
}

#[test]
fn test_sdk_v27_ledger_state_persistence() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&1u32);
    submit_test_price(&client, &source, &asset, 5000, 200);

    let price = client.get_price(&asset);
    assert_eq!(price.price, 5000);
    assert_eq!(price.timestamp, 200);
}

#[test]
fn test_sdk_v27_multiple_assets_concurrent_prices() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin, sources, assets) = setup_full_oracle(&e, 2, 3);

    for (i, asset) in assets.iter().enumerate() {
        for (j, source) in sources.iter().enumerate() {
            let price = ((i + 1) * (j + 1) * 100) as i128;
            submit_test_price(&client, source, asset, price, 300);
        }
    }

    for (i, asset) in assets.iter().enumerate() {
        let price_data = client.get_price(asset);
        assert!(price_data.price > 0);
    }
}

#[test]
fn test_sdk_v27_vector_operations() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let mut sources: Vec<Address> = Vec::new(&e);
    for i in 0..3 {
        let source = Address::generate(&e);
        client.add_source(&source, &String::from_str(&e, &format!("Source{}", i)));
        sources.push_back(source);
    }

    assert_eq!(sources.len(), 3);
}

#[test]
fn test_sdk_v27_string_operations() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let source = Address::generate(&e);
    let name = String::from_str(&e, "TestSource");
    client.add_source(&source, &name);

    let sources = client.list_sources();
    assert!(sources.len() > 0);
}

#[test]
fn test_sdk_v27_timestamp_handling() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 1, 1000000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 2000, 1000000);

    let price = client.get_price(&asset);
    assert_eq!(price.timestamp, 1000000);
}
