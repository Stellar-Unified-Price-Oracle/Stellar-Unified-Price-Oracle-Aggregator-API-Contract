#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, String, Symbol, Vec};

use crate::test_helpers::*;
use crate::{Asset, PriceData, PriceEntry};

#[test]
fn test_initialize() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(
        &admin,
        &2u32,
        &10u32,
        &18u32,
        &String::from_str(&e, "Stellar Price Oracle Aggregator"),
    );

    assert_eq!(client.get_admin_address(), admin);
    assert_eq!(client.get_min_sources_required(), 2u32);
    assert_eq!(client.get_max_history_length(), 10u32);
    assert_eq!(client.get_decimals(), 18u32);
    assert_eq!(
        client.get_description(),
        String::from_str(&e, "Stellar Price Oracle Aggregator")
    );
}

#[test]
fn test_initialize_defaults() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(&admin, &0u32, &0u32, &6u32, &String::from_str(&e, "Test"));

    assert_eq!(client.get_min_sources_required(), 1u32);
    assert_eq!(client.get_max_history_length(), 100u32);
    assert_eq!(client.get_decimals(), 6u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn test_initialize_twice() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(&admin, &2u32, &10u32, &18u32, &String::from_str(&e, "Test"));
    client.initialize(&admin, &2u32, &10u32, &18u32, &String::from_str(&e, "Test"));
}

#[test]
fn test_set_admin() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let new_admin = Address::generate(&e);
    client.set_admin(&new_admin);
    assert_eq!(client.get_admin_address(), new_admin);
}

#[test]
fn test_set_admin_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let new_admin = Address::generate(&e);
    clear_auth(&e);
    assert!(client.try_set_admin(&new_admin).is_err());
}

#[test]
fn test_admin_functions() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_min_sources_required(&3u32);
    assert_eq!(client.get_min_sources_required(), 3u32);

    client.set_max_history_length(&50u32);
    assert_eq!(client.get_max_history_length(), 50u32);

    client.set_decimals(&8u32);
    assert_eq!(client.get_decimals(), 8u32);

    client.set_description(&String::from_str(&e, "Updated Description"));
    assert_eq!(
        client.get_description(),
        String::from_str(&e, "Updated Description")
    );
}

#[test]
fn test_register_asset() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    assert!(client.is_asset_registered(&asset));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_register_asset_twice() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset = Address::generate(&e);
    client.register_asset(&asset);
    client.register_asset(&asset);
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_register_asset_max_assets_reached() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_max_assets(&2u32);

    let asset1 = register_test_asset(&e, &client);
    assert!(client.is_asset_registered(&asset1));

    let asset2 = register_test_asset(&e, &client);
    assert!(client.is_asset_registered(&asset2));

    // Third registration should fail
    let _asset3 = register_test_asset(&e, &client);
}

#[test]
fn test_default_max_assets() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    assert_eq!(client.get_max_assets(), 100u32);
}

#[test]
fn test_unregister_asset() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    assert!(client.is_asset_registered(&asset));

    client.unregister_asset(&asset);
    assert!(!client.is_asset_registered(&asset));
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_unregister_unregistered_asset() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset = Address::generate(&e);
    client.unregister_asset(&asset);
}

#[test]
fn test_add_remove_source() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let source = register_test_source(&e, &client, "Chainlink");
    assert!(client.is_source(&source));

    client.remove_source(&source);
    assert!(!client.is_source(&source));
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_add_source_twice() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "Chainlink"));
    client.add_source(&source, &String::from_str(&e, "Chainlink"));
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_remove_nonexistent_source() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let source = Address::generate(&e);
    client.remove_source(&source);
}

#[test]
fn test_get_oracle_sources() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");

    let sources = client.get_oracle_sources();
    assert_eq!(sources.sources.len(), 2);
    assert_eq!(
        sources.metadata.get(source1.clone()).unwrap(),
        String::from_str(&e, "Chainlink")
    );
    assert_eq!(
        sources.metadata.get(source2.clone()).unwrap(),
        String::from_str(&e, "Band")
    );
}

#[test]
fn test_submit_price_and_get_price() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);

    // Only one source submitted, min_sources=2 → not aggregated yet → None
    assert!(client.get_price(&asset, &0u64).is_none());

    submit_test_price(&client, &source2, &asset, 110i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 105i128);
    assert_eq!(price.num_sources, 2u32);
    assert_eq!(price.timestamp, 1234567890u64);
    assert_eq!(price.decimals, 18u32);
}

#[test]
fn test_submit_price_median_odd() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    let source3 = register_test_source(&e, &client, "Redstone");
    client.set_min_sources_required(&3u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);
    submit_test_price(&client, &source3, &asset, 300i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 200i128);
    assert_eq!(price.num_sources, 3u32);
}

#[test]
fn test_submit_price_median_even() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "A");
    let source2 = register_test_source(&e, &client, "B");
    let source3 = register_test_source(&e, &client, "C");
    let source4 = register_test_source(&e, &client, "D");
    client.set_min_sources_required(&4u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);
    submit_test_price(&client, &source3, &asset, 300i128, 1234567890);
    submit_test_price(&client, &source4, &asset, 400i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 250i128);
    assert_eq!(price.num_sources, 4u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_submit_price_unauthorized_source() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let fake_source = Address::generate(&e);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &fake_source, &asset, 100i128, 1234567890);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_submit_price_invalid_zero() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    register_test_source(&e, &client, "Band");
    let asset1 = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset1, 0i128, 1234567890);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_submit_price_invalid_negative() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    register_test_source(&e, &client, "Band");
    let asset1 = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset1, -100i128, 1234567890);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_submit_price_unregistered_asset() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");

    let unregistered_asset = Address::generate(&e);
    submit_test_price(&client, &source1, &unregistered_asset, 100i128, 1234567890);
}

#[test]
fn test_get_source_price() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    register_test_source(&e, &client, "Band");
    let asset1 = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset1, 100i128, 1234567890);

    let entry: PriceEntry = client.get_source_price(&asset1, &source1);
    assert_eq!(entry.price, 100i128);
    assert_eq!(entry.timestamp, 1234567890u64);
    assert_eq!(entry.source, source1);
    assert_eq!(entry.decimals, 18u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_get_source_price_nonexistent_source() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    register_test_source(&e, &client, "Chainlink");
    register_test_source(&e, &client, "Band");
    let asset1 = register_test_asset(&e, &client);

    let fake_source = Address::generate(&e);
    client.get_source_price(&asset1, &fake_source);
}

#[test]
fn test_get_all_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 2);

    let price0: PriceEntry = all_prices.get_unchecked(0);
    let price1: PriceEntry = all_prices.get_unchecked(1);
    assert_eq!(price0.price, 100i128);
    assert_eq!(price1.price, 200i128);
}

#[test]
fn test_get_latest_ledger() {
    let e = Env::default();
    ledger_default(&e, 42, 1234567890);

    let (client, _) = setup_contract(&e);
    assert_eq!(client.get_latest_ledger(), 42u32);
}

#[test]
fn test_get_price_no_data() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);

    // No prices submitted → None
    assert!(client.get_price(&asset, &0u64).is_none());
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_get_price_unregistered_asset() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset = Address::generate(&e);
    client.get_price(&asset, &0u64);
}

#[test]
fn test_historical_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 110i128, 1234567890);

    assert!(client.has_historical_price(&asset, &100u32));

    let history = client.get_historical_price(&asset, &100u32);
    assert_eq!(history.price, 105i128);
    assert_eq!(history.ledger, 100u32);
    assert_eq!(history.num_sources, 2u32);

    let history_range = client.get_historical_prices(&asset, &100u32, &100u32);
    assert_eq!(history_range.len(), 1);

    let empty_range = client.get_historical_prices(&asset, &101u32, &110u32);
    assert_eq!(empty_range.len(), 0);
}

#[test]
fn test_historical_prices_multiple() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    let source3 = register_test_source(&e, &client, "Redstone");
    client.set_min_sources_required(&3u32);
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 100, 1234567890);
    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);
    submit_test_price(&client, &source3, &asset, 300i128, 1234567890);

    ledger_default(&e, 101, 1234567891);
    submit_test_price(&client, &source1, &asset, 110i128, 1234567891);
    submit_test_price(&client, &source2, &asset, 210i128, 1234567891);
    submit_test_price(&client, &source3, &asset, 310i128, 1234567891);

    ledger_default(&e, 102, 1234567892);
    submit_test_price(&client, &source1, &asset, 120i128, 1234567892);
    submit_test_price(&client, &source2, &asset, 220i128, 1234567892);
    submit_test_price(&client, &source3, &asset, 320i128, 1234567892);

    let history_range = client.get_historical_prices(&asset, &100u32, &102u32);
    assert_eq!(history_range.len(), 3);
    assert_eq!(history_range.get_unchecked(0).price, 200i128);
    assert_eq!(history_range.get_unchecked(1).price, 210i128);
    assert_eq!(history_range.get_unchecked(2).price, 220i128);
}

#[test]
fn test_has_historical_price_false() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    assert!(!client.has_historical_price(&asset, &999u32));
}

#[test]
fn test_has_historical_price_unregistered_asset() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset = Address::generate(&e);
    assert!(!client.has_historical_price(&asset, &100u32));
}

#[test]
fn test_upgrade() {
    // Included wasm upgrade test requires a pre-built artifact.
    // Skip in environments where `target/wasm32v1-none/release/price_oracle.wasm` is missing.
    #[cfg(feature = "skip_wasm_upgrade_tests")]
    {
        return;
    }

    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let wasm = include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm");
    let new_wasm_hash = e
        .deployer()
        .upload_contract_wasm(Bytes::from_slice(&e, wasm));
    client.upgrade(&new_wasm_hash);
}

#[test]
fn test_upgrade_unauthorized() {
    // Included wasm upgrade test requires a pre-built artifact.
    // Skip in environments where `target/wasm32v1-none/release/price_oracle.wasm` is missing.
    #[cfg(feature = "skip_wasm_upgrade_tests")]
    {
        return;
    }

    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let wasm = include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm");
    let new_wasm_hash = e
        .deployer()
        .upload_contract_wasm(Bytes::from_slice(&e, wasm));
    clear_auth(&e);
    assert!(client.try_upgrade(&new_wasm_hash).is_err());
}

#[test]
fn test_unauthorized_add_source() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let source = Address::generate(&e);
    clear_auth(&e);
    assert!(client
        .try_add_source(&source, &String::from_str(&e, "Bad Source"))
        .is_err());
}

#[test]
fn test_unauthorized_remove_source() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Test");

    clear_auth(&e);
    assert!(client.try_remove_source(&source).is_err());
}

#[test]
fn test_unauthorized_set_min_sources() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    assert!(client.try_set_min_sources_required(&5u32).is_err());
}

#[test]
fn test_unauthorized_set_max_history() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    assert!(client.try_set_max_history_length(&50u32).is_err());
}

#[test]
fn test_unauthorized_set_decimals() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    assert!(client.try_set_decimals(&8u32).is_err());
}

#[test]
fn test_unauthorized_set_description() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    assert!(client
        .try_set_description(&String::from_str(&e, "Hacked"))
        .is_err());
}

#[test]
fn test_multiple_assets() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);

    let xlm = register_test_asset(&e, &client);
    let eth = register_test_asset(&e, &client);
    let btc = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &xlm, 100i128, 1234567890);
    submit_test_price(&client, &source2, &xlm, 102i128, 1234567890);

    submit_test_price(&client, &source1, &eth, 180000i128, 1234567890);
    submit_test_price(&client, &source2, &eth, 181000i128, 1234567890);

    submit_test_price(&client, &source1, &btc, 30000000i128, 1234567890);
    submit_test_price(&client, &source2, &btc, 31000000i128, 1234567890);

    let xlm_price = client.get_price(&xlm, &0u64).unwrap();
    assert_eq!(xlm_price.price, 101i128);

    let eth_price = client.get_price(&eth, &0u64).unwrap();
    assert_eq!(eth_price.price, 180500i128);

    let btc_price = client.get_price(&btc, &0u64).unwrap();
    assert_eq!(btc_price.price, 30500000i128);
}

#[test]
fn test_submit_price_updates_timestamp() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1000);
    submit_test_price(&client, &source2, &asset, 110i128, 2000);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.timestamp, 2000u64);

    submit_test_price(&client, &source2, &asset, 120i128, 3000);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.timestamp, 3000u64);
}

#[test]
fn test_single_source_no_aggregation() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 100i128);
    assert_eq!(price.num_sources, 1u32);
}

#[test]
fn test_price_source_not_affected_by_other_assets() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);

    let asset_a = register_test_asset(&e, &client);
    let asset_b = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset_a, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset_a, 110i128, 1234567890);

    let price_a = client.get_price(&asset_a, &0u64).unwrap();
    assert_eq!(price_a.price, 105i128);

    // asset_b has no submissions → None
    assert!(client.get_price(&asset_b, &0u64).is_none());
}

// ---- SEP-40 Oracle Interface Tests ----

#[test]
fn test_sep40_base() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let result = client.base();
    assert_eq!(result, Asset::Other(Symbol::new(&e, "USD")));
}

#[test]
fn test_sep40_assets() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    let assets = client.assets();
    assert_eq!(assets.len(), 2);
    assert_eq!(assets.get_unchecked(0), Asset::Stellar(asset1));
    assert_eq!(assets.get_unchecked(1), Asset::Stellar(asset2));
}

#[test]
fn test_sep40_resolution() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    assert_eq!(client.resolution(), 0u32);

    client.set_resolution(&300u32);
    assert_eq!(client.resolution(), 300u32);
}

#[test]
fn test_sep40_lastprice() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 110i128, 1234567890);

    let result = client.lastprice(&Asset::Stellar(asset));
    assert!(result.is_some());
    let data: PriceData = result.unwrap();
    assert_eq!(data.price, 105i128);
    assert_eq!(data.timestamp, 1234567890u64);
}

#[test]
fn test_sep40_lastprice_unregistered() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);

    let unregistered = Address::generate(&e);
    let result = client.lastprice(&Asset::Stellar(unregistered));
    assert!(result.is_none());
}

#[test]
fn test_sep40_lastprice_other() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let result = client.lastprice(&Asset::Other(Symbol::new(&e, "EUR")));
    assert!(result.is_none());
}

#[test]
fn test_sep40_lastprice_stale() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    client.set_min_sources_required(&1u32);
    client.set_resolution(&10u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);

    // Advance ledger past resolution window
    ledger_default(&e, 200, 1234567910);
    let result = client.lastprice(&Asset::Stellar(asset));
    assert!(result.is_none());
}

#[test]
fn test_sep40_price() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    client.set_min_sources_required(&2u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 110i128, 1234567890);

    let result = client.price(&Asset::Stellar(asset), &1234567890u64);
    assert!(result.is_some());
    let data: PriceData = result.unwrap();
    assert_eq!(data.price, 105i128);
}

#[test]
fn test_sep40_price_wrong_timestamp() {
    let e = Env::default();
    // Keep ledger low so history back-scan stays under footprint limit (100)
    ledger_default(&e, 50, 1000);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1000);

    // Query with timestamp before data exists → should find no match
    let result = client.price(&Asset::Stellar(asset), &999u64);
    assert!(result.is_none());
}

#[test]
fn test_sep40_price_other() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let result = client.price(&Asset::Other(Symbol::new(&e, "BTC")), &1234567890u64);
    assert!(result.is_none());
}

#[test]
fn test_sep40_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    let source3 = register_test_source(&e, &client, "Redstone");
    client.set_min_sources_required(&3u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);
    submit_test_price(&client, &source3, &asset, 300i128, 1234567890);

    let result = client.prices(&Asset::Stellar(asset), &5u32);
    assert!(result.is_some());
    let prices: Vec<PriceData> = result.unwrap();
    assert!(prices.len() >= 1);
    assert_eq!(prices.get_unchecked(0).price, 200i128);
}

#[test]
fn test_sep40_prices_empty() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    let result = client.prices(&Asset::Stellar(asset), &5u32);
    assert!(result.is_some());
    let prices: Vec<PriceData> = result.unwrap();
    // No prices submitted, no aggregate stored yet
    assert_eq!(prices.len(), 0);
}

#[test]
fn test_sep40_prices_unregistered_asset() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let unregistered = Address::generate(&e);
    let result = client.prices(&Asset::Stellar(unregistered), &5u32);
    assert!(result.is_none());
}

// ---- SEP-40 decimals() ----

#[test]
fn test_sep40_decimals() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    assert_eq!(client.decimals(), 18u32);

    client.set_decimals(&8u32);
    assert_eq!(client.decimals(), 8u32);
    // Alias must match get_decimals
    assert_eq!(client.decimals(), client.get_decimals());
}

// ---- Timestamp Validation Tests ----

#[test]
fn test_submit_price_current_timestamp_accepted() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, _admin, source, asset) = setup_basic(&e);

    // Timestamp equal to ledger time — accepted
    client.submit_price(&source, &asset, &100i128, &1000u64);
}

#[test]
fn test_submit_price_past_timestamp_accepted() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, _admin, source, asset) = setup_basic(&e);

    // Timestamp in the past — accepted
    client.submit_price(&source, &asset, &100i128, &500u64);
}

#[test]
fn test_submit_price_slightly_future_timestamp_accepted() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, _admin, source, asset) = setup_basic(&e);

    // Timestamp within threshold (default 300s) — accepted
    client.submit_price(&source, &asset, &100i128, &1299u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_submit_price_far_future_timestamp_rejected() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, _admin, source, asset) = setup_basic(&e);

    // Timestamp more than 5 minutes (300s) in the future — rejected
    client.submit_price(&source, &asset, &100i128, &1301u64);
}

#[test]
fn test_set_get_timestamp_threshold() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Default is 300
    assert_eq!(client.get_timestamp_threshold(), 300u64);

    client.set_timestamp_threshold(&600u64);
    assert_eq!(client.get_timestamp_threshold(), 600u64);
}

#[test]
fn test_timestamp_threshold_configurable() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, _admin, source, asset) = setup_basic(&e);

    // With default threshold of 300s, timestamp 1310 would be rejected
    // Set threshold to 600s
    client.set_timestamp_threshold(&600u64);

    // Now 1599 should be accepted (within 600s)
    client.submit_price(&source, &asset, &100i128, &1599u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_timestamp_threshold_custom_rejects_beyond() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, _admin, source, asset) = setup_basic(&e);

    client.set_timestamp_threshold(&60u64);

    // 1061 is 61s in future — beyond custom threshold of 60s
    client.submit_price(&source, &asset, &100i128, &1061u64);
}

// ---- Asset Lifecycle Tests ----

#[test]
fn test_asset_lifecycle_register_submit_unregister_reregister() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Oracle");
    let asset = register_test_asset(&e, &client);

    // Submit a price
    submit_test_price(&client, &source, &asset, 500i128, 1000);
    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 500i128);

    // Unregister the asset
    client.unregister_asset(&asset);
    assert!(!client.is_asset_registered(&asset));

    // Asset no longer in the assets list
    let assets_list = client.assets();
    let mut found = false;
    for i in 0..assets_list.len() {
        if let crate::Asset::Stellar(ref a) = assets_list.get_unchecked(i) {
            if *a == asset {
                found = true;
            }
        }
    }
    assert!(!found);

    // Re-register the same asset
    client.register_asset(&asset);
    assert!(client.is_asset_registered(&asset));

    // No aggregate price yet after re-registration
    assert!(client.get_price(&asset, &0u64).is_none());

    // Submit new price after re-registration
    submit_test_price(&client, &source, &asset, 600i128, 1000);
    let new_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(new_price.price, 600i128);
}

#[test]
fn test_asset_not_in_list_after_unregister() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    assert_eq!(client.assets().len(), 1);
    client.unregister_asset(&asset);
    assert_eq!(client.assets().len(), 0);
}

#[test]
fn test_asset_reregister_after_unregister() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Oracle");

    let asset_addr = Address::generate(&e);
    client.register_asset(&asset_addr);
    submit_test_price(&client, &source, &asset_addr, 100i128, 1000);

    client.unregister_asset(&asset_addr);

    // Re-register
    client.register_asset(&asset_addr);
    assert!(client.is_asset_registered(&asset_addr));

    // Submit fresh price
    submit_test_price(&client, &source, &asset_addr, 200i128, 1000);
    let p = client.get_price(&asset_addr, &0u64).unwrap();
    assert_eq!(p.price, 200i128);
}

// ---- Task 4: Removed Source Data Integrity Tests ----

#[test]
fn test_removed_source_cannot_submit_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);

    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Oracle");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 100i128, 1000);

    client.remove_source(&source);

    // Removed source cannot submit
    assert!(client
        .try_submit_price(&source, &asset, &200i128, &1000u64)
        .is_err());
}

#[test]
fn test_removed_source_price_not_in_get_all_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source1 = register_test_source(&e, &client, "Oracle1");
    let source2 = register_test_source(&e, &client, "Oracle2");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1000);
    submit_test_price(&client, &source2, &asset, 200i128, 1000);

    // Remove source1
    client.remove_source(&source1);

    // get_all_prices only returns active sources
    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 1);
    let entry: PriceEntry = all_prices.get_unchecked(0);
    assert_eq!(entry.source, source2);
}

#[test]
fn test_removed_source_historical_price_still_accessible() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&2u32);
    let source1 = register_test_source(&e, &client, "Oracle1");
    let source2 = register_test_source(&e, &client, "Oracle2");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1000);
    submit_test_price(&client, &source2, &asset, 200i128, 1000);

    // Aggregate was recorded at ledger 100
    assert!(client.has_historical_price(&asset, &100u32));
    let hist = client.get_historical_price(&asset, &100u32);
    assert_eq!(hist.price, 150i128);

    // Remove source1
    client.remove_source(&source1);

    // Historical price is still accessible
    assert!(client.has_historical_price(&asset, &100u32));
    let hist_after = client.get_historical_price(&asset, &100u32);
    assert_eq!(hist_after.price, 150i128);
}

#[test]
fn test_removed_source_is_no_longer_source() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Oracle");

    assert!(client.is_source(&source));
    client.remove_source(&source);
    assert!(!client.is_source(&source));
}

// ===== Issue #82: Overflow/Underflow Boundary Tests =====

#[test]
fn test_median_i128_max_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Oracle");
    let asset = register_test_asset(&e, &client);

    // Submit i128::MAX price — should not panic
    submit_test_price(&client, &source, &asset, i128::MAX, 9999);
    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
    assert_eq!(price.unwrap().price, i128::MAX);
}

#[test]
fn test_median_two_i128_max_prices_no_overflow() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&2u32);
    let source1 = register_test_source(&e, &client, "Oracle1");
    let source2 = register_test_source(&e, &client, "Oracle2");
    let asset = register_test_asset(&e, &client);

    // Both sources submit i128::MAX — median(MAX, MAX) = MAX; a + (b-a)/2 = MAX + 0 = MAX
    submit_test_price(&client, &source1, &asset, i128::MAX, 9999);
    submit_test_price(&client, &source2, &asset, i128::MAX, 9999);
    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
    assert_eq!(price.unwrap().price, i128::MAX);
}

#[test]
fn test_median_min_and_max_i128_no_overflow() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&2u32);
    let source1 = register_test_source(&e, &client, "Oracle1");
    let source2 = register_test_source(&e, &client, "Oracle2");
    let asset = register_test_asset(&e, &client);

    // median(1, MAX) = 1 + (MAX - 1) / 2; no overflow because a + (b - a) / 2 pattern is safe
    submit_test_price(&client, &source1, &asset, 1i128, 9999);
    submit_test_price(&client, &source2, &asset, i128::MAX, 9999);
    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
    let expected = 1i128 + (i128::MAX - 1) / 2;
    assert_eq!(price.unwrap().price, expected);
}

#[test]
fn test_historical_prices_start_greater_than_end_returns_error() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Oracle");
    let asset = register_test_asset(&e, &client);
    submit_test_price(&client, &source, &asset, 100i128, 9999);

    // end < start should panic (NoData) rather than underflow
    let result = client.try_get_historical_prices(&asset, &200u32, &100u32);
    assert!(result.is_err());
}

#[test]
fn test_get_price_staleness_u64_max_age_no_overflow() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Oracle");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 100i128, 9999);
    // max_age = u64::MAX should not overflow timestamp.saturating_add(max_age)
    let price = client.get_price(&asset, &u64::MAX);
    assert!(price.is_some());
}

#[test]
fn test_mean_saturating_sum_large_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&2u32);
    let source1 = register_test_source(&e, &client, "Oracle1");
    let source2 = register_test_source(&e, &client, "Oracle2");
    let asset = register_test_asset(&e, &client);

    // Two very large prices; sum would overflow i128 without saturating_add in compute_mean
    submit_test_price(&client, &source1, &asset, i128::MAX / 2 + 1, 9999);
    submit_test_price(&client, &source2, &asset, i128::MAX / 2 + 1, 9999);
    // Default aggregation is median; median of two equal values = that value
    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
    assert_eq!(price.unwrap().price, i128::MAX / 2 + 1);
}

// ===== Issue #85: Strict Input Validation Tests =====

// --- add_source name validation ---

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_add_source_empty_name_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = soroban_sdk::Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, ""));
}

#[test]
#[should_panic(expected = "Error(Contract, #17)")]
fn test_add_source_name_too_long_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = soroban_sdk::Address::generate(&e);
    // 65-character name (max is 64)
    let long_name = String::from_str(&e, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    client.add_source(&source, &long_name);
}

#[test]
fn test_add_source_name_exactly_64_chars_accepted() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = soroban_sdk::Address::generate(&e);
    // exactly 64 characters
    let name = String::from_str(&e, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    client.add_source(&source, &name);
    assert!(client.is_source(&source));
}

// --- set_max_history_length zero validation ---

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_set_max_history_length_zero_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    client.set_max_history_length(&0u32);
}

#[test]
fn test_set_max_history_length_one_accepted() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    client.set_max_history_length(&1u32);
    assert_eq!(client.get_max_history_length(), 1u32);
}

// --- set_decimals upper bound validation ---

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_set_decimals_above_18_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    client.set_decimals(&19u32);
}

#[test]
fn test_set_decimals_18_accepted() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    client.set_decimals(&18u32);
    assert_eq!(client.get_decimals(), 18u32);
}

// --- initialize decimals upper bound validation ---

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_initialize_decimals_above_18_rejected() {
    let e = Env::default();
    let admin = soroban_sdk::Address::generate(&e);
    let client = create_contract(&e);
    client.initialize(&admin, &1u32, &10u32, &19u32, &String::from_str(&e, "Test"));
}

// --- override_price reason length validation ---

#[test]
#[should_panic(expected = "Error(Contract, #20)")]
fn test_override_price_reason_too_long_rejected() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    // 257-char reason string (max is 256)
    let reason = String::from_str(
        &e,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    client.override_price(&asset, &1000i128, &reason, &300u32);
}

#[test]
fn test_override_price_reason_exactly_256_chars_accepted() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    // exactly 256 characters
    let reason = String::from_str(
        &e,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    client.override_price(&asset, &1000i128, &reason, &300u32);
    assert!(client.get_price_override(&asset).is_some());
}

// --- prices SEP-40 records cap validation ---

#[test]
#[should_panic(expected = "Error(Contract, #19)")]
fn test_prices_records_exceeds_max_history_rejected() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);
    // max_history is 10 (from setup_contract), requesting 11 should fail
    let asset = register_test_asset(&e, &client);
    client.prices(&crate::Asset::Stellar(asset), &11u32);
}

#[test]
fn test_prices_records_at_max_history_accepted() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);
    // Reduce max_history to 3 so the ledger scan (3*10=30) stays within footprint limits
    client.set_max_history_length(&3u32);
    let asset = register_test_asset(&e, &client);
    let result = client.prices(&crate::Asset::Stellar(asset), &3u32);
    assert!(result.is_some());
}

// --- propose_operation invalid op_type validation ---

#[test]
#[should_panic(expected = "Error(Contract, #18)")]
fn test_propose_operation_invalid_type_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let data = soroban_sdk::Bytes::new(&e);
    client.propose_operation(&99u32, &data);
}

#[test]
fn test_propose_operation_valid_types_accepted() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let data = soroban_sdk::Bytes::new(&e);
    // op_type 0..=7 are all valid
    for op in 0u32..=7 {
        client.propose_operation(&op, &data);
    }
}
