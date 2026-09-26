//! # #450 — Cross-chain replay and signature malleability audit
//!
//! Adversarial tests backing the threat matrix in
//! `docs/security/cross-chain-replay-audit.md`. Every adapter that credits a
//! price is attacked for replay (same chain, cross chain, cross adapter),
//! emitter / guardian forgery, chain-identity spoofing and signature
//! malleability.

use ed25519_dalek::{Signer, SigningKey};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, BytesN, Env, String, Vec,
};

use crate::bridge_common::encode_price_payload;
use crate::test_helpers::*;
use crate::types::{CrossChainPricePayload, ErrorCode, WormholeVaa};
use crate::PriceOracleContractClient;

const ETH_EID: u32 = 30101;
const BSC_EID: u32 = 30102;

struct Bridges<'a> {
    client: PriceOracleContractClient<'a>,
    asset: Address,
    gateway: Address,
    endpoint: Address,
    axelar_source: Address,
    lz_source: Address,
    sender: BytesN<32>,
    foreign: BytesN<32>,
}

fn eth(e: &Env) -> String {
    String::from_str(e, "ethereum")
}

fn axelar_addr(e: &Env) -> String {
    String::from_str(e, "0xSourceContract")
}

/// Wires Axelar and LayerZero for the same asset on "ethereum" with one
/// dedicated bridge source each.
fn setup(e: &Env) -> Bridges<'_> {
    e.mock_all_auths();
    e.cost_estimate().disable_resource_limits();
    let (client, _admin) = setup_contract(e);
    client.set_min_sources_required(&1u32);
    e.ledger().with_mut(|l| l.timestamp = 1_000_000);

    let asset = register_test_asset(e, &client);
    let axelar_source = register_test_source(e, &client, "Axelar");
    let lz_source = register_test_source(e, &client, "LayerZero");
    client.add_source_asset(&axelar_source, &asset);
    client.add_source_asset(&lz_source, &asset);

    let foreign = BytesN::from_array(e, &[0x11; 32]);
    client.register_foreign_asset_mapping(&asset, &eth(e), &foreign, &18u32);

    let gateway = Address::generate(e);
    client.set_axelar_gateway(&gateway);
    client.set_axelar_trusted_source(&eth(e), &axelar_addr(e), &axelar_source);

    let endpoint = Address::generate(e);
    let sender = BytesN::from_array(e, &[0x22; 32]);
    client.set_layerzero_endpoint(&endpoint);
    client.set_lz_chain_name(&ETH_EID, &eth(e));
    client.set_lz_trusted_remote(&ETH_EID, &sender, &lz_source);

    Bridges {
        client,
        asset,
        gateway,
        endpoint,
        axelar_source,
        lz_source,
        sender,
        foreign,
    }
}

fn payload(e: &Env, foreign: &BytesN<32>, price: i128, nonce: u64) -> Bytes {
    encode_price_payload(
        e,
        &CrossChainPricePayload {
            foreign_asset: foreign.clone(),
            price,
            decimals: 18,
            timestamp: e.ledger().timestamp(),
            nonce,
        },
    )
}

fn cmd(e: &Env, b: u8) -> BytesN<32> {
    BytesN::from_array(e, &[b; 32])
}

// ---------------------------------------------------------------------------
// Axelar
// ---------------------------------------------------------------------------

#[test]
fn axelar_same_command_replay_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    b.client
        .execute_axelar_message(&b.gateway, &cmd(&e, 1), &eth(&e), &axelar_addr(&e), &p);
    let r = b.client.try_execute_axelar_message(
        &b.gateway,
        &cmd(&e, 1),
        &eth(&e),
        &axelar_addr(&e),
        &p,
    );
    assert_eq!(r, Err(Ok(code(ErrorCode::AxelarCommandAlreadyExecuted))));
}

#[test]
fn axelar_replay_on_other_chain_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    // A fresh command id claiming the same message came from another chain:
    // the (chain, address) pair is not trusted there.
    let r = b.client.try_execute_axelar_message(
        &b.gateway,
        &cmd(&e, 2),
        &String::from_str(&e, "bsc"),
        &axelar_addr(&e),
        &p,
    );
    assert_eq!(r, Err(Ok(code(ErrorCode::AxelarSourceNotTrusted))));
}

#[test]
fn axelar_forged_emitter_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    let r = b.client.try_execute_axelar_message(
        &b.gateway,
        &cmd(&e, 3),
        &eth(&e),
        &String::from_str(&e, "0xAttacker"),
        &p,
    );
    assert_eq!(r, Err(Ok(code(ErrorCode::AxelarSourceNotTrusted))));
}

#[test]
fn axelar_forged_gateway_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    let fake_gateway = Address::generate(&e);
    let r = b.client.try_execute_axelar_message(
        &fake_gateway,
        &cmd(&e, 4),
        &eth(&e),
        &axelar_addr(&e),
        &p,
    );
    assert_eq!(r, Err(Ok(code(ErrorCode::NotAuthorized))));
}

#[test]
fn axelar_chain_identity_bound_to_asset_mapping() {
    let e = Env::default();
    let b = setup(&e);
    // Trust the same emitter on a cheap chain without an asset mapping there:
    // the "ethereum" foreign id must not resolve on "bsc".
    let bsc = String::from_str(&e, "bsc");
    b.client
        .set_axelar_trusted_source(&bsc, &axelar_addr(&e), &b.axelar_source);
    let p = payload(&e, &b.foreign, 100, 1);
    let r =
        b.client
            .try_execute_axelar_message(&b.gateway, &cmd(&e, 5), &bsc, &axelar_addr(&e), &p);
    assert_eq!(r, Err(Ok(code(ErrorCode::ForeignAssetNotMapped))));
}

// ---------------------------------------------------------------------------
// LayerZero
// ---------------------------------------------------------------------------

#[test]
fn lz_same_nonce_replay_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    b.client
        .lz_receive(&b.endpoint, &ETH_EID, &b.sender, &1u64, &cmd(&e, 1), &p);
    let r = b
        .client
        .try_lz_receive(&b.endpoint, &ETH_EID, &b.sender, &1u64, &cmd(&e, 1), &p);
    assert_eq!(r, Err(Ok(code(ErrorCode::LzNonceOutOfOrder))));
}

#[test]
fn lz_nonce_skip_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    let r = b
        .client
        .try_lz_receive(&b.endpoint, &ETH_EID, &b.sender, &2u64, &cmd(&e, 1), &p);
    assert_eq!(r, Err(Ok(code(ErrorCode::LzNonceOutOfOrder))));
}

#[test]
fn lz_replay_on_other_eid_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    let r = b
        .client
        .try_lz_receive(&b.endpoint, &BSC_EID, &b.sender, &1u64, &cmd(&e, 1), &p);
    assert_eq!(r, Err(Ok(code(ErrorCode::LzRemoteNotTrusted))));
}

#[test]
fn lz_forged_sender_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    let attacker = BytesN::from_array(&e, &[0x66; 32]);
    let r = b
        .client
        .try_lz_receive(&b.endpoint, &ETH_EID, &attacker, &1u64, &cmd(&e, 1), &p);
    assert_eq!(r, Err(Ok(code(ErrorCode::LzRemoteNotTrusted))));
}

#[test]
fn lz_forged_endpoint_fails() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    let fake = Address::generate(&e);
    let r = b
        .client
        .try_lz_receive(&fake, &ETH_EID, &b.sender, &1u64, &cmd(&e, 1), &p);
    assert_eq!(r, Err(Ok(code(ErrorCode::NotAuthorized))));
}

#[test]
fn lz_eid_without_chain_name_fails() {
    let e = Env::default();
    let b = setup(&e);
    // Trusted remote on an eid whose chain identity was never bound.
    b.client
        .set_lz_trusted_remote(&BSC_EID, &b.sender, &b.lz_source);
    let p = payload(&e, &b.foreign, 100, 1);
    let r = b
        .client
        .try_lz_receive(&b.endpoint, &BSC_EID, &b.sender, &1u64, &cmd(&e, 1), &p);
    assert_eq!(r, Err(Ok(code(ErrorCode::LzChainNameNotConfigured))));
}

// ---------------------------------------------------------------------------
// Cross-adapter
// ---------------------------------------------------------------------------

/// Replaying the same effect through the same adapter overwrites the bridge
/// source's single submission slot instead of adding a second contribution.
#[test]
fn same_adapter_redelivery_occupies_one_slot() {
    let e = Env::default();
    let b = setup(&e);
    let p = payload(&e, &b.foreign, 100, 1);
    b.client
        .execute_axelar_message(&b.gateway, &cmd(&e, 1), &eth(&e), &axelar_addr(&e), &p);
    b.client
        .execute_axelar_message(&b.gateway, &cmd(&e, 2), &eth(&e), &axelar_addr(&e), &p);
    let agg = b.client.get_price(&b.asset, &u64::MAX).unwrap();
    assert_eq!(agg.num_sources, 1);
}

/// When both adapters for one upstream feed are attributed to the same bridge
/// source (the documented configuration), the payload cannot be credited twice.
#[test]
fn cross_adapter_same_feed_is_not_double_counted() {
    let e = Env::default();
    let b = setup(&e);
    b.client
        .set_lz_trusted_remote(&ETH_EID, &b.sender, &b.axelar_source);
    let p = payload(&e, &b.foreign, 100, 1);
    b.client
        .execute_axelar_message(&b.gateway, &cmd(&e, 1), &eth(&e), &axelar_addr(&e), &p);
    b.client
        .lz_receive(&b.endpoint, &ETH_EID, &b.sender, &1u64, &cmd(&e, 9), &p);
    let agg = b.client.get_price(&b.asset, &u64::MAX).unwrap();
    assert_eq!(agg.num_sources, 1);
}

// ---------------------------------------------------------------------------
// Wormhole (module-level: the relay is not exposed as a contract endpoint)
// ---------------------------------------------------------------------------

/// Ed25519 group order L, little-endian.
const ED25519_L: [u8; 32] = [
    0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7, 0xa2, 0xde, 0xf9, 0xde, 0x14,
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0x10,
];

fn body_hash(e: &Env, vaa: &WormholeVaa) -> [u8; 32] {
    let mut body = Bytes::new(e);
    body.append(&Bytes::from_slice(e, &vaa.emitter_chain.to_be_bytes()));
    body.append(&vaa.emitter_address.clone().into());
    body.append(&Bytes::from_slice(e, &vaa.sequence.to_be_bytes()));
    body.append(&vaa.payload);
    let first: Bytes = e.crypto().sha256(&body).into();
    e.crypto().sha256(&first).to_array()
}

fn signed_vaa(e: &Env, signers: &[(&SigningKey, u32)], sequence: u64) -> WormholeVaa {
    let mut vaa = WormholeVaa {
        emitter_chain: 2,
        emitter_address: BytesN::from_array(e, &[0x42; 32]),
        sequence,
        payload: crate::wormhole_relay::encode_price_payload(e, 100, 18, 1_000),
        signatures: Vec::new(e),
        guardian_indices: Vec::new(e),
    };
    let h = body_hash(e, &vaa);
    for (sk, idx) in signers {
        vaa.signatures
            .push_back(BytesN::from_array(e, &sk.sign(&h).to_bytes()));
        vaa.guardian_indices.push_back(*idx);
    }
    vaa
}

fn with_guardians(e: &Env, quorum: u32, f: impl FnOnce(&SigningKey, &SigningKey)) {
    e.mock_all_auths();
    let (client, _admin) = setup_contract(e);
    let g1 = SigningKey::from_bytes(&[1; 32]);
    let g2 = SigningKey::from_bytes(&[2; 32]);
    e.as_contract(&client.address, || {
        let mut set: Vec<BytesN<32>> = Vec::new(e);
        set.push_back(BytesN::from_array(e, &g1.verifying_key().to_bytes()));
        set.push_back(BytesN::from_array(e, &g2.verifying_key().to_bytes()));
        crate::wormhole_relay::set_guardian_set(e, set, quorum);
        f(&g1, &g2);
    });
}

#[test]
fn wormhole_duplicate_guardian_does_not_reach_quorum() {
    let e = Env::default();
    with_guardians(&e, 2, |g1, _| {
        let vaa = signed_vaa(&e, &[(g1, 0), (g1, 0)], 1);
        assert!(!crate::wormhole_relay::verify_vaa_quorum(&e, &vaa));
    });
}

#[test]
#[should_panic]
fn wormhole_forged_guardian_signature_fails() {
    let e = Env::default();
    with_guardians(&e, 1, |_, _| {
        let attacker = SigningKey::from_bytes(&[9; 32]);
        let vaa = signed_vaa(&e, &[(&attacker, 0)], 1);
        crate::wormhole_relay::verify_vaa_quorum(&e, &vaa);
    });
}

#[test]
#[should_panic(expected = "Error(Contract, #146)")]
fn wormhole_out_of_range_guardian_index_fails() {
    let e = Env::default();
    with_guardians(&e, 1, |g1, _| {
        let vaa = signed_vaa(&e, &[(g1, 7)], 1);
        crate::wormhole_relay::verify_vaa_quorum(&e, &vaa);
    });
}

#[test]
#[should_panic]
fn wormhole_resigned_body_for_other_chain_fails() {
    let e = Env::default();
    with_guardians(&e, 1, |g1, _| {
        // Signature covers emitter_chain 2; retargeting to chain 4 breaks it.
        let mut vaa = signed_vaa(&e, &[(g1, 0)], 1);
        vaa.emitter_chain = 4;
        crate::wormhole_relay::verify_vaa_quorum(&e, &vaa);
    });
}

/// Malleability: `S + L` is a second encoding of the same Ed25519 signature.
/// The host verifier rejects non-canonical `S`, so the variant is unusable.
#[test]
#[should_panic]
fn wormhole_malleated_signature_rejected() {
    let e = Env::default();
    with_guardians(&e, 1, |g1, _| {
        let mut vaa = signed_vaa(&e, &[(g1, 0)], 1);
        let mut sig = vaa.signatures.get(0).unwrap().to_array();
        let mut carry = 0u16;
        for i in 0..32 {
            let v = sig[32 + i] as u16 + ED25519_L[i] as u16 + carry;
            sig[32 + i] = v as u8;
            carry = v >> 8;
        }
        vaa.signatures.set(0, BytesN::from_array(&e, &sig));
        crate::wormhole_relay::verify_vaa_quorum(&e, &vaa);
    });
}

#[test]
fn wormhole_valid_quorum_passes() {
    let e = Env::default();
    with_guardians(&e, 2, |g1, g2| {
        let vaa = signed_vaa(&e, &[(g1, 0), (g2, 1)], 1);
        assert!(crate::wormhole_relay::verify_vaa_quorum(&e, &vaa));
    });
}

fn code(c: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(c as u32)
}
