//! Tests for Issue #351 — Formal Verification of Median and Aggregation Math

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env,
};

use crate::test_helpers::{setup_contract, register_test_source, register_test_asset};

fn set_ledger(e: &Env, seq: u32, ts: u64) {
    e.ledger().set(LedgerInfo {
        timestamp: ts,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100,
        min_persistent_entry_ttl: 100,
        max_entry_ttl: 99_999,
    });
}

// ══════════════════════════════════════════════════════════════════════════════
// Issue #351 — Formal Verification of Median and Aggregation Math
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_median_of_single_price_returns_that_price() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&1u32);

    client.submit_price(&source, &asset, &1_000_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());
    let agg_price = price.unwrap();
    assert_eq!(agg_price.price, 1_000_000);
}

#[test]
fn test_median_of_odd_prices_returns_middle_value() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&3u32);

    client.submit_price(&source1, &asset, &1_000_000, &1_000);
    client.submit_price(&source2, &asset, &2_000_000, &1_000);
    client.submit_price(&source3, &asset, &3_000_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());
    let agg_price = price.unwrap();
    assert_eq!(agg_price.price, 2_000_000);
}

#[test]
fn test_median_of_even_prices_returns_average_of_middle_two() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let source4 = register_test_source(&e, &client, "Source 4");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&4u32);

    client.submit_price(&source1, &asset, &1_000_000, &1_000);
    client.submit_price(&source2, &asset, &2_000_000, &1_000);
    client.submit_price(&source3, &asset, &3_000_000, &1_000);
    client.submit_price(&source4, &asset, &4_000_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());
    let agg_price = price.unwrap();
    assert_eq!(agg_price.price, 2_500_000);
}

#[test]
fn test_median_with_outliers_ignores_extremes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let source4 = register_test_source(&e, &client, "Source 4");
    let source5 = register_test_source(&e, &client, "Source 5");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&5u32);

    client.submit_price(&source1, &asset, &100_000, &1_000);
    client.submit_price(&source2, &asset, &2_000_000, &1_000);
    client.submit_price(&source3, &asset, &2_100_000, &1_000);
    client.submit_price(&source4, &asset, &2_200_000, &1_000);
    client.submit_price(&source5, &asset, &100_000_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());
    let agg_price = price.unwrap();
    assert_eq!(agg_price.price, 2_100_000);
}

#[test]
fn test_no_overflow_with_large_price_values() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&3u32);

    let large_price = i128::MAX / 2;

    client.submit_price(&source1, &asset, &large_price, &1_000);
    client.submit_price(&source2, &asset, &large_price, &1_000);
    client.submit_price(&source3, &asset, &large_price, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());
    let agg_price = price.unwrap();
    assert_eq!(agg_price.price, large_price);
}

#[test]
fn test_negative_prices_handled_correctly() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&3u32);

    client.submit_price(&source1, &asset, &-1_000_000, &1_000);
    client.submit_price(&source2, &asset, &0, &1_000);
    client.submit_price(&source3, &asset, &1_000_000, &1_000);

    let price = client.get_price(&asset);
    assert!(price.is_some());
    let agg_price = price.unwrap();
    assert_eq!(agg_price.price, 0);
}

#[test]
fn test_aggregate_price_includes_all_source_prices() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&3u32);

    client.submit_price(&source1, &asset, &1_000_000, &1_000);
    client.submit_price(&source2, &asset, &2_000_000, &1_000);
    client.submit_price(&source3, &asset, &3_000_000, &1_000);

    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 3);
}

#[test]
fn test_per_source_price_retrieval_accuracy() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&1u32);

    let submitted_price = 1_234_567i128;
    client.submit_price(&source, &asset, &submitted_price, &1_000);

    let source_price = client.get_source_price(&asset, &source);
    assert!(source_price.is_some());
    let price_entry = source_price.unwrap();
    assert_eq!(price_entry.price, submitted_price);
}

#[test]
fn test_median_parity_verification() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let source4 = register_test_source(&e, &client, "Source 4");
    let source5 = register_test_source(&e, &client, "Source 5");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&5u32);

    let prices = vec![5_000_000, 1_000_000, 3_000_000, 2_000_000, 4_000_000];
    let sources = vec![&source1, &source2, &source3, &source4, &source5];

    for (i, (src, price)) in sources.iter().zip(prices.iter()).enumerate() {
        client.submit_price(src, &asset, price, &1_000);
    }

    let agg_price = client.get_price(&asset).unwrap();
    assert_eq!(agg_price.price, 3_000_000);
}

#[test]
fn test_price_timestamp_consistency_in_aggregation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    set_ledger(&e, 100, 1_000);

    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    client.set_min_sources_required(&3u32);

    let ts = 1_000u64;
    client.submit_price(&source1, &asset, &1_000_000, &ts);
    client.submit_price(&source2, &asset, &2_000_000, &ts);
    client.submit_price(&source3, &asset, &3_000_000, &ts);

    let agg_price = client.get_price(&asset).unwrap();
    assert_eq!(agg_price.timestamp, ts);
}
