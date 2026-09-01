#![cfg(test)]

use soroban_sdk::Env;

use crate::test_helpers::{ledger_default, register_test_asset, setup_contract};

#[test]
fn test_optimistic_proposal_finalizes_after_dispute_window() {
    let env = Env::default();
    ledger_default(&env, 10, 1_000);

    let (client, _admin) = setup_contract(&env);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&env, &client);

    let proposal_id = client.propose_price(&asset, &123i128, &1_000u64, &10i128);
    assert_eq!(proposal_id, 1u32);

    let pending = client.get_proposal(&proposal_id).unwrap();
    assert_eq!(pending.price, 123);
    assert!(!pending.disputed);
    assert_eq!(pending.status, 0u32);

    ledger_default(&env, 130, 1_100);
    let finalized = client.get_proposal(&proposal_id).unwrap();
    assert_eq!(finalized.status, 1u32);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 123);
    assert_eq!(price.timestamp, 1_000);
}

#[test]
fn test_dispute_and_resolve_proposal() {
    let env = Env::default();
    ledger_default(&env, 10, 1_000);

    let (client, _admin) = setup_contract(&env);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&env, &client);

    let proposal_id = client.propose_price(&asset, &500i128, &1_000u64, &10i128);
    client.dispute_proposal(&proposal_id);

    let disputed = client.get_proposal(&proposal_id).unwrap();
    assert!(disputed.disputed);
    assert_eq!(disputed.status, 2u32);

    client.resolve_dispute(&proposal_id, &false);
    let resolved = client.get_proposal(&proposal_id).unwrap();
    assert_eq!(resolved.status, 3u32);
}

#[test]
fn test_resolve_via_external_data() {
    let env = Env::default();
    ledger_default(&env, 10, 1_000);

    let (client, admin) = setup_contract(&env);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&env, &client);

    let proposal_id = client.propose_price(&asset, &123i128, &1_000u64, &10i128);
    client.dispute_proposal(&proposal_id);

    let source = soroban_sdk::Address::generate(&env);
    let external_price = 456i128;
    let timestamp = 1_000u64;
    let mut preimage = soroban_sdk::Bytes::new(&env);
    preimage.append(&source.to_xdr(&env));
    let price_bytes = external_price.to_le_bytes();
    for b in price_bytes.iter() {
        preimage.push_back(b);
    }
    let ts_bytes = timestamp.to_le_bytes();
    for b in ts_bytes.iter() {
        preimage.push_back(b);
    }
    let data_hash: soroban_sdk::BytesN<32> = env.crypto().sha256(&preimage).into();
    let signature = soroban_sdk::BytesN::<64>::from_slice(&env, &[0u8; 64]);

    let proof = crate::ExternalDataProof {
        source: source.clone(),
        data_hash,
        timestamp,
        signature,
    };

    client.resolve_via_external_data(&proposal_id, &external_price, &proof);

    let resolved = client.get_proposal(&proposal_id).unwrap();
    assert_eq!(resolved.status, 3u32);

    let price = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(price.price, 456);
}

#[test]
fn test_get_active_proposals() {
    let env = Env::default();
    ledger_default(&env, 10, 1_000);

    let (client, _admin) = setup_contract(&env);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&env, &client);

    let _p1 = client.propose_price(&asset, &100i128, &1_000u64, &10i128);
    let _p2 = client.propose_price(&asset, &200i128, &1_000u64, &10i128);
    client.dispute_proposal(&2u32);

    let active = client.get_active_proposals();
    assert_eq!(active.len(), 2);
}
