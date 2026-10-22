#![cfg(test)]

use crate::test_helpers::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

/// Advances the ledger to `seq` and `ts` so consecutive submissions land in
/// distinct history buckets and stay within the timestamp threshold.
fn at(e: &Env, seq: u32, ts: u64) {
    e.ledger().with_mut(|l| {
        l.sequence_number = seq;
        l.timestamp = ts;
    });
}

#[test]
fn test_delta_encoding_basic() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    at(&e, 1, 100);
    submit_test_price(&client, &source, &asset, 1000, 100);
    at(&e, 2, 200);
    submit_test_price_n(&client, &source, &asset, 1050, 200, 2);

    let history = client.get_price_history(&asset, &0u32, &2u32);
    assert_eq!(history.len(), 2);
}

#[test]
fn test_delta_encoding_full_value() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 5000, 100);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 5000);
}

#[test]
fn test_delta_encoding_small_changes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    at(&e, 1, 100);
    submit_test_price(&client, &source, &asset, 10000, 100);
    at(&e, 2, 200);
    submit_test_price_n(&client, &source, &asset, 10001, 200, 2);
    at(&e, 3, 300);
    submit_test_price_n(&client, &source, &asset, 10002, 300, 3);

    let history = client.get_price_history(&asset, &0u32, &3u32);
    assert_eq!(history.len(), 3);

    assert_eq!(history.get(0).unwrap().price, 10000);
    assert_eq!(history.get(1).unwrap().price, 10001);
    assert_eq!(history.get(2).unwrap().price, 10002);
}

#[test]
fn test_delta_encoding_large_swings() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 100, 100);
    submit_test_price_n(&client, &source, &asset, 10000, 200, 2);
    submit_test_price_n(&client, &source, &asset, 500, 300, 3);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 500);
}

#[test]
fn test_delta_encoding_backward_compatible_reads() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 2000, 100);

    let price_from_current = client.get_price(&asset, &0u64).unwrap();
    let history = client.get_price_history(&asset, &0u32, &1u32);
    let price_from_history = history.get(0).unwrap();

    assert_eq!(price_from_current.price, price_from_history.price);
}

#[test]
fn test_delta_encoding_version_bit() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 3000, 100);
    submit_test_price_n(&client, &source, &asset, 3100, 200, 2);

    let history = client.get_price_history(&asset, &0u32, &2u32);
    assert!(history.len() >= 1);
}

#[test]
fn test_delta_encoding_stable_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    let stable_price = 1000000;
    for i in 0..10 {
        at(&e, i + 1, 100 + (i as u64 * 100));
        submit_test_price_n(
            &client,
            &source,
            &asset,
            stable_price,
            100 + (i as u64 * 100),
            (i + 1) as u64,
        );
    }

    let history = client.get_price_history(&asset, &0u32, &10u32);
    assert_eq!(history.len(), 10);

    for entry in history.iter() {
        assert_eq!(entry.price, stable_price);
    }
}

#[test]
fn test_delta_encoding_zero_delta() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 1500, 100);
    submit_test_price_n(&client, &source, &asset, 1500, 200, 2);

    let current = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(current.price, 1500);
}

#[test]
fn test_delta_encoding_negative_delta() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source, &asset, 2000, 100);
    submit_test_price_n(&client, &source, &asset, 1800, 200, 2);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 1800);
}

#[test]
fn test_delta_encoding_interpolation_compatibility() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    at(&e, 1, 100);
    submit_test_price(&client, &source, &asset, 1000, 100);
    at(&e, 2, 200);
    submit_test_price_n(&client, &source, &asset, 2000, 200, 2);

    let history = client.get_price_history(&asset, &0u32, &2u32);

    let first = history.get(0).unwrap();
    let second = history.get(1).unwrap();

    assert_eq!(first.price, 1000);
    assert_eq!(second.price, 2000);
}

#[test]
fn test_delta_encoding_storage_bounds() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    // Prices must be strictly positive (see `ErrorCode::InvalidPrice`), so we
    // exercise the largest and smallest valid magnitudes instead of the signed
    // extremes.
    let max_i128 = i128::MAX;
    let min_positive = 1i128;

    at(&e, 1, 100);
    submit_test_price(&client, &source, &asset, max_i128, 100);
    at(&e, 2, 200);
    submit_test_price_n(&client, &source, &asset, min_positive, 200, 2);

    let history = client.get_price_history(&asset, &0u32, &2u32);
    assert_eq!(history.len(), 2);
}
