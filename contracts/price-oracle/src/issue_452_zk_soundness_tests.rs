//! Issue #452 — ZK proof verifier soundness and malleability audit.
//!
//! # Statement and verification equation
//!
//! `zk_submit_price(source, asset, proof, signals)` claims that `signals =
//! [asset_hash, price, timestamp]` satisfy the circuit behind the stored
//! verifying key. Groth16 would check `e(A,B) = e(α,β)·e(vk_x,γ)·e(C,δ)` with
//! `vk_x = IC₀ + Σ sᵢ·ICᵢ₊₁`.
//!
//! What `zk_verify::groth16_verify` actually checks:
//!
//! 1. `|A| = 64`, `|B| = 128`, `|C| = 64` bytes.
//! 2. `|signals| = ic_len - 1`.
//! 3. `fs_check = sha256(sha256(A‖B‖C‖vk_x‖signals) ‖ pairing_precomp)`.
//!
//! No pairing is evaluated. Every input to the tag is public (the verifying key
//! is readable via `zk_get_verification_key`), so the tag is a checksum, not a
//! proof.
//!
//! # Soundness assumptions and what the verifier does *not* prove
//!
//! The verifier proves only that the submitter knew the public verifying key.
//! It does **not** prove knowledge of a witness, that the price is correct,
//! that `asset_hash` matches `asset`, or that the proof is fresh. Security rests
//! entirely on `source.require_auth()` and source registration.
//!
//! # Attack classes
//!
//! | # | Attack | Result | Test |
//! |---|---|---|---|
//! | 1 | Proof for a false statement | **accepted** — tag is forgeable from public data | `gap_forged_proof_for_arbitrary_statement_accepted` |
//! | 2 | Replay under another asset/domain | **accepted** — `signals[0]` is never compared to `asset`; no contract/network id in the transcript | `gap_proof_replays_across_assets` |
//! | 3 | Encoding malleability | **accepted** — signals ≥ r and non-zero high bytes are not rejected; only the low bytes are decoded | `gap_non_canonical_price_signal_accepted` |
//! | 4 | Missing Fiat-Shamir binding | fails closed for A, B, C and every signal given a fixed tag | `tampering_*` |
//! | 5 | Identity / subgroup edge cases | fails closed on size and count; points are never curve/subgroup checked | `malformed_*`, `gap_vk_without_ic_points_accepted` |
//!
//! # Divergence from Groth16 and consequence
//!
//! The implementation diverges completely: no pairing check, no point
//! validation, `ic_len` not reconciled with `ic_bytes` (missing IC points are
//! silently treated as the identity). Consequence: any registered source can
//! submit any price, making the ZK path equivalent to an ordinary signed
//! submission. Remediation (out of scope: "implementing a new proving system")
//! is a real BN254 pairing check via the host `crypto().bn254()` primitives,
//! point validation, and binding `asset`, contract id and network id into the
//! public inputs.

use soroban_sdk::{vec, Address, Bytes, BytesN, Env, Vec};

use crate::test_helpers::{ledger_default, register_test_asset, setup_basic};
use crate::types::{AggregatePrice, DataKey, Groth16Proof, Groth16VerifyingKey};
use crate::PriceOracleContractClient;

const TS: u64 = 1_000_000;

fn u64_signal(e: &Env, v: u64) -> BytesN<32> {
    let mut b = [0u8; 32];
    b[24..].copy_from_slice(&v.to_be_bytes());
    BytesN::from_array(e, &b)
}

fn precomp(e: &Env) -> Bytes {
    Bytes::from_slice(e, &[7u8; 32])
}

fn setup(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address) {
    e.mock_all_auths();
    ledger_default(e, 100, TS);
    let (client, _admin, source, asset) = setup_basic(e);
    client.zk_set_verification_key(&Groth16VerifyingKey {
        ic_len: 4,
        ic_bytes: Bytes::new(e),
        pairing_precomp: precomp(e),
    });
    (client, source, asset)
}

fn signals(e: &Env, price: u64) -> Vec<BytesN<32>> {
    vec![
        e,
        u64_signal(e, 42),
        u64_signal(e, price),
        u64_signal(e, TS),
    ]
}

/// Computes the tag the verifier expects using only public data.
fn forge(e: &Env, a: Bytes, b: Bytes, c: Bytes, sigs: &Vec<BytesN<32>>) -> Groth16Proof {
    let mut t = Bytes::new(e);
    t.append(&a);
    t.append(&b);
    t.append(&c);
    for s in sigs.iter() {
        t.append(&Bytes::from_slice(e, &s.to_array()));
    }
    let mut tag = Bytes::from_slice(e, &e.crypto().sha256(&t).to_array());
    tag.append(&precomp(e));
    let fs_check = BytesN::from_array(e, &e.crypto().sha256(&tag).to_array());
    Groth16Proof { a, b, c, fs_check }
}

fn proof(e: &Env, sigs: &Vec<BytesN<32>>) -> Groth16Proof {
    forge(
        e,
        Bytes::from_slice(e, &[1u8; 64]),
        Bytes::from_slice(e, &[2u8; 128]),
        Bytes::from_slice(e, &[3u8; 64]),
        sigs,
    )
}

fn aggregate(e: &Env, client: &PriceOracleContractClient<'_>, asset: &Address) -> Option<i128> {
    e.as_contract(&client.address, || {
        e.storage()
            .persistent()
            .get::<_, AggregatePrice>(&DataKey::Aggregate(asset.clone()))
            .map(|a| a.price)
    })
}

fn assert_rejected(
    client: &PriceOracleContractClient<'_>,
    source: &Address,
    asset: &Address,
    p: &Groth16Proof,
    sigs: &Vec<BytesN<32>>,
) {
    assert!(client.try_zk_submit_price(source, asset, p, sigs).is_err());
}

#[test]
fn gap_forged_proof_for_arbitrary_statement_accepted() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    let sigs = signals(&e, 123_456);
    client.zk_submit_price(&source, &asset, &proof(&e, &sigs), &sigs);
    assert_eq!(aggregate(&e, &client, &asset), Some(123_456));
}

#[test]
fn gap_proof_replays_across_assets() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    let other = register_test_asset(&e, &client);
    let sigs = signals(&e, 777);
    let p = proof(&e, &sigs);
    client.zk_submit_price(&source, &asset, &p, &sigs);
    client.zk_submit_price(&source, &other, &p, &sigs);
    assert_eq!(aggregate(&e, &client, &other), Some(777));
}

#[test]
fn gap_non_canonical_price_signal_accepted() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    // High 16 bytes set: value >= r, yet decodes to the same low-128-bit price.
    let mut raw = [0xFFu8; 32];
    raw[16..].copy_from_slice(&[0u8; 16]);
    raw[24..].copy_from_slice(&999u64.to_be_bytes());
    let sigs = vec![
        &e,
        u64_signal(&e, 42),
        BytesN::from_array(&e, &raw),
        u64_signal(&e, TS),
    ];
    client.zk_submit_price(&source, &asset, &proof(&e, &sigs), &sigs);
    assert_eq!(aggregate(&e, &client, &asset), Some(999));
}

#[test]
fn gap_vk_without_ic_points_accepted() {
    let e = Env::default();
    let (client, _source, _asset) = setup(&e);
    // ic_len claims 4 points but ic_bytes is empty; the key is stored as-is.
    assert_eq!(client.zk_get_verification_key().unwrap().ic_bytes.len(), 0);
}

#[test]
fn tampering_any_public_input_is_rejected() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    let sigs = signals(&e, 500);
    let p = proof(&e, &sigs);
    for i in 0..sigs.len() {
        let mut changed = sigs.clone();
        let mut raw = changed.get_unchecked(i).to_array();
        raw[31] ^= 1;
        changed.set(i, BytesN::from_array(&e, &raw));
        assert_rejected(&client, &source, &asset, &p, &changed);
    }
    assert_eq!(aggregate(&e, &client, &asset), None);
}

#[test]
fn tampering_proof_elements_is_rejected() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    let sigs = signals(&e, 500);
    let p = proof(&e, &sigs);

    let mut t = p.clone();
    t.a.set(0, 9);
    assert_rejected(&client, &source, &asset, &t, &sigs);
    let mut t = p.clone();
    t.b.set(127, 9);
    assert_rejected(&client, &source, &asset, &t, &sigs);
    let mut t = p.clone();
    t.c.set(10, 9);
    assert_rejected(&client, &source, &asset, &t, &sigs);
    let mut t = p;
    t.fs_check = BytesN::from_array(&e, &[0u8; 32]);
    assert_rejected(&client, &source, &asset, &t, &sigs);
}

#[test]
fn malformed_point_sizes_are_rejected() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    let sigs = signals(&e, 500);
    for (a, b, c) in [
        (63u32, 128u32, 64u32),
        (64, 129, 64),
        (64, 128, 0),
        (0, 0, 0),
    ] {
        let p = forge(
            &e,
            Bytes::from_slice(&e, &[1u8; 64][..a as usize]),
            Bytes::from_slice(&e, &[2u8; 129][..b as usize]),
            Bytes::from_slice(&e, &[3u8; 64][..c as usize]),
            &sigs,
        );
        assert_rejected(&client, &source, &asset, &p, &sigs);
    }
}

#[test]
fn malformed_signal_count_is_rejected() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    let mut sigs = signals(&e, 500);
    sigs.push_back(u64_signal(&e, 1));
    assert_rejected(&client, &source, &asset, &proof(&e, &sigs), &sigs);
    let short = vec![&e, u64_signal(&e, 42), u64_signal(&e, 500)];
    assert_rejected(&client, &source, &asset, &proof(&e, &short), &short);
}

#[test]
fn zero_and_negative_prices_are_rejected() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    let zero = signals(&e, 0);
    assert_rejected(&client, &source, &asset, &proof(&e, &zero), &zero);

    let mut raw = [0u8; 32];
    raw[16] = 0x80; // sign bit of the decoded i128
    raw[31] = 1;
    let neg = vec![
        &e,
        u64_signal(&e, 42),
        BytesN::from_array(&e, &raw),
        u64_signal(&e, TS),
    ];
    assert_rejected(&client, &source, &asset, &proof(&e, &neg), &neg);
}

#[test]
#[should_panic(expected = "Error(Contract, #97)")]
fn missing_verifying_key_is_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, TS);
    let (client, _admin, source, asset) = setup_basic(&e);
    let sigs = signals(&e, 500);
    client.zk_submit_price(&source, &asset, &proof(&e, &sigs), &sigs);
}
