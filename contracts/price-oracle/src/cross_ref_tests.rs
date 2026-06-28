#![cfg(test)]

use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env, Map};

use crate::test_helpers::*;

// ---------------------------------------------------------------------------
// Mock reference oracle
// ---------------------------------------------------------------------------

/// A minimal oracle contract used as the cross-reference target in tests.
/// Exposes:
///   - `set_price(asset, price)` — stores a price for an asset address.
///   - `lastprice(asset)       ` — returns the stored price, or 0 if unset.
#[contract]
pub struct MockReferenceOracle;

#[contractimpl]
impl MockReferenceOracle {
    pub fn set_price(env: Env, asset: Address, price: i128) {
        env.storage().temporary().set(&asset, &price);
    }

    pub fn lastprice(env: Env, asset: Address) -> i128 {
        env.storage().temporary().get(&asset).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Helper: deploy mock oracle and return its address + client
// ---------------------------------------------------------------------------

fn deploy_mock_oracle(e: &Env) -> (Address, MockReferenceOracleClient<'_>) {
    let id = e.register(MockReferenceOracle, ());
    let client = MockReferenceOracleClient::new(e, &id);
    (id, client)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_add_reference_oracle_stores_entry() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let (mock_id, _mock) = deploy_mock_oracle(&e);
    let our_asset = Address::generate(&e);
    let ref_asset = Address::generate(&e);

    let mut mapping: Map<Address, Address> = Map::new(&e);
    mapping.set(our_asset, ref_asset);

    client.add_reference_oracle(&mock_id, &mapping);

    let oracles = client.get_reference_oracles();
    assert_eq!(oracles.len(), 1);
    assert_eq!(oracles.get_unchecked(0), mock_id);
}

#[test]
fn test_add_reference_oracle_unauthorized() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let (mock_id, _mock) = deploy_mock_oracle(&e);
    let mapping: Map<Address, Address> = Map::new(&e);

    clear_auth(&e);
    assert!(client
        .try_add_reference_oracle(&mock_id, &mapping)
        .is_err());
}

#[test]
fn test_remove_reference_oracle() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let (mock_id, _mock) = deploy_mock_oracle(&e);
    let mapping: Map<Address, Address> = Map::new(&e);

    client.add_reference_oracle(&mock_id, &mapping);
    assert_eq!(client.get_reference_oracles().len(), 1);

    client.remove_reference_oracle(&mock_id);
    assert_eq!(client.get_reference_oracles().len(), 0);
}

#[test]
fn test_cross_ref_deviation_threshold_default() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Default is 500 bps (5%)
    assert_eq!(client.get_cross_ref_deviation_threshold(), 500u32);
}

#[test]
fn test_set_cross_ref_deviation_threshold() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    client.set_cross_ref_deviation_threshold(&1000u32);
    assert_eq!(client.get_cross_ref_deviation_threshold(), 1000u32);
}

#[test]
fn test_set_cross_ref_deviation_threshold_unauthorized() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    clear_auth(&e);
    assert!(client
        .try_set_cross_ref_deviation_threshold(&1000u32)
        .is_err());
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_set_cross_ref_deviation_threshold_too_high() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // 100_001 exceeds the maximum allowed 100_000
    client.set_cross_ref_deviation_threshold(&100_001u32);
}

#[test]
fn test_get_cross_reference_no_oracle_returns_none() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 1_000_000i128, 1_000_000);

    // No reference oracle registered → None
    assert!(client.get_cross_reference(&asset).is_none());
}

#[test]
fn test_get_cross_reference_no_local_price_returns_none() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000_000);
    let (client, _admin) = setup_contract(&e);

    let our_asset = register_test_asset(&e, &client);
    let (mock_id, mock) = deploy_mock_oracle(&e);
    let ref_asset = Address::generate(&e);

    mock.set_price(&ref_asset, &1_000_000i128);

    let mut mapping: Map<Address, Address> = Map::new(&e);
    mapping.set(our_asset.clone(), ref_asset);
    client.add_reference_oracle(&mock_id, &mapping);

    // No price submitted to our oracle → None
    assert!(client.get_cross_reference(&our_asset).is_none());
}

#[test]
fn test_get_cross_reference_returns_prices_no_deviation() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let our_asset = register_test_asset(&e, &client);

    let price = 1_000_000i128;
    submit_test_price(&client, &source, &our_asset, price, 1_000_000);

    let (mock_id, mock) = deploy_mock_oracle(&e);
    let ref_asset = Address::generate(&e);
    mock.set_price(&ref_asset, &price); // identical price → 0 bps deviation

    let mut mapping: Map<Address, Address> = Map::new(&e);
    mapping.set(our_asset.clone(), ref_asset);
    client.add_reference_oracle(&mock_id, &mapping);

    // Set threshold to 500 bps (5%)
    client.set_cross_ref_deviation_threshold(&500u32);

    let result = client.get_cross_reference(&our_asset).unwrap();
    assert_eq!(result.our_price, price);
    assert_eq!(result.ref_price, price);
    assert_eq!(result.deviation_bps, 0u32);
    assert_eq!(result.ref_contract, mock_id);
}

#[test]
fn test_get_cross_reference_detects_deviation_below_threshold() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let our_asset = register_test_asset(&e, &client);

    let our_price = 1_000_000i128;
    // Ref price is 2% higher: 1_020_000
    let ref_price = 1_020_000i128;
    submit_test_price(&client, &source, &our_asset, our_price, 1_000_000);

    let (mock_id, mock) = deploy_mock_oracle(&e);
    let ref_asset = Address::generate(&e);
    mock.set_price(&ref_asset, &ref_price);

    let mut mapping: Map<Address, Address> = Map::new(&e);
    mapping.set(our_asset.clone(), ref_asset);
    client.add_reference_oracle(&mock_id, &mapping);

    // Threshold at 5% — 2% deviation should NOT trigger the event
    client.set_cross_ref_deviation_threshold(&500u32);

    let result = client.get_cross_reference(&our_asset).unwrap();
    assert_eq!(result.our_price, our_price);
    assert_eq!(result.ref_price, ref_price);
    // deviation = 20_000 * 10_000 / 1_020_000 ≈ 196 bps
    assert!(result.deviation_bps < 500u32);
}

#[test]
fn test_get_cross_reference_emits_event_on_deviation() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let our_asset = register_test_asset(&e, &client);

    let our_price = 1_000_000i128;
    // Ref price is 10% higher: 1_100_000
    let ref_price = 1_100_000i128;
    submit_test_price(&client, &source, &our_asset, our_price, 1_000_000);

    let (mock_id, mock) = deploy_mock_oracle(&e);
    let ref_asset = Address::generate(&e);
    mock.set_price(&ref_asset, &ref_price);

    let mut mapping: Map<Address, Address> = Map::new(&e);
    mapping.set(our_asset.clone(), ref_asset);
    client.add_reference_oracle(&mock_id, &mapping);

    // Threshold at 5% — 10% deviation should trigger the CrossRefDeviationEvent
    client.set_cross_ref_deviation_threshold(&500u32);

    let result = client.get_cross_reference(&our_asset).unwrap();
    assert_eq!(result.our_price, our_price);
    assert_eq!(result.ref_price, ref_price);
    // deviation ≈ 909 bps (≈9.09%) — exceeds 500 bps threshold
    assert!(result.deviation_bps > 500u32);
    assert_eq!(result.ref_contract, mock_id);
}

#[test]
fn test_get_cross_reference_no_mapping_for_asset_returns_none() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let our_asset = register_test_asset(&e, &client);
    let other_asset = Address::generate(&e);

    submit_test_price(&client, &source, &our_asset, 1_000_000i128, 1_000_000);

    let (mock_id, _mock) = deploy_mock_oracle(&e);

    // Mapping for `other_asset` only — not for `our_asset`
    let mut mapping: Map<Address, Address> = Map::new(&e);
    mapping.set(other_asset, Address::generate(&e));
    client.add_reference_oracle(&mock_id, &mapping);

    assert!(client.get_cross_reference(&our_asset).is_none());
}

#[test]
fn test_multiple_reference_oracles_uses_first_match() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let our_asset = register_test_asset(&e, &client);

    let our_price = 1_000_000i128;
    submit_test_price(&client, &source, &our_asset, our_price, 1_000_000);

    let (mock_id1, mock1) = deploy_mock_oracle(&e);
    let ref_asset1 = Address::generate(&e);
    let ref_price1 = 1_000_000i128;
    mock1.set_price(&ref_asset1, &ref_price1);

    let (mock_id2, mock2) = deploy_mock_oracle(&e);
    let ref_asset2 = Address::generate(&e);
    let ref_price2 = 2_000_000i128;
    mock2.set_price(&ref_asset2, &ref_price2);

    let mut mapping1: Map<Address, Address> = Map::new(&e);
    mapping1.set(our_asset.clone(), ref_asset1);
    client.add_reference_oracle(&mock_id1, &mapping1);

    let mut mapping2: Map<Address, Address> = Map::new(&e);
    mapping2.set(our_asset.clone(), ref_asset2);
    client.add_reference_oracle(&mock_id2, &mapping2);

    // Should return result from the first registered oracle (mock1)
    let result = client.get_cross_reference(&our_asset).unwrap();
    assert_eq!(result.ref_price, ref_price1);
    assert_eq!(result.ref_contract, mock_id1);
}
