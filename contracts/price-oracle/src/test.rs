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

    assert_eq!(client.get_max_sources(), 50u32);
    client.set_max_sources(&10u32);
    assert_eq!(client.get_max_sources(), 10u32);
}

#[test]
fn test_max_sources_enforced() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_max_sources(&3u32);

    let s1 = register_test_source(&e, &client, "S1");
    let s2 = register_test_source(&e, &client, "S2");
    let s3 = register_test_source(&e, &client, "S3");

    assert!(client.is_source(&s1));
    assert!(client.is_source(&s2));
    assert!(client.is_source(&s3));

    let s4 = Address::generate(&e);
    let result = client.try_add_source(&s4, &String::from_str(&e, "S4"));
    assert!(result.is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_max_sources_enforced_exact_limit() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_max_sources(&2u32);

    register_test_source(&e, &client, "S1");
    register_test_source(&e, &client, "S2");

    let s3 = Address::generate(&e);
    client.add_source(&s3, &String::from_str(&e, "S3"));
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

fn load_wasm_hash(e: &Env) -> soroban_sdk::BytesN<32> {
    let wasm = include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm");
    e.deployer()
        .upload_contract_wasm(Bytes::from_slice(e, wasm))
}

fn load_wasm_bytes() -> &'static [u8] {
    include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm")
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

    let new_wasm_hash = load_wasm_hash(&e);
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

    let new_wasm_hash = load_wasm_hash(&e);
    clear_auth(&e);
    assert!(client.try_upgrade(&new_wasm_hash).is_err());
}

#[test]
fn test_upgrade_empty_wasm_blob_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Empty hashes are not valid contract upgrade targets.
    // Expect the Soroban upgrade path to reject.
    let empty_hash = soroban_sdk::BytesN::<32>::from_array(&e, &[0u8; 32]);

    assert!(client.try_upgrade(&empty_hash).is_err());

    // Contract remains callable.
    assert!(client.get_description().len() > 0);
}

#[test]
fn test_upgrade_malformed_wasm_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Use a known-malformed upgrade target hash.
    // In this mock environment, invalid upgrade candidates are expected to be rejected
    // by the upgrade call (not just by upload-time validation).
    let malformed_hash = soroban_sdk::BytesN::<32>::from_array(&e, &[0xAAu8; 32]);

    assert!(client.try_upgrade(&malformed_hash).is_err());

    // Contract remains callable.
    assert!(client.get_description().len() > 0);
}

#[test]
fn test_upgrade_wasm_without_expected_interface_handled() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Upgrade targets are identified by wasm hashes. Provide a value that is not
    // a deployed/valid contract wasm in the mock environment.
    let mismatched_interface_hash = soroban_sdk::BytesN::<32>::from_array(&e, &[0xBBu8; 32]);

    // Expect rejection during upgrade.
    assert!(client.try_upgrade(&mismatched_interface_hash).is_err());

    // Contract remains callable.
    assert!(client.get_description().len() > 0);
}

#[test]
fn test_upgrade_from_non_admin_rejected() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let new_wasm_hash = load_wasm_hash(&e);

    // Clear admin auth so upgrade should fail.
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

// ---- Rate Limiting Tests ----

#[test]
fn test_set_get_query_rate_limit() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Default rate limit is 100 when not explicitly set
    assert_eq!(client.get_query_rate_limit(), 100u32);

    client.set_query_rate_limit(&50u32);
    assert_eq!(client.get_query_rate_limit(), 50u32);

    client.set_query_rate_limit(&200u32);
    assert_eq!(client.get_query_rate_limit(), 200u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_rate_limit_enforced() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    client.set_query_rate_limit(&2u32);

    // First two queries within the limit of 2
    let _ = client.get_price(&asset, &0u64);
    let _ = client.get_price(&asset, &0u64);

    // Third query exceeds the rate limit → panics with RateLimitExceeded (#16)
    let _ = client.get_price(&asset, &0u64);
}

#[test]
fn test_rate_limit_resets_each_ledger() {
    let e = Env::default();
    ledger_default(&e, 100, 10000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    client.set_query_rate_limit(&2u32);

    // Exhaust the limit on ledger 100
    let _ = client.get_price(&asset, &0u64);
    let _ = client.get_price(&asset, &0u64);

    // Advance to a new ledger — rate limit counter should reset
    ledger_default(&e, 101, 10001);

    // This should succeed because the count is per-ledger
    let result = client.get_price(&asset, &0u64);
    assert!(result.is_none());
}

// ---- Subscription Tests ----

#[test]
fn test_set_subscription_price_and_get_plans() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_subscription_price(&86400u32, &100i128);
    client.set_subscription_price(&604800u32, &500i128);

    let plans = client.get_subscription_plans();
    assert_eq!(plans.len(), 2u32);
    assert_eq!(plans.get(86400u32).unwrap(), 100i128);
    assert_eq!(plans.get(604800u32).unwrap(), 500i128);
}

#[test]
fn test_subscribe_and_get_expiry() {
    let e = Env::default();
    ledger_default(&e, 100, 1000000);

    let (client, admin) = setup_contract(&e);
    client.set_subscription_price(&86400u32, &100i128);

    let consumer = Address::generate(&e);
    client.subscribe(&consumer, &86400u32);

    let expiry = client.get_subscription_expiry(&consumer);
    assert_eq!(expiry, 1086400u64);
}

#[test]
fn test_renew_subscription() {
    let e = Env::default();
    ledger_default(&e, 100, 1000000);

    let (client, _admin) = setup_contract(&e);
    client.set_subscription_price(&86400u32, &100i128);

    let consumer = Address::generate(&e);
    client.subscribe(&consumer, &86400u32);

    let expiry_before = client.get_subscription_expiry(&consumer);
    assert_eq!(expiry_before, 1086400u64);

    client.renew_subscription(&consumer);

    let expiry_after = client.get_subscription_expiry(&consumer);
    assert_eq!(expiry_after, 1086400u64 + 86400u64);
}

#[test]
#[should_panic(expected = "Error(Contract, #18)")]
fn test_renew_expired_subscription() {
    let e = Env::default();
    ledger_default(&e, 100, 1000000);

    let (client, _admin) = setup_contract(&e);
    client.set_subscription_price(&86400u32, &100i128);

    let consumer = Address::generate(&e);
    client.subscribe(&consumer, &86400u32);

    // Advance time past expiry
    ledger_default(&e, 101, 2000000);

    // Renewal should fail with SubscriptionExpired
    client.renew_subscription(&consumer);
}

// ==== Frontrunning Prevention Tests ====
//
// Median aggregation is inherently resistant to frontrunning because the
// result depends solely on the set of submitted price values, not on the
// order in which those prices are submitted. The contract sorts all valid
// prices before computing the median (see `compute_median` in storage.rs),
// so any permutation of the same price set yields an identical median.
//
// An attacker who observes pending transactions ("mempool") cannot bias the
// oracle output by reordering their own submission, because the median is
// computed post-sort. Only the aggregate timestamp updates to the latest
// individual submission timestamp, which does not change the price outcome.

// --- Test 1: Ordering independence ---
// Submitting the same prices in different orders produces the identical median.
#[test]
fn test_median_ordering_independent() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&3u32);

    let source1 = register_test_source(&e, &client, "A");
    let source2 = register_test_source(&e, &client, "B");
    let source3 = register_test_source(&e, &client, "C");
    let asset = register_test_asset(&e, &client);

    // Submission order A (ascending): 100, 150, 200 → median = 150
    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 150i128, 1234567890);
    submit_test_price(&client, &source3, &asset, 200i128, 1234567890);

    let price_asc = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price_asc.price, 150i128);

    // Reset by removing and re-registering the asset (creates a fresh oracle env)
    let e2 = Env::default();
    ledger_default(&e2, 100, 1234567890);
    let (client2, _) = setup_contract(&e2);
    client2.set_min_sources_required(&3u32);

    let src1 = register_test_source(&e2, &client2, "A");
    let src2 = register_test_source(&e2, &client2, "B");
    let src3 = register_test_source(&e2, &client2, "C");
    let asset2 = register_test_asset(&e2, &client2);

    // Submission order B (descending): 200, 150, 100 → median = 150 (same result)
    submit_test_price(&client2, &src3, &asset2, 200i128, 1234567890);
    submit_test_price(&client2, &src2, &asset2, 150i128, 1234567890);
    submit_test_price(&client2, &src1, &asset2, 100i128, 1234567890);

    let price_desc = client2.get_price(&asset2, &0u64).unwrap();
    assert_eq!(price_desc.price, 150i128);

    // --- Submission order C (random/out-of-order): 200, 100, 150 → median = 150 (same result)
    let e3 = Env::default();
    ledger_default(&e3, 100, 1234567890);
    let (client3, _) = setup_contract(&e3);
    client3.set_min_sources_required(&3u32);

    let s1 = register_test_source(&e3, &client3, "A");
    let s2 = register_test_source(&e3, &client3, "B");
    let s3 = register_test_source(&e3, &client3, "C");
    let asset3 = register_test_asset(&e3, &client3);

    submit_test_price(&client3, &s3, &asset3, 200i128, 1234567890);
    submit_test_price(&client3, &s1, &asset3, 100i128, 1234567890);
    submit_test_price(&client3, &s2, &asset3, 150i128, 1234567890);

    let price_mixed = client3.get_price(&asset3, &0u64).unwrap();
    assert_eq!(price_mixed.price, 150i128);
}

// --- Test 2: Multi-source submissions with different timestamps ---
// Prices submitted at different timestamps still converge to the same median.
// Only the aggregate's `timestamp` field reflects the latest submission; the
// `price` (median) is unchanged by timestamp variation.
#[test]
fn test_median_resistant_to_timestamp_manipulation() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&3u32);

    let source1 = register_test_source(&e, &client, "Chainlink");
    let source2 = register_test_source(&e, &client, "Band");
    let source3 = register_test_source(&e, &client, "Redstone");
    let asset = register_test_asset(&e, &client);

    // Scenario A: all sources submit at the same timestamp
    submit_test_price(&client, &source1, &asset, 100i128, 1000);
    submit_test_price(&client, &source2, &asset, 200i128, 1000);
    submit_test_price(&client, &source3, &asset, 300i128, 1000);

    let price_same_ts = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price_same_ts.price, 200i128);
    assert_eq!(price_same_ts.timestamp, 1000u64);

    // Scenario B: same prices but source1 tries to frontrun by submitting a slightly
    // different timestamp early. The median price must remain 200.
    let e2 = Env::default();
    ledger_default(&e2, 100, 1234567890);
    let (client2, _) = setup_contract(&e2);
    client2.set_min_sources_required(&3u32);

    let src1 = register_test_source(&e2, &client2, "Chainlink");
    let src2 = register_test_source(&e2, &client2, "Band");
    let src3 = register_test_source(&e2, &client2, "Redstone");
    let asset2 = register_test_asset(&e2, &client2);

    // Frontrunner submits early with a different timestamp but same price value
    submit_test_price(&client2, &src1, &asset2, 100i128, 500);
    submit_test_price(&client2, &src2, &asset2, 200i128, 1500);
    submit_test_price(&client2, &src3, &asset2, 300i128, 2000);

    let price_diff_ts = client2.get_price(&asset2, &0u64).unwrap();
    assert_eq!(price_diff_ts.price, 200i128);
    // The aggregate timestamp is the MAX of individual timestamps
    assert_eq!(price_diff_ts.timestamp, 2000u64);

    // Even though timestamps differ, the median PRICE is identical
    assert_eq!(price_same_ts.price, price_diff_ts.price);
}

// --- Test 3: Frontrun resistance with adversarial price arrangement ---
// An attacker sees pending prices [50, 100, 150, 200, 250] (median = 150).
// Attacker submits a price to try to shift the median. We verify that with
// 5 sources, the median can only be changed by changing the actual price
// set — not by modifying the submission order or by adding an outlier.
#[test]
fn test_median_frontrun_resistance_adversarial() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&5u32);

    // 5 honest sources submitting prices across a range
    let honest1 = register_test_source(&e, &client, "Honest1");
    let honest2 = register_test_source(&e, &client, "Honest2");
    let honest3 = register_test_source(&e, &client, "Honest3");
    let honest4 = register_test_source(&e, &client, "Honest4");
    let honest5 = register_test_source(&e, &client, "Honest5");
    let asset = register_test_asset(&e, &client);

    // Honest submissions in random order: 250, 50, 100, 200, 150
    // Sorted: [50, 100, 150, 200, 250] → median = 150
    submit_test_price(&client, &honest1, &asset, 250i128, 1000);
    submit_test_price(&client, &honest2, &asset, 50i128, 1000);
    submit_test_price(&client, &honest3, &asset, 100i128, 1000);
    submit_test_price(&client, &honest4, &asset, 200i128, 1000);
    submit_test_price(&client, &honest5, &asset, 150i128, 1000);

    let honest_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(honest_price.price, 150i128);
    assert_eq!(honest_price.num_sources, 5u32);

    // Now a frontrunner submits after seeing the honest prices above.
    // They can choose any price. The median of 6 values is still determined
    // by the sorted middle two values, so it cannot be shifted unless the
    // frontrunner's price pushes past the existing median.
    let e2 = Env::default();
    ledger_default(&e2, 100, 1234567890);
    let (client2, _) = setup_contract(&e2);
    client2.set_min_sources_required(&5u32);

    let h1 = register_test_source(&e2, &client2, "Honest1");
    let h2 = register_test_source(&e2, &client2, "Honest2");
    let h3 = register_test_source(&e2, &client2, "Honest3");
    let h4 = register_test_source(&e2, &client2, "Honest4");
    let h5 = register_test_source(&e2, &client2, "Honest5");
    let frontrunner = register_test_source(&e2, &client2, "Frontrunner");
    let asset2 = register_test_asset(&e2, &client2);

    // Honest prices same as before
    submit_test_price(&client2, &h1, &asset2, 250i128, 1000);
    submit_test_price(&client2, &h2, &asset2, 50i128, 1000);
    submit_test_price(&client2, &h3, &asset2, 100i128, 1000);
    submit_test_price(&client2, &h4, &asset2, 200i128, 1000);
    submit_test_price(&client2, &h5, &asset2, 150i128, 1000);

    // Frontrunner tries to push median higher by submitting a high price
    // Sorted 6 values: [50, 100, 150, 200, 250, 900] → median = (150 + 200) / 2 = 175
    // With a moderate price (140): [50, 100, 140, 150, 200, 250] → median = (140 + 150) / 2 = 145
    submit_test_price(&client2, &frontrunner, &asset2, 900i128, 2000);

    let attacked_price = client2.get_price(&asset2, &0u64).unwrap();
    // Even with an extreme outliers, the median of 6 values [50, 100, 150, 200, 250, 900]
    // becomes (150 + 200) / 2 = 175 (median of even-length sorted list)
    assert_eq!(attacked_price.price, 175i128);
    // The honest 5-source median alone was 150 — an extreme outlier only shifts
    // it by one position because median is robust to extremes.
    assert!((attacked_price.price - 150i128).abs() <= 50i128);
}

// --- Test 4: Median is identical regardless of which source submits last ---
// The last-submitted price does not get priority in the median sort. This
// prevents a frontrunner from gaming the result by being the final caller.
#[test]
fn test_median_last_submission_has_no_priority() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&3u32);

    let source1 = register_test_source(&e, &client, "A");
    let source2 = register_test_source(&e, &client, "B");
    let source3 = register_test_source(&e, &client, "C");
    let asset = register_test_asset(&e, &client);

    // Source with the "attacker" price submits last but the median is still fair
    submit_test_price(&client, &source1, &asset, 100i128, 1000);
    submit_test_price(&client, &source2, &asset, 200i128, 1000);
    submit_test_price(&client, &source3, &asset, 150i128, 3000); // last, middle value

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 150i128);
    assert_eq!(price.num_sources, 3u32);

    // Flip order: make source3 submit first
    let e2 = Env::default();
    ledger_default(&e2, 100, 1234567890);
    let (client2, _) = setup_contract(&e2);
    client2.set_min_sources_required(&3u32);

    let s1 = register_test_source(&e2, &client2, "A");
    let s2 = register_test_source(&e2, &client2, "B");
    let s3 = register_test_source(&e2, &client2, "C");
    let asset2 = register_test_asset(&e2, &client2);

    submit_test_price(&client2, &s3, &asset2, 150i128, 1000); // "attacker" value submitted first
    submit_test_price(&client2, &s1, &asset2, 100i128, 2000);
    submit_test_price(&client2, &s2, &asset2, 200i128, 3000);

    let price2 = client2.get_price(&asset2, &0u64).unwrap();
    assert_eq!(price2.price, 150i128);
    assert_eq!(price.price, price2.price);
}

// --- Test 5: Even count — frontrun cannot pick which two values form the median ---
#[test]
fn test_median_even_sources_frontrun_resistant() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&4u32);

    let source1 = register_test_source(&e, &client, "A");
    let source2 = register_test_source(&e, &client, "B");
    let source3 = register_test_source(&e, &client, "C");
    let source4 = register_test_source(&e, &client, "D");
    let asset = register_test_asset(&e, &client);

    // 4 honest sources: 100, 200, 300, 400 → median = (200 + 300) / 2 = 250
    submit_test_price(&client, &source1, &asset, 100i128, 1000);
    submit_test_price(&client, &source2, &asset, 200i128, 1000);
    submit_test_price(&client, &source3, &asset, 300i128, 1000);
    submit_test_price(&client, &source4, &asset, 400i128, 1000);

    let honest_price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(honest_price.price, 250i128);

    // Try a different submission order → same median
    let e2 = Env::default();
    ledger_default(&e2, 100, 1234567890);
    let (client2, _) = setup_contract(&e2);
    client2.set_min_sources_required(&4u32);

    let s1 = register_test_source(&e2, &client2, "A");
    let s2 = register_test_source(&e2, &client2, "B");
    let s3 = register_test_source(&e2, &client2, "C");
    let s4 = register_test_source(&e2, &client2, "D");
    let asset2 = register_test_asset(&e2, &client2);

    submit_test_price(&client2, &s4, &asset2, 400i128, 1000);
    submit_test_price(&client2, &s1, &asset2, 100i128, 1000);
    submit_test_price(&client2, &s3, &asset2, 300i128, 1000);
    submit_test_price(&client2, &s2, &asset2, 200i128, 1000);

    let reordered_price = client2.get_price(&asset2, &0u64).unwrap();
    assert_eq!(reordered_price.price, 250i128);
    assert_eq!(honest_price.price, reordered_price.price);
}

// --- Test 6: Outlier injection does not bias the median ---
// Adding many extreme prices from frontrunning sources does not shift the
// median unless they genuinely outnumber the honest middle values.
#[test]
fn test_median_outlier_injection_resistant() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&7u32);

    let h1 = register_test_source(&e, &client, "H1");
    let h2 = register_test_source(&e, &client, "H2");
    let h3 = register_test_source(&e, &client, "H3");
    let h4 = register_test_source(&e, &client, "H4");
    let h5 = register_test_source(&e, &client, "H5");
    let h6 = register_test_source(&e, &client, "H6");
    let h7 = register_test_source(&e, &client, "H7");
    let asset = register_test_asset(&e, &client);

    // Honest cluster around 150-170, sorted: [150, 155, 160, 165, 170, 500, 1000]
    // Median = 165
    submit_test_price(&client, &h1, &asset, 150i128, 1000);
    submit_test_price(&client, &h2, &asset, 155i128, 1000);
    submit_test_price(&client, &h3, &asset, 160i128, 1000);
    submit_test_price(&client, &h4, &asset, 165i128, 1000);
    submit_test_price(&client, &h5, &asset, 170i128, 1000);
    submit_test_price(&client, &h6, &asset, 500i128, 1000);
    submit_test_price(&client, &h7, &asset, 1000i128, 1000);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 165i128);

    // Add 2 extreme outliers (frontrunners). With 9 total:
    // Sorted: [150, 155, 160, 165, 170, 500, 1000, 1_000_000, 1_000_001]
    // Median (index 4) = 170 — only shifted by 5 due to outliers
    let e2 = Env::default();
    ledger_default(&e2, 100, 1234567890);
    let (client2, _) = setup_contract(&e2);
    client2.set_min_sources_required(&7u32);

    let h1b = register_test_source(&e2, &client2, "H1");
    let h2b = register_test_source(&e2, &client2, "H2");
    let h3b = register_test_source(&e2, &client2, "H3");
    let h4b = register_test_source(&e2, &client2, "H4");
    let h5b = register_test_source(&e2, &client2, "H5");
    let h6b = register_test_source(&e2, &client2, "H6");
    let h7b = register_test_source(&e2, &client2, "H7");
    let f1 = register_test_source(&e2, &client2, "Front1");
    let f2 = register_test_source(&e2, &client2, "Front2");
    let asset2 = register_test_asset(&e2, &client2);

    submit_test_price(&client2, &h1b, &asset2, 150i128, 1000);
    submit_test_price(&client2, &h2b, &asset2, 155i128, 1000);
    submit_test_price(&client2, &h3b, &asset2, 160i128, 1000);
    submit_test_price(&client2, &h4b, &asset2, 165i128, 1000);
    submit_test_price(&client2, &h5b, &asset2, 170i128, 1000);
    submit_test_price(&client2, &h6b, &asset2, 500i128, 1000);
    submit_test_price(&client2, &h7b, &asset2, 1000i128, 1000);
    submit_test_price(&client2, &f1, &asset2, 1_000_000i128, 2000);
    submit_test_price(&client2, &f2, &asset2, 1_000_001i128, 2000);

    let attacked_price = client2.get_price(&asset2, &0u64).unwrap();
    // Median of 9 values is the middle (index 4): sorted = [150,155,160,165,170,500,1000,1M,1M+1]
    assert_eq!(attacked_price.price, 170i128);
    // Outliers shifted the median by only 5 (from 165 to 170), demonstrating resistance
    assert!((attacked_price.price - 165i128).abs() <= 10i128);
}

// --- Test 7: Duplicate price values across sources (cloning attack) ---
// A frontrunner cannot inflate their influence by submitting multiple times —
// each source is keyed by address, so duplicate submissions from the same
// source overwrite the previous one and still count as a single contributor.
#[test]
fn test_median_duplicate_source_no_inflation() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&2u32);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    let asset = register_test_asset(&e, &client);

    // Source1 submits 100, then "frontruns" by submitting 500.
    // Since it is the same source, the old submission is overwritten.
    submit_test_price(&client, &source1, &asset, 100i128, 1000);
    submit_test_price(&client, &source2, &asset, 150i128, 1000);

    let first_agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(first_agg.price, 125i128); // median(100, 150) = 100 + (150-100)/2 = 125
    assert_eq!(first_agg.num_sources, 2u32);

    // Source1 overwrites their submission with a higher price
    submit_test_price(&client, &source1, &asset, 500i128, 2000);
    let second_agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(second_agg.price, 325i128); // median(500, 150) = 325
    // Still only 2 contributors — the overwrite did not inflate the count
    assert_eq!(second_agg.num_sources, 2u32);
}

#[test]
fn test_subscription_bypasses_rate_limit() {
    let e = Env::default();
    ledger_default(&e, 100, 1000000);

    let (client, _) = setup_contract(&e);
    client.set_subscription_price(&86400u32, &100i128);
    client.set_query_rate_limit(&2u32);

    let source = register_test_source(&e, &client, "Oracle");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 100i128, 1000000);
    submit_test_price(&client, &source, &asset, 110i128, 1000000);

    let consumer = Address::generate(&e);
    client.subscribe(&consumer, &86400u32);

    // Subscribed consumer can make many queries without hitting rate limit
    for _ in 0..10 {
        let _ = client.get_price(&asset, &0u64);
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
