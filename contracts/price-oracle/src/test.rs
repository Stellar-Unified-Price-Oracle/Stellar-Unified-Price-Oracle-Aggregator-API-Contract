#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Bytes, Env, String, Symbol, Vec,
};

use crate::{Asset, PriceData, PriceEntry, PriceOracleContract, PriceOracleContractClient};

fn create_contract(e: &Env) -> PriceOracleContractClient<'_> {
    e.mock_all_auths();
    let contract_id = e.register(PriceOracleContract, ());
    PriceOracleContractClient::new(e, &contract_id)
}

fn clear_auth(e: &Env) {
    use soroban_sdk::xdr::SorobanAuthorizationEntry;
    e.set_auths(&[] as &[SorobanAuthorizationEntry]);
}

fn init_admin(client: &PriceOracleContractClient<'_>, admin: &Address) {
    client.initialize(
        admin,
        &2u32,
        &10u32,
        &18u32,
        &String::from_str(&client.env, "Stellar Price Oracle Aggregator"),
    );
}

fn setup_basic(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address, Address) {
    let admin = Address::generate(e);
    let client = create_contract(e);
    init_admin(&client, &admin);

    let source1 = Address::generate(e);
    let source2 = Address::generate(e);
    let asset1 = Address::generate(e);

    client.add_source(&source1, &String::from_str(e, "Chainlink"));
    client.add_source(&source2, &String::from_str(e, "Band"));
    client.register_asset(&asset1);

    (client, admin, source1, asset1)
}

fn ledger_default(e: &Env, seq: u32, timestamp: u64) {
    e.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 4096,
    });
}

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
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let new_admin = Address::generate(&e);
    client.set_admin(&new_admin);
    assert_eq!(client.get_admin_address(), new_admin);
}

#[test]
fn test_set_admin_unauthorized() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let new_admin = Address::generate(&e);
    clear_auth(&e);
    assert!(client.try_set_admin(&new_admin).is_err());
}

#[test]
fn test_admin_functions() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

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
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.register_asset(&asset);
    assert!(client.is_asset_registered(&asset));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_register_asset_twice() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.register_asset(&asset);
    client.register_asset(&asset);
}

#[test]
fn test_unregister_asset() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.register_asset(&asset);
    assert!(client.is_asset_registered(&asset));

    client.unregister_asset(&asset);
    assert!(!client.is_asset_registered(&asset));
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_unregister_unregistered_asset() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.unregister_asset(&asset);
}

#[test]
fn test_add_remove_source() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "Chainlink"));
    assert!(client.is_source(&source));

    client.remove_source(&source);
    assert!(!client.is_source(&source));
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_add_source_twice() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "Chainlink"));
    client.add_source(&source, &String::from_str(&e, "Chainlink"));
}

#[test]
#[should_panic(expected = "Error(Contract, #5)")]
fn test_remove_nonexistent_source() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source = Address::generate(&e);
    client.remove_source(&source);
}

#[test]
fn test_get_oracle_sources() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));

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

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);

    // Only one source submitted, min_sources=2 → not aggregated yet → None
    assert!(client.get_price(&asset, &0u64).is_none());

    client.submit_price(&source2, &asset, &110i128, &1234567890);

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

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);
    let source3 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.add_source(&source3, &String::from_str(&e, "Redstone"));
    client.set_min_sources_required(&3u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &200i128, &1234567890);
    client.submit_price(&source3, &asset, &300i128, &1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 200i128);
    assert_eq!(price.num_sources, 3u32);
}

#[test]
fn test_submit_price_median_even() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);
    let source3 = Address::generate(&e);
    let source4 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "A"));
    client.add_source(&source2, &String::from_str(&e, "B"));
    client.add_source(&source3, &String::from_str(&e, "C"));
    client.add_source(&source4, &String::from_str(&e, "D"));
    client.set_min_sources_required(&4u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &200i128, &1234567890);
    client.submit_price(&source3, &asset, &300i128, &1234567890);
    client.submit_price(&source4, &asset, &400i128, &1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 250i128);
    assert_eq!(price.num_sources, 4u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_submit_price_unauthorized_source() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let fake_source = Address::generate(&e);
    let asset = Address::generate(&e);

    client.register_asset(&asset);

    client.submit_price(&fake_source, &asset, &100i128, &1234567890);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_submit_price_invalid_zero() {
    let e = Env::default();
    let (client, _admin, source1, asset1) = setup_basic(&e);

    client.submit_price(&source1, &asset1, &0i128, &1234567890);
}

#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_submit_price_invalid_negative() {
    let e = Env::default();
    let (client, _admin, source1, asset1) = setup_basic(&e);

    client.submit_price(&source1, &asset1, &(-100i128), &1234567890);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_submit_price_unregistered_asset() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    client.add_source(&source1, &String::from_str(&e, "Chainlink"));

    let unregistered_asset = Address::generate(&e);
    client.submit_price(&source1, &unregistered_asset, &100i128, &1234567890);
}

#[test]
fn test_get_source_price() {
    let e = Env::default();
    let (client, _admin, source1, asset1) = setup_basic(&e);

    client.submit_price(&source1, &asset1, &100i128, &1234567890);

    let entry = client.get_source_price(&asset1, &source1);
    assert_eq!(entry.price, 100i128);
    assert_eq!(entry.timestamp, 1234567890u64);
    assert_eq!(entry.source, source1);
    assert_eq!(entry.decimals, 18u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_get_source_price_nonexistent_source() {
    let e = Env::default();
    let (client, _admin, _source1, asset1) = setup_basic(&e);

    let fake_source = Address::generate(&e);
    client.get_source_price(&asset1, &fake_source);
}

#[test]
fn test_get_all_prices() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &200i128, &1234567890);

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

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);
    assert_eq!(client.get_latest_ledger(), 42u32);
}

#[test]
fn test_get_price_no_data() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    // No prices submitted → None
    assert!(client.get_price(&asset, &0u64).is_none());
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_get_price_unregistered_asset() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.get_price(&asset, &0u64);
}

#[test]
fn test_historical_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &110i128, &1234567890);

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
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);
    let source3 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.add_source(&source3, &String::from_str(&e, "Redstone"));
    client.set_min_sources_required(&3u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    ledger_default(&e, 100, 1234567890);
    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &200i128, &1234567890);
    client.submit_price(&source3, &asset, &300i128, &1234567890);

    ledger_default(&e, 101, 1234567891);
    client.submit_price(&source1, &asset, &110i128, &1234567891);
    client.submit_price(&source2, &asset, &210i128, &1234567891);
    client.submit_price(&source3, &asset, &310i128, &1234567891);

    ledger_default(&e, 102, 1234567892);
    client.submit_price(&source1, &asset, &120i128, &1234567892);
    client.submit_price(&source2, &asset, &220i128, &1234567892);
    client.submit_price(&source3, &asset, &320i128, &1234567892);

    let history_range = client.get_historical_prices(&asset, &100u32, &102u32);
    assert_eq!(history_range.len(), 3);
    assert_eq!(history_range.get_unchecked(0).price, 200i128);
    assert_eq!(history_range.get_unchecked(1).price, 210i128);
    assert_eq!(history_range.get_unchecked(2).price, 220i128);
}

#[test]
fn test_has_historical_price_false() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    assert!(!client.has_historical_price(&asset, &999u32));
}

#[test]
fn test_has_historical_price_unregistered_asset() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    assert!(!client.has_historical_price(&asset, &100u32));
}

#[test]
fn test_upgrade() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let wasm = include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm");
    let new_wasm_hash = e
        .deployer()
        .upload_contract_wasm(Bytes::from_slice(&e, wasm));
    client.upgrade(&new_wasm_hash);
}

#[test]
fn test_upgrade_unauthorized() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

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
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source = Address::generate(&e);
    clear_auth(&e);
    assert!(client
        .try_add_source(&source, &String::from_str(&e, "Bad Source"))
        .is_err());
}

#[test]
fn test_unauthorized_remove_source() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "Test"));

    clear_auth(&e);
    assert!(client.try_remove_source(&source).is_err());
}

#[test]
fn test_unauthorized_set_min_sources() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    clear_auth(&e);
    assert!(client.try_set_min_sources_required(&5u32).is_err());
}

#[test]
fn test_unauthorized_set_max_history() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    clear_auth(&e);
    assert!(client.try_set_max_history_length(&50u32).is_err());
}

#[test]
fn test_unauthorized_set_decimals() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    clear_auth(&e);
    assert!(client.try_set_decimals(&8u32).is_err());
}

#[test]
fn test_unauthorized_set_description() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    clear_auth(&e);
    assert!(client
        .try_set_description(&String::from_str(&e, "Hacked"))
        .is_err());
}

#[test]
fn test_multiple_assets() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let xlm = Address::generate(&e);
    let eth = Address::generate(&e);
    let btc = Address::generate(&e);

    client.register_asset(&xlm);
    client.register_asset(&eth);
    client.register_asset(&btc);

    client.submit_price(&source1, &xlm, &100i128, &1234567890);
    client.submit_price(&source2, &xlm, &102i128, &1234567890);

    client.submit_price(&source1, &eth, &180000i128, &1234567890);
    client.submit_price(&source2, &eth, &181000i128, &1234567890);

    client.submit_price(&source1, &btc, &30000000i128, &1234567890);
    client.submit_price(&source2, &btc, &31000000i128, &1234567890);

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

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1000u64);
    client.submit_price(&source2, &asset, &110i128, &2000u64);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.timestamp, 2000u64);

    client.submit_price(&source2, &asset, &120i128, &3000u64);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.timestamp, 3000u64);
}

#[test]
fn test_single_source_no_aggregation() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.set_min_sources_required(&1u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 100i128);
    assert_eq!(price.num_sources, 1u32);
}

#[test]
fn test_price_source_not_affected_by_other_assets() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let asset_a = Address::generate(&e);
    let asset_b = Address::generate(&e);
    client.register_asset(&asset_a);
    client.register_asset(&asset_b);

    client.submit_price(&source1, &asset_a, &100i128, &1234567890);
    client.submit_price(&source2, &asset_a, &110i128, &1234567890);

    let price_a = client.get_price(&asset_a, &0u64).unwrap();
    assert_eq!(price_a.price, 105i128);

    // asset_b has no submissions → None
    assert!(client.get_price(&asset_b, &0u64).is_none());
}

// ---- SEP-40 Oracle Interface Tests ----

#[test]
fn test_sep40_base() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let result = client.base();
    assert_eq!(result, Asset::Other(Symbol::new(&e, "USD")));
}

#[test]
fn test_sep40_assets() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset1 = Address::generate(&e);
    let asset2 = Address::generate(&e);
    client.register_asset(&asset1);
    client.register_asset(&asset2);

    let assets = client.assets();
    assert_eq!(assets.len(), 2);
    assert_eq!(assets.get_unchecked(0), Asset::Stellar(asset1));
    assert_eq!(assets.get_unchecked(1), Asset::Stellar(asset2));
}

#[test]
fn test_sep40_resolution() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    assert_eq!(client.resolution(), 0u32);

    client.set_resolution(&300u32);
    assert_eq!(client.resolution(), 300u32);
}

#[test]
fn test_sep40_lastprice() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);
    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &110i128, &1234567890);

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

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let unregistered = Address::generate(&e);
    let result = client.lastprice(&Asset::Stellar(unregistered));
    assert!(result.is_none());
}

#[test]
fn test_sep40_lastprice_other() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let result = client.lastprice(&Asset::Other(Symbol::new(&e, "EUR")));
    assert!(result.is_none());
}

#[test]
fn test_sep40_lastprice_stale() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.set_min_sources_required(&1u32);
    client.set_resolution(&10u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);

    // Advance ledger past resolution window
    ledger_default(&e, 200, 1234567910);
    let result = client.lastprice(&Asset::Stellar(asset));
    assert!(result.is_none());
}

#[test]
fn test_sep40_price() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);
    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.set_min_sources_required(&2u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &110i128, &1234567890);

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

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.set_min_sources_required(&1u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1000);

    // Query with timestamp before data exists → should find no match
    let result = client.price(&Asset::Stellar(asset), &999u64);
    assert!(result.is_none());
}

#[test]
fn test_sep40_price_other() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let result = client.price(&Asset::Other(Symbol::new(&e, "BTC")), &1234567890u64);
    assert!(result.is_none());
}

#[test]
fn test_sep40_prices() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);
    let source3 = Address::generate(&e);

    client.add_source(&source1, &String::from_str(&e, "Chainlink"));
    client.add_source(&source2, &String::from_str(&e, "Band"));
    client.add_source(&source3, &String::from_str(&e, "Redstone"));
    client.set_min_sources_required(&3u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    client.submit_price(&source1, &asset, &100i128, &1234567890);
    client.submit_price(&source2, &asset, &200i128, &1234567890);
    client.submit_price(&source3, &asset, &300i128, &1234567890);

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

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    let result = client.prices(&Asset::Stellar(asset), &5u32);
    assert!(result.is_some());
    let prices: Vec<PriceData> = result.unwrap();
    // Falls back to aggregate entry with price 0 when no history exists
    assert_eq!(prices.len(), 1);
    assert_eq!(prices.get_unchecked(0).price, 0i128);
}

#[test]
fn test_sep40_prices_unregistered_asset() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

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

// ---- Event emission tests ----

#[test]
fn test_event_source_added() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "Chainlink"));

    let events = e.events().all();
    // Last event should be SourceAddedEvent
    assert!(!events.is_empty());
    let last = events.get_unchecked(events.len() - 1);
    // topics: [event_name_sym, source, admin]; data: name
    let (_, topics, _): (Address, soroban_sdk::Vec<soroban_sdk::Val>, soroban_sdk::Val) = last;
    // First topic is the event type symbol "SourceAddedEvent", second is source, third is admin
    assert_eq!(topics.len(), 3);
}

#[test]
fn test_event_source_removed() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "Test"));

    let events_before = e.events().all().len();
    client.remove_source(&source);

    let events = e.events().all();
    assert_eq!(events.len(), events_before + 1);
}

#[test]
fn test_event_asset_registered() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    let events_before = e.events().all().len();
    client.register_asset(&asset);

    let events = e.events().all();
    assert_eq!(events.len(), events_before + 1);
}

#[test]
fn test_event_asset_unregistered() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    let events_before = e.events().all().len();
    client.unregister_asset(&asset);

    let events = e.events().all();
    assert_eq!(events.len(), events_before + 1);
}

#[test]
fn test_event_price_submitted() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, _admin, source1, asset1) = setup_basic(&e);

    let events_before = e.events().all().len();
    client.submit_price(&source1, &asset1, &100i128, &1000u64);

    let events = e.events().all();
    // At minimum PriceSubmittedEvent was emitted (price_updated may also fire)
    assert!(events.len() > events_before);
}

#[test]
fn test_event_price_updated_emitted_on_aggregate_change() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    let source2 = Address::generate(&e);
    client.add_source(&source1, &String::from_str(&e, "A"));
    client.add_source(&source2, &String::from_str(&e, "B"));
    client.set_min_sources_required(&2u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    // Submit two prices so aggregation fires and price_updated is emitted
    client.submit_price(&source1, &asset, &100i128, &1000u64);
    let events_after_first = e.events().all().len();

    client.submit_price(&source2, &asset, &200i128, &1000u64);
    let events_after_second = e.events().all().len();

    // Second submit triggers aggregation → PriceSubmittedEvent + PriceUpdatedEvent
    assert!(events_after_second > events_after_first + 1);
}

#[test]
fn test_event_price_updated_not_emitted_when_unchanged() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);

    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let source1 = Address::generate(&e);
    client.add_source(&source1, &String::from_str(&e, "A"));
    client.set_min_sources_required(&1u32);

    let asset = Address::generate(&e);
    client.register_asset(&asset);

    // First submit sets price to 100
    client.submit_price(&source1, &asset, &100i128, &1000u64);
    let events_after_first = e.events().all().len();

    // Second submit with same price and same timestamp — no change, no price_updated
    client.submit_price(&source1, &asset, &100i128, &1000u64);
    let events_after_second = e.events().all().len();

    // Only PriceSubmittedEvent added, no PriceUpdatedEvent
    assert_eq!(events_after_second, events_after_first + 1);
}

#[test]
fn test_event_admin_changed() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    init_admin(&client, &admin);

    let new_admin = Address::generate(&e);
    let events_before = e.events().all().len();
    client.set_admin(&new_admin);

    let events = e.events().all();
    assert_eq!(events.len(), events_before + 1);
    // Topics: [event_sym, old_admin, new_admin]
    let (_, topics, _): (Address, soroban_sdk::Vec<soroban_sdk::Val>, soroban_sdk::Val) =
        events.get_unchecked(events.len() - 1);
    assert_eq!(topics.len(), 3);
}
