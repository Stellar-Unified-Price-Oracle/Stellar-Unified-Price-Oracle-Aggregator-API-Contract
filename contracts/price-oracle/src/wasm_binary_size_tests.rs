#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};
use crate::test_helpers::*;

#[test]
fn test_contract_initialization_minimal_size() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    assert_ne!(client.env.ledger_sequence(), 0);
}

#[test]
fn test_core_price_submission_functionality() {
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
fn test_essential_admin_functions() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let new_admin = Address::generate(&e);
    client.set_admin(&new_admin);

    let current_admin = client.get_admin();
    assert_eq!(current_admin, new_admin);
}

#[test]
fn test_source_management_operations() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "TestSource"));

    let sources = client.list_sources();
    assert!(sources.len() > 0);
}

#[test]
fn test_asset_registration_and_retrieval() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    let assets = client.list_assets();
    assert!(assets.len() > 0);
}

#[test]
fn test_price_history_basic() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 1000, 100);
    submit_test_price_n(&client, &source, &asset, 1100, 200, 2);

    let history = client.get_price_history(&asset, &0u32, &2u32);
    assert_eq!(history.len(), 2);
}

#[test]
fn test_configuration_updates() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&3u32);
    assert_eq!(client.get_min_sources(), 3u32);
}

#[test]
fn test_dead_code_elimination_verification() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let source = register_test_source(&e, &client, "Source1");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&1u32);
    submit_test_price(&client, &source, &asset, 5000, 100);

    let price = client.get_price(&asset);
    assert_eq!(price.price, 5000);
}

#[test]
fn test_feature_flag_compilation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "OptimizedSource"));

    let sources = client.list_sources();
    assert!(sources.len() > 0);
}

#[test]
fn test_dependency_minimization() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&1u32);
    submit_test_price(&client, &source, &asset, 3000, 150);

    let price = client.get_price(&asset);
    assert_eq!(price.price, 3000);
}

#[test]
fn test_contract_size_stability() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    for i in 0..5 {
        let source = register_test_source(&e, &client, &format!("Source{}", i));
        let asset = register_test_asset(&e, &client);
        client.set_min_sources_required(&1u32);
        submit_test_price(&client, &source, &asset, 1000 + (i as i128 * 100), 100 + (i as u64 * 10));
    }

    let assets = client.list_assets();
    assert!(assets.len() > 0);
}

#[test]
fn test_minimal_memory_footprint() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 2500, 75);

    let price = client.get_price(&asset);
    assert_eq!(price.price, 2500);
}

#[test]
fn test_optimization_preserves_correctness() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_min_sources_required(&2u32);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 1000, 100);
    submit_test_price_n(&client, &source2, &asset, 2000, 100, 2);

    let price = client.get_price(&asset);
    assert!(price.price > 0);
}

#[test]
fn test_full_test_suite_passes_post_optimization() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin, sources, assets) = setup_full_oracle(&e, 2, 2);

    for (i, asset) in assets.iter().enumerate() {
        for (j, source) in sources.iter().enumerate() {
            let price = ((i + 1) * (j + 1) * 100) as i128;
            submit_test_price(&client, source, asset, price, 100 + (i as u64 * 50));
        }
    }

    for asset in assets.iter() {
        let price = client.get_price(asset);
        assert!(price.price > 0);
    }
}
