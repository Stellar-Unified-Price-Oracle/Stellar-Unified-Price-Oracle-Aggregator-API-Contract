//! Integration test suite for SEP-40 consumer compatibility (#101).
//!
//! Tests the oracle contract against simulated SEP-40 consumer contracts to
//! ensure the standard interface (`base`, `assets`, `decimals`, `resolution`,
//! `lastprice`, `price`, `prices`) behaves correctly from a consumer's perspective.

#![cfg(test)]

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, Symbol, Vec,
};

use crate::test_helpers::{
    ledger_default, register_test_asset, register_test_source, setup_contract,
};
use crate::{Asset, PriceData, PriceOracleContractClient};

// ---------------------------------------------------------------------------
// Minimal mock SEP-40 consumer — calls oracle and validates results
// ---------------------------------------------------------------------------

/// A minimal consumer contract that exercises the SEP-40 interface.
#[contract]
pub struct MockSep40Consumer;

#[contractimpl]
impl MockSep40Consumer {
    /// Reads the oracle base asset and returns its USD symbol.
    pub fn read_base(env: Env, oracle: Address) -> bool {
        let client = PriceOracleContractClient::new(&env, &oracle);
        let base = client.base();
        matches!(base, Asset::Other(ref sym) if *sym == Symbol::new(&env, "USD"))
    }

    /// Reads the oracle assets list and returns its length.
    pub fn read_assets_count(env: Env, oracle: Address) -> u32 {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.assets().len()
    }

    /// Reads decimals and returns the value.
    pub fn read_decimals(env: Env, oracle: Address) -> u32 {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.decimals()
    }

    /// Reads resolution and returns the value.
    pub fn read_resolution(env: Env, oracle: Address) -> u32 {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.resolution()
    }

    /// Calls lastprice and returns the price value or 0 if None.
    pub fn read_lastprice(env: Env, oracle: Address, asset: Address) -> Option<PriceData> {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.lastprice(&Asset::Stellar(asset))
    }

    /// Calls price(asset, timestamp).
    pub fn read_price_at(
        env: Env,
        oracle: Address,
        asset: Address,
        timestamp: u64,
    ) -> Option<PriceData> {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.price(&Asset::Stellar(asset), &timestamp)
    }

    /// Calls prices(asset, records).
    pub fn read_prices(
        env: Env,
        oracle: Address,
        asset: Address,
        records: u32,
    ) -> Option<Vec<PriceData>> {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.prices(&Asset::Stellar(asset), &records)
    }
}

// ---------------------------------------------------------------------------
// Helper: set up oracle + consumer in a shared environment
// ---------------------------------------------------------------------------

fn setup_consumer_oracle(
    e: &Env,
) -> (
    PriceOracleContractClient,
    Address, // oracle contract id
    Address, // consumer contract id
) {
    e.mock_all_auths();
    let (client, _admin) = setup_contract(e);
    let oracle_id = client.address.clone();
    let consumer_id = e.register(MockSep40Consumer, ());
    (client, oracle_id, consumer_id)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_consumer_reads_base() {
    let e = Env::default();
    let (_, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    let result = consumer.read_base(&oracle_id);
    assert!(result, "base() should return Asset::Other(USD)");
}

#[test]
fn test_consumer_reads_assets() {
    let e = Env::default();
    let (client, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    // No assets yet
    assert_eq!(consumer.read_assets_count(&oracle_id), 0u32);

    // Register two assets
    register_test_asset(&e, &client);
    register_test_asset(&e, &client);

    assert_eq!(consumer.read_assets_count(&oracle_id), 2u32);
}

#[test]
fn test_consumer_reads_decimals() {
    let e = Env::default();
    let (_, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    // Default decimals from setup_contract is 18
    assert_eq!(consumer.read_decimals(&oracle_id), 18u32);
}

#[test]
fn test_consumer_reads_resolution() {
    let e = Env::default();
    let (client, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    // Default resolution is 0 (no staleness window)
    assert_eq!(consumer.read_resolution(&oracle_id), 0u32);

    // Admin sets resolution
    client.set_resolution(&3600u32);
    assert_eq!(consumer.read_resolution(&oracle_id), 3600u32);
}

#[test]
fn test_consumer_calls_lastprice() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000_000);
    let (client, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Feed1");
    let asset = register_test_asset(&e, &client);

    // No price yet — should return None
    assert!(consumer.read_lastprice(&oracle_id, &asset).is_none());

    // Submit a price
    client.submit_price(&source, &asset, &1_000_000i128, &1_000_000u64);

    // Now lastprice should return the price
    let pd = consumer.read_lastprice(&oracle_id, &asset).unwrap();
    assert_eq!(pd.price, 1_000_000i128);
    assert_eq!(pd.timestamp, 1_000_000u64);
}

#[test]
fn test_consumer_calls_price_at_timestamp() {
    let e = Env::default();
    ledger_default(&e, 200, 2_000_000);
    let (client, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Feed1");
    let asset = register_test_asset(&e, &client);

    client.submit_price(&source, &asset, &5_000i128, &2_000_000u64);

    // Exact timestamp match
    let pd = consumer
        .read_price_at(&oracle_id, &asset, &2_000_000u64)
        .unwrap();
    assert_eq!(pd.price, 5_000i128);

    // Timestamp in the future relative to the submission — should still resolve
    let pd2 = consumer
        .read_price_at(&oracle_id, &asset, &2_999_999u64)
        .unwrap();
    assert_eq!(pd2.price, 5_000i128);
}

#[test]
fn test_consumer_calls_prices_multiple_records() {
    let e = Env::default();
    let (client, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Feed1");
    let asset = register_test_asset(&e, &client);

    // Submit prices at three different ledgers
    for seq in [100u32, 101, 102] {
        e.ledger().set(LedgerInfo {
            timestamp: seq as u64 * 10,
            protocol_version: 26,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 4096,
        });
        client.submit_price(&source, &asset, &(seq as i128 * 100), &(seq as u64 * 10));
    }

    let records = consumer.read_prices(&oracle_id, &asset, &3u32).unwrap();
    assert_eq!(records.len(), 3u32);
}

#[test]
fn test_consumer_prices_zero_records() {
    let e = Env::default();
    let (client, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "Feed1");
    let asset = register_test_asset(&e, &client);
    client.submit_price(&source, &asset, &100i128, &0u64);

    // Requesting 0 records should return an empty vec, not None
    let records = consumer.read_prices(&oracle_id, &asset, &0u32).unwrap();
    assert_eq!(records.len(), 0u32);
}

#[test]
fn test_consumer_lastprice_non_stellar_asset_returns_none() {
    let e = Env::default();
    let (client, _oracle_id, _consumer_id) = setup_consumer_oracle(&e);

    // SEP-40: lastprice on Asset::Other should return None
    let result = client.lastprice(&Asset::Other(Symbol::new(&e, "BTC")));
    assert!(result.is_none());
}

#[test]
fn test_consumer_price_unregistered_asset_returns_none() {
    let e = Env::default();
    let (client, _oracle_id, _consumer_id) = setup_consumer_oracle(&e);

    let random_addr = Address::generate(&e);
    // Not registered — should return None (not panic)
    let result = client.price(&Asset::Stellar(random_addr), &0u64);
    assert!(result.is_none());
}

#[test]
fn test_consumer_handles_resolution_staleness() {
    let e = Env::default();
    ledger_default(&e, 100, 1000);
    let (client, oracle_id, consumer_id) = setup_consumer_oracle(&e);
    let consumer = MockSep40ConsumerClient::new(&e, &consumer_id);

    client.set_min_sources_required(&1u32);
    client.set_resolution(&60u32); // 60 second window
    let source = register_test_source(&e, &client, "Feed1");
    let asset = register_test_asset(&e, &client);

    // Submit price at t=1000
    client.submit_price(&source, &asset, &999i128, &1000u64);

    // At t=1000 price is fresh
    assert!(consumer.read_lastprice(&oracle_id, &asset).is_some());

    // Advance ledger time beyond the resolution window
    ledger_default(&e, 200, 2000);

    // Price is now stale — consumer should receive None
    assert!(consumer.read_lastprice(&oracle_id, &asset).is_none());
}
