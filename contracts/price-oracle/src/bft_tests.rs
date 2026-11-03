#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    xdr::ToXdr,
    Address, Bytes, BytesN, Env, Vec,
};

use crate::test_helpers::*;

fn advance_ledger(e: &Env, seq: u32) {
    e.ledger().set(LedgerInfo {
        timestamp: (seq as u64) * 5,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
}

fn make_hash(
    e: &Env,
    source: &Address,
    price: i128,
    salt_val: u64,
    round_ledger: u32,
) -> BytesN<32> {
    let price_bytes = price.to_le_bytes();
    let salt_bytes = salt_val.to_le_bytes();
    let round_bytes = round_ledger.to_le_bytes();

    let mut preimage = Bytes::new(e);
    for b in price_bytes.iter() {
        preimage.push_back(*b);
    }
    for b in salt_bytes.iter() {
        preimage.push_back(*b);
    }
    for b in round_bytes.iter() {
        preimage.push_back(*b);
    }
    preimage.append(&source.clone().to_xdr(e));
    e.crypto().sha256(&preimage).into()
}

fn salt_bytes(e: &Env, val: u64) -> Bytes {
    let mut bytes = Bytes::new(e);
    for byte in val.to_le_bytes().iter() {
        bytes.push_back(*byte);
    }
    bytes
}

#[test]
fn test_bft_aggregation_filters_outlier_after_commit_reveal() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&4u32);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);
    client.set_bft_parameters(&1u32, &0u32);

    let asset = register_test_asset(&e, &client);
    let sources: Vec<Address> = Vec::from_array(
        &e,
        [
            register_test_source(&e, &client, "source-1"),
            register_test_source(&e, &client, "source-2"),
            register_test_source(&e, &client, "source-3"),
            register_test_source(&e, &client, "source-4"),
        ],
    );

    let round_ledger = client.current_round_ledger();
    let prices = [100i128, 101, 102, 1000];
    let salts = [11u64, 12, 13, 14];

    for i in 0..sources.len() {
        let source = sources.get_unchecked(i);
        let price = prices[i as usize];
        let salt = salts[i as usize];
        let hash = make_hash(&e, &source, price, salt, round_ledger);
        client.commit_price(&source, &asset, &hash);
    }

    advance_ledger(&e, 20);

    for i in 0..sources.len() {
        let source = sources.get_unchecked(i);
        let price = prices[i as usize];
        let salt = salts[i as usize];
        client.reveal_price(
            &source,
            &asset,
            &price,
            &salt_bytes(&e, salt),
            &round_ledger,
        );
    }

    let aggregate = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(aggregate.price, 101i128);
    assert_eq!(aggregate.num_sources, 4u32);
}

/// Runs one commit-reveal round with BFT enabled and returns the aggregate.
fn bft_round(method: u32, prices: &[i128]) -> i128 {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&(prices.len() as u32));
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);
    client.set_bft_parameters(&1u32, &method);
    assert_eq!(client.get_bft_fault_tolerance(), 1u32);
    assert_eq!(client.get_bft_aggregation_method(), method);

    let asset = register_test_asset(&e, &client);
    let round_ledger = client.current_round_ledger();
    let mut sources = std::vec::Vec::new();
    for (i, price) in prices.iter().enumerate() {
        let name = std::format!("source-{i}");
        let source = register_test_source(&e, &client, &name);
        let hash = make_hash(&e, &source, *price, i as u64, round_ledger);
        client.commit_price(&source, &asset, &hash);
        sources.push(source);
    }
    advance_ledger(&e, 20);
    for (i, source) in sources.iter().enumerate() {
        client.reveal_price(
            source,
            &asset,
            &prices[i],
            &salt_bytes(&e, i as u64),
            &round_ledger,
        );
    }
    client.get_price(&asset, &0u64).unwrap().price
}

/// No outlier is filtered; trimming one per side leaves [101, 102, 103, 110,
/// 111]: median 103, mean 105.
const SKEWED: [i128; 7] = [100, 101, 102, 103, 110, 111, 112];

#[test]
fn test_bft_filters_outlier_before_median() {
    // BFT drops 1000 and trims to [101]; a plain median of all four is 103.
    assert_eq!(bft_round(0, &[100, 101, 105, 1000]), 101);
}

#[test]
fn test_bft_median_method_uses_median() {
    assert_eq!(bft_round(0, &SKEWED), 103);
}

#[test]
fn test_bft_mean_method_uses_mean() {
    assert_eq!(bft_round(1, &SKEWED), 105);
}

#[test]
fn test_bft_defaults_are_off_and_median() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    assert_eq!(client.get_bft_fault_tolerance(), 0u32);
    assert_eq!(client.get_bft_aggregation_method(), 0u32);
}

#[test]
fn test_bft_rejects_direct_submission() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    client.set_bft_parameters(&1u32, &0u32);
    let res = client.try_submit_price(&source, &asset, &100i128, &0u64);
    assert_eq!(
        res,
        Err(Ok(crate::errors::ErrorCode::CommitRevealRequired.into()))
    );
}
