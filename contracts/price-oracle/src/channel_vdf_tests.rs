#![cfg(test)]
//! #466 — State-channel and VDF sampler manipulation tests.

use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, BytesN, Env, Vec};

use crate::test_helpers::{
    deploy_token, ledger_default, mint_token, register_test_source, setup_contract,
};
use crate::types::BatchItem;
use crate::PriceOracleContractClient;

fn batch(e: &Env, items: &[(u64, u128)]) -> Vec<BatchItem> {
    let mut v = Vec::new(e);
    for (nonce, price) in items {
        v.push_back(BatchItem {
            nonce: *nonce,
            price: *price,
            timestamp: 1_000,
        });
    }
    v
}

/// Mirrors `state_channel::hash_batch_payload` and signs the digest.
fn sign(e: &Env, sk: &SigningKey, b: &Vec<BatchItem>) -> (BytesN<64>, BytesN<32>) {
    let mut buf = Bytes::from_slice(e, b"sc_batch_v1");
    for item in b.iter() {
        buf.append(&Bytes::from_slice(e, &item.nonce.to_le_bytes()));
        buf.append(&Bytes::from_slice(e, &item.price.to_le_bytes()));
        buf.append(&Bytes::from_slice(e, &item.timestamp.to_le_bytes()));
    }
    let digest: BytesN<32> = e.crypto().sha256(&buf).into();
    let sig = sk.sign(&digest.to_array());
    (
        BytesN::from_array(e, &sig.to_bytes()),
        BytesN::from_array(e, &sk.verifying_key().to_bytes()),
    )
}

fn open(e: &Env) -> (PriceOracleContractClient<'_>, Address, SigningKey) {
    ledger_default(e, 10, 1_000);
    let (client, _) = setup_contract(e);
    let source = Address::generate(e);
    let token = deploy_token(e);
    mint_token(e, &token, &source, 1_000);
    client.sc_open_channel(&source, &1_000i128, &token);
    let sk = SigningKey::from_bytes(&[7u8; 32]);
    let b = batch(e, &[(1, 100)]);
    let (sig, pk) = sign(e, &sk, &b);
    client.sc_submit_batch(&source, &b, &sig, &pk);
    (client, source, sk)
}

#[test]
fn superseded_state_cannot_settle() {
    let e = Env::default();
    let (client, source, sk) = open(&e);
    let b2 = batch(&e, &[(2, 200)]);
    let (sig2, pk) = sign(&e, &sk, &b2);
    client.sc_submit_batch(&source, &b2, &sig2, &pk);

    // Replaying the earlier signed state (nonce 1) is rejected on both paths.
    let old = batch(&e, &[(1, 100)]);
    let (sig1, _) = sign(&e, &sk, &old);
    assert!(client
        .try_sc_submit_batch(&source, &old, &sig1, &pk)
        .is_err());
    ledger_default(&e, 20, 1_000 + 3_600);
    assert!(client
        .try_sc_dispute_channel(&source, &old, &sig1, &pk)
        .is_err());
    assert_eq!(client.sc_get_channel(&source).unwrap().last_price, 200);
}

#[test]
fn unsigned_state_cannot_settle() {
    let e = Env::default();
    let (client, source, sk) = open(&e);
    let forged = batch(&e, &[(9, 1)]);
    let (_, pk) = sign(&e, &sk, &forged);
    let bad_sig = BytesN::from_array(&e, &[0u8; 64]);
    ledger_default(&e, 20, 1_000 + 3_600);
    assert!(client
        .try_sc_dispute_channel(&source, &forged, &bad_sig, &pk)
        .is_err());
}

#[test]
fn forced_close_with_attacker_key_is_rejected() {
    let e = Env::default();
    let (client, source, _sk) = open(&e);
    // Attacker signs a favourable state with their own key and supplies that key.
    let attacker = SigningKey::from_bytes(&[42u8; 32]);
    let forged = batch(&e, &[(99, 1)]);
    let (sig, pk) = sign(&e, &attacker, &forged);
    ledger_default(&e, 20, 1_000 + 3_600);
    assert!(client
        .try_sc_dispute_channel(&source, &forged, &sig, &pk)
        .is_err());
    assert!(client
        .try_sc_submit_batch(&source, &forged, &sig, &pk)
        .is_err());
    assert_eq!(client.sc_get_channel(&source).unwrap().nonce, 1);
}

#[test]
fn dispute_before_timeout_is_rejected() {
    let e = Env::default();
    let (client, source, sk) = open(&e);
    let b = batch(&e, &[(5, 500)]);
    let (sig, pk) = sign(&e, &sk, &b);
    assert!(client
        .try_sc_dispute_channel(&source, &b, &sig, &pk)
        .is_err());
}

#[test]
fn channel_state_never_reaches_main_chain_price() {
    let e = Env::default();
    let (client, source, sk) = open(&e);
    let b = batch(&e, &[(2, 123_456)]);
    let (sig, pk) = sign(&e, &sk, &b);
    let asset = crate::test_helpers::register_test_asset(&e, &client);
    client.sc_submit_batch(&source, &b, &sig, &pk);
    // Channel settlement is bookkeeping only; it never writes an aggregate price.
    assert!(client.get_price(&asset, &0u64).is_none());
}

// ── VDF sampler ─────────────────────────────────────────────────────────────

fn sampler(e: &Env, n: usize) -> PriceOracleContractClient<'_> {
    ledger_default(e, 10, 1_000);
    let (client, _) = setup_contract(e);
    for i in 0..n {
        register_test_source(e, &client, if i % 2 == 0 { "A" } else { "B" });
    }
    client.vdf_set_sampling_size(&2u32);
    client
}

#[test]
fn seed_is_caller_independent_and_ledger_bound() {
    let e = Env::default();
    let client = sampler(&e, 4);
    let s1 = client.vdf_get_current_seed();
    assert_eq!(s1, client.vdf_get_current_seed());
    ledger_default(&e, 11, 1_005);
    assert_ne!(s1, client.vdf_get_current_seed());
}

#[test]
fn caller_cannot_bias_selection_with_forged_output() {
    let e = Env::default();
    let client = sampler(&e, 6);
    let all = client.vdf_sample_sources(&Bytes::new(&e), &BytesN::from_array(&e, &[0; 32]), &1u64);
    assert_eq!(all.len(), 6);
    // Grinding chosen outputs/proofs never yields a caller-chosen subset: every
    // unverified attempt falls back to the full source set.
    for i in 0..32u8 {
        let out = BytesN::from_array(&e, &[i; 32]);
        let proof = Bytes::from_slice(&e, &[i, i.wrapping_mul(3), 1]);
        assert_eq!(client.vdf_sample_sources(&proof, &out, &1_000u64), all);
    }
}

#[test]
fn vdf_check_cannot_be_bypassed() {
    let e = Env::default();
    let client = sampler(&e, 4);
    let seed = client.vdf_get_current_seed();
    let out = BytesN::from_array(&e, &[1; 32]);
    let proof = Bytes::from_slice(&e, &[1, 2, 3]);
    // Every short-circuit input is refused.
    assert!(!client.vdf_verify_proof(&seed, &Bytes::new(&e), &0u64, &out));
    assert!(!client.vdf_verify_proof(&seed, &proof, &0u64, &out));
    assert!(!client.vdf_verify_proof(&seed, &proof, &1_000_001u64, &out));
    assert!(!client.vdf_verify_proof(&seed, &proof, &10u64, &out));

    // With the check removed, a caller grinding `output` could pick the subset;
    // show that distinct outputs do select distinct subsets, i.e. the check is
    // the only thing preventing a chosen outcome.
    let pick = |b: u8| {
        e.as_contract(&client.address, || {
            let src = crate::storage::read_oracle_sources(&e).sources;
            crate::vdf_sampler::select_for_test(&e, &src, 2, BytesN::from_array(&e, &[b; 32]))
        })
    };
    let first = pick(0);
    assert!((1..=64u8).any(|b| pick(b) != first));
}
