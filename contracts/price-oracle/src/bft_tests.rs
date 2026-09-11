#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
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
        max_entry_ttl: 6000,
    });
}

fn make_hash(e: &Env, price: i128, salt_val: u64, round_ledger: u32) -> BytesN<32> {
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
        let hash = make_hash(&e, price, salt, round_ledger);
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
