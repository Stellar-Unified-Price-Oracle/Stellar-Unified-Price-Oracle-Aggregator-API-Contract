//! # #451 — Forgery resistance for external payload decoders
//!
//! Adversarial tests for every decoder that accepts externally supplied
//! bytes. Grammars and findings are documented in
//! `docs/security/decoder-forgery-audit.md`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Bytes, BytesN, Env, String,
};

use crate::bridge_common::{decode_price_payload, encode_price_payload, PRICE_PAYLOAD_LEN};
use crate::test_helpers::*;
use crate::types::{BatchOperation, CrossChainPricePayload, ErrorCode};

fn canonical(e: &Env) -> Bytes {
    encode_price_payload(
        e,
        &CrossChainPricePayload {
            foreign_asset: BytesN::from_array(e, &[0x11; 32]),
            price: 100,
            decimals: 18,
            timestamp: 1_000_000,
            nonce: 1,
        },
    )
}

// ---------------------------------------------------------------------------
// bridge_common::decode_price_payload (Axelar / LayerZero wire format)
// ---------------------------------------------------------------------------

#[test]
fn bridge_payload_every_truncation_rejected() {
    let e = Env::default();
    let full = canonical(&e);
    for len in 0..PRICE_PAYLOAD_LEN {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            decode_price_payload(&e, &full.slice(0..len));
        }));
        assert!(r.is_err(), "truncated length {len} accepted");
    }
}

#[test]
#[should_panic(expected = "Error(Contract, #112)")]
fn bridge_payload_trailing_byte_rejected() {
    let e = Env::default();
    let mut p = canonical(&e);
    p.push_back(0);
    decode_price_payload(&e, &p);
}

#[test]
#[should_panic(expected = "Error(Contract, #112)")]
fn bridge_payload_duplicated_field_rejected() {
    let e = Env::default();
    // Appending a second nonce field (first-wins vs last-wins ambiguity) is
    // impossible to express: the grammar is fixed-width and exact-length.
    let mut p = canonical(&e);
    p.append(&Bytes::from_slice(&e, &2u64.to_le_bytes()));
    decode_price_payload(&e, &p);
}

#[test]
fn bridge_payload_round_trip_is_canonical() {
    let e = Env::default();
    let p = canonical(&e);
    assert_eq!(encode_price_payload(&e, &decode_price_payload(&e, &p)), p);
}

/// A price with the top bit set decodes to a negative `i128`; the apply path
/// must reject it rather than publish it.
#[test]
fn bridge_payload_high_bit_price_rejected_on_apply() {
    let e = Env::default();
    e.mock_all_auths();
    e.cost_estimate().disable_resource_limits();
    let (client, _admin) = setup_contract(&e);
    e.ledger().with_mut(|l| l.timestamp = 1_000_000);
    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "Axelar");
    client.add_source_asset(&source, &asset);
    let chain = String::from_str(&e, "ethereum");
    let foreign = BytesN::from_array(&e, &[0x11; 32]);
    client.register_foreign_asset_mapping(&asset, &chain, &foreign, &18u32);
    let gateway = soroban_sdk::Address::generate(&e);
    let addr = String::from_str(&e, "0xSourceContract");
    client.set_axelar_gateway(&gateway);
    client.set_axelar_trusted_source(&chain, &addr, &source);

    let mut p = canonical(&e);
    p.set(47, 0x80); // most significant byte of the LE u128 price
    let r = client.try_execute_axelar_message(
        &gateway,
        &BytesN::from_array(&e, &[1; 32]),
        &chain,
        &addr,
        &p,
    );
    assert_eq!(r, Err(Ok(code(ErrorCode::InvalidPrice))));

    // Contextual binding: a well-formed payload for an asset not mapped on
    // this chain is rejected.
    let mut other = canonical(&e);
    other.set(0, 0x99);
    let r = client.try_execute_axelar_message(
        &gateway,
        &BytesN::from_array(&e, &[2; 32]),
        &chain,
        &addr,
        &other,
    );
    assert_eq!(r, Err(Ok(code(ErrorCode::ForeignAssetNotMapped))));
}

// ---------------------------------------------------------------------------
// wormhole_relay::decode_price_payload
// ---------------------------------------------------------------------------

#[test]
fn wormhole_payload_truncation_and_extension_rejected() {
    let e = Env::default();
    let full = crate::wormhole_relay::encode_price_payload(&e, 100, 18, 1_000);
    assert_eq!(full.len(), 28);
    for len in [0u32, 1, 16, 20, 27] {
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::wormhole_relay::decode_price_payload(&e, &full.slice(0..len));
        }));
        assert!(r.is_err(), "truncated length {len} accepted");
    }
    let mut long = full.clone();
    long.push_back(0);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        crate::wormhole_relay::decode_price_payload(&e, &long);
    }));
    assert!(r.is_err(), "trailing byte accepted");
}

#[test]
#[should_panic(expected = "Error(Contract, #147)")]
fn wormhole_payload_trailing_error_is_distinct() {
    let e = Env::default();
    let mut p = crate::wormhole_relay::encode_price_payload(&e, 100, 18, 1_000);
    p.push_back(0);
    crate::wormhole_relay::decode_price_payload(&e, &p);
}

// ---------------------------------------------------------------------------
// timelock batch operation payloads
// ---------------------------------------------------------------------------

fn run_batch(e: &Env, op_type: u32, data: &[u8]) -> bool {
    e.mock_all_auths();
    let (client, _admin) = setup_contract(e);
    let mut ops = soroban_sdk::Vec::new(e);
    ops.push_back(BatchOperation {
        op_type,
        data: Bytes::from_slice(e, data),
    });
    let id = client.propose_batch(&ops);
    let seq = e.ledger().sequence();
    e.ledger().with_mut(|l| l.sequence_number = seq + 1_000);
    client.try_execute_batch(&id).is_ok()
}

#[test]
fn batch_payload_exact_length_accepted() {
    let e = Env::default();
    assert!(run_batch(&e, 3, &[0, 0, 0, 50]));
}

#[test]
fn batch_payload_truncated_rejected() {
    let e = Env::default();
    assert!(!run_batch(&e, 3, &[0, 0, 50]));
}

#[test]
fn batch_payload_trailing_rejected() {
    let e = Env::default();
    assert!(!run_batch(&e, 3, &[0, 0, 0, 50, 0xff]));
}

#[test]
fn batch_upgrade_short_hash_rejected() {
    let e = Env::default();
    assert!(!run_batch(&e, 0, &[1; 31]));
}

// ---------------------------------------------------------------------------
// price_proof external proofs
// ---------------------------------------------------------------------------

#[test]
fn external_proof_short_fields_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "Cex");
    let proof = crate::types::PriceProof {
        proof_type: crate::types::ProofType::CexSignedResponse,
        payload_hash: Bytes::from_slice(&e, &[1; 31]),
        signature: Bytes::from_slice(&e, &[1; 64]),
        signer_count: 0,
    };
    let r = client.try_submit_price_with_external_proof(&source, &asset, &100, &1, &proof);
    assert_eq!(r, Err(Ok(code(ErrorCode::InvalidProof))));
}

fn code(c: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(c as u32)
}
