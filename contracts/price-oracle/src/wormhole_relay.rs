//! # Wormhole Price Relay (VAA Verification)
//!
//! Lets verified prices observed on other Wormhole-connected chains feed this
//! oracle, by verifying a Wormhole VAA (Verified Action Approval) on-chain and
//! mapping its payload into the existing [`crate::cross_chain_verify`] storage —
//! the same `submit_cross_chain_price` path the admin-driven cross-reference
//! checks already use (#226).
//!
//! ## Flow
//!
//! 1. Admin registers the current Wormhole guardian set ([`set_guardian_set`])
//!    and a `wormhole_chain_id → oracle_chain address` mapping
//!    ([`set_chain_mapping`]) so relayed prices land under the right
//!    `DataKey::CrossChainPrice(asset, oracle_chain)` key.
//! 2. An off-chain relayer submits a [`WormholeVaa`] carrying the source
//!    guardians' signatures and an encoded price payload.
//! 3. [`submit_price_via_wormhole`] verifies a quorum of guardian signatures,
//!    checks the VAA hasn't been replayed, decodes the payload, and writes it
//!    into `CrossChainPrice` via
//!    [`crate::cross_chain_verify::store_cross_chain_price`].
//!
//! Anyone may call `submit_price_via_wormhole` — the guardian quorum itself is
//! the authorization, not the caller's identity (the same trust model as
//! Wormhole's real Ethereum-side relayers).
//!
//! ## Guardian signature scheme (simplification)
//!
//! Real-world Wormhole guardians sign a `keccak256(keccak256(body))` digest with
//! secp256k1 and are identified by 20-byte Ethereum-style addresses. This
//! contract instead verifies Ed25519 signatures directly against registered
//! guardian public keys over a `sha256(sha256(body))` digest — the same
//! simplification [`crate::cross_chain_relay::verify_validator_set`] already
//! makes for Stellar SCP validators, since Soroban's host crypto surface used
//! elsewhere in this contract exposes `sha256` / `ed25519_verify` directly. The
//! quorum logic (accept once `>= quorum` distinct guardians have validly signed)
//! mirrors Wormhole's actual 2/3-plus-one guardian consensus rule.

use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, String, Vec};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, WormholeGuardianSet, WormholePricePayload, WormholeVaa};

/// Length in bytes of the encoded price payload: `price(16) || decimals(4) || timestamp(8)`.
const PAYLOAD_LEN: u32 = 28;

// ─── Guardian set & chain mapping configuration ────────────────────────────────

/// Registers (or rotates) the Wormhole guardian set. Admin-only.
///
/// `quorum` must be in `1..=guardians.len()`. The stored set's `set_index` is
/// bumped by one on every call, starting at `0` for the first registration.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] — `guardians` is empty, or `quorum` is
///   `0` or exceeds `guardians.len()`.
pub fn set_guardian_set(env: &Env, guardians: Vec<BytesN<32>>, quorum: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if guardians.is_empty() || quorum == 0 || quorum > guardians.len() {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let next_index = get_guardian_set(env).map(|s| s.set_index + 1).unwrap_or(0);
    let set = WormholeGuardianSet {
        guardians: guardians.clone(),
        set_index: next_index,
    };

    let key = DataKey::WormholeGuardianSet;
    env.storage().persistent().set(&key, &set);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let quorum_key = DataKey::WormholeGuardianQuorum;
    env.storage().persistent().set(&quorum_key, &quorum);
    env.storage()
        .persistent()
        .extend_ttl(&quorum_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    crate::events::WormholeGuardianSetEvent {
        set_index: next_index,
        guardian_count: guardians.len(),
        quorum,
    }
    .publish(env);
}

/// Returns the currently registered guardian set, or `None` if never configured.
pub fn get_guardian_set(env: &Env) -> Option<WormholeGuardianSet> {
    env.storage()
        .persistent()
        .get(&DataKey::WormholeGuardianSet)
}

/// Returns the currently configured guardian quorum (minimum valid signatures).
pub fn get_guardian_quorum(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::WormholeGuardianQuorum)
        .unwrap_or(0)
}

/// Maps a Wormhole chain id (e.g. `2` = Ethereum) to the `Address` key this
/// oracle uses under `DataKey::CrossChainPrice(asset, oracle_chain)`. Admin-only.
pub fn set_chain_mapping(env: &Env, wormhole_chain_id: u32, oracle_chain: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::WormholeChainMapping(wormhole_chain_id);
    env.storage().persistent().set(&key, &oracle_chain);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns the oracle-chain address mapped to `wormhole_chain_id`, if any.
pub fn get_chain_mapping(env: &Env, wormhole_chain_id: u32) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::WormholeChainMapping(wormhole_chain_id))
}

// ─── VAA verification ───────────────────────────────────────────────────────────

/// Computes the Wormhole-style double-SHA256 digest of a VAA body:
/// `emitter_chain(4 BE) || emitter_address(32) || sequence(8 BE) || payload`.
fn compute_body_hash(env: &Env, vaa: &WormholeVaa) -> BytesN<32> {
    let mut body = Bytes::new(env);
    body.append(&Bytes::from_slice(env, &vaa.emitter_chain.to_be_bytes()));
    body.append(&vaa.emitter_address.clone().into());
    body.append(&Bytes::from_slice(env, &vaa.sequence.to_be_bytes()));
    body.append(&vaa.payload);

    let first: Bytes = env.crypto().sha256(&body).into();
    env.crypto().sha256(&first).into()
}

/// Verifies that a quorum of registered guardians have validly signed `vaa`.
///
/// Guardian indices must be unique and in range; a duplicate index is ignored
/// rather than double-counted. As with
/// [`crate::cross_chain_relay::verify_validator_set`], an invalid Ed25519
/// signature panics the call (Soroban's `ed25519_verify` has no fallible form) —
/// callers should only submit signatures they expect to be valid.
///
/// # Panics
/// * [`ErrorCode::GuardianSetNotConfigured`] — no guardian set registered.
/// * [`ErrorCode::InvalidGuardianSignatureSet`] — mismatched/empty signature
///   arrays, or a `guardian_indices` entry is out of range.
pub fn verify_vaa_quorum(env: &Env, vaa: &WormholeVaa) -> bool {
    let guardian_set = get_guardian_set(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::GuardianSetNotConfigured));
    let quorum = get_guardian_quorum(env);

    if vaa.signatures.is_empty() || vaa.signatures.len() != vaa.guardian_indices.len() {
        panic_with_error!(env, ErrorCode::InvalidGuardianSignatureSet);
    }

    let body_hash = compute_body_hash(env, vaa);
    let msg: Bytes = body_hash.into();

    let mut seen_indices: Vec<u32> = Vec::new(env);
    let mut valid_count: u32 = 0;

    for i in 0..vaa.guardian_indices.len() {
        let idx = vaa.guardian_indices.get_unchecked(i);
        if idx >= guardian_set.guardians.len() {
            panic_with_error!(env, ErrorCode::InvalidGuardianSignatureSet);
        }

        let mut duplicate = false;
        for j in 0..seen_indices.len() {
            if seen_indices.get_unchecked(j) == idx {
                duplicate = true;
                break;
            }
        }
        if duplicate {
            continue;
        }
        seen_indices.push_back(idx);

        let guardian_pk = guardian_set.guardians.get_unchecked(idx);
        let sig = vaa.signatures.get_unchecked(i);
        env.crypto().ed25519_verify(&guardian_pk, &msg, &sig);
        valid_count += 1;
    }

    valid_count >= quorum
}

/// Decodes a VAA payload into `(price, decimals, timestamp)`.
///
/// Layout: `price(16 bytes LE) || decimals(4 bytes LE) || timestamp(8 bytes LE)`.
///
/// # Panics
/// * [`ErrorCode::InvalidVaaPayload`] — payload length is not exactly 28 bytes.
pub fn decode_price_payload(env: &Env, payload: &Bytes) -> WormholePricePayload {
    if payload.len() != PAYLOAD_LEN {
        panic_with_error!(env, ErrorCode::InvalidVaaPayload);
    }

    let mut price_bytes = [0u8; 16];
    for i in 0..16u32 {
        price_bytes[i as usize] = payload.get_unchecked(i);
    }
    let mut decimals_bytes = [0u8; 4];
    for i in 0..4u32 {
        decimals_bytes[i as usize] = payload.get_unchecked(16 + i);
    }
    let mut ts_bytes = [0u8; 8];
    for i in 0..8u32 {
        ts_bytes[i as usize] = payload.get_unchecked(20 + i);
    }

    WormholePricePayload {
        price: i128::from_le_bytes(price_bytes),
        decimals: u32::from_le_bytes(decimals_bytes),
        timestamp: u64::from_le_bytes(ts_bytes),
    }
}

/// Encodes `(price, decimals, timestamp)` into the payload format
/// [`decode_price_payload`] expects. Used by relayers/tests to build a VAA.
pub fn encode_price_payload(env: &Env, price: i128, decimals: u32, timestamp: u64) -> Bytes {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, &price.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &decimals.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &timestamp.to_le_bytes()));
    buf
}

// ─── Relay entrypoint ────────────────────────────────────────────────────────

/// Verifies `vaa` and, on success, maps its price payload into
/// `submit_cross_chain_price`'s storage for `asset`.
///
/// Callable by anyone — the verified guardian quorum is the authorization.
///
/// # Panics
/// * [`ErrorCode::GuardianSetNotConfigured`] / [`ErrorCode::InvalidGuardianSignatureSet`]
///   — see [`verify_vaa_quorum`].
/// * [`ErrorCode::GuardianQuorumNotMet`] — fewer than the configured quorum of
///   guardians validly signed.
/// * [`ErrorCode::UnmappedWormholeChain`] — no oracle-chain mapping registered
///   for `vaa.emitter_chain`.
/// * [`ErrorCode::VaaAlreadyProcessed`] — `vaa.sequence` does not exceed the
///   last accepted sequence for this emitter (replay).
/// * [`ErrorCode::InvalidVaaPayload`] — payload is not a valid encoded price.
/// * [`ErrorCode::AssetNotRegistered`] / [`ErrorCode::InvalidPrice`] — see
///   [`crate::cross_chain_verify::store_cross_chain_price`].
pub fn submit_price_via_wormhole(env: &Env, asset: Address, vaa: WormholeVaa) {
    if !verify_vaa_quorum(env, &vaa) {
        panic_with_error!(env, ErrorCode::GuardianQuorumNotMet);
    }

    let oracle_chain = get_chain_mapping(env, vaa.emitter_chain)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::UnmappedWormholeChain));

    let seq_key = DataKey::WormholeLastSequence(vaa.emitter_chain, vaa.emitter_address.clone());
    let last_sequence: u64 = env.storage().persistent().get(&seq_key).unwrap_or(0);
    if vaa.sequence <= last_sequence && last_sequence > 0 {
        panic_with_error!(env, ErrorCode::VaaAlreadyProcessed);
    }
    if vaa.sequence == 0 {
        panic_with_error!(env, ErrorCode::VaaAlreadyProcessed);
    }

    let payload = decode_price_payload(env, &vaa.payload);

    crate::cross_chain_verify::store_cross_chain_price(
        env,
        asset.clone(),
        oracle_chain,
        payload.price,
        payload.decimals,
        String::from_str(env, "wormhole"),
        payload.timestamp,
    );

    env.storage().persistent().set(&seq_key, &vaa.sequence);
    env.storage()
        .persistent()
        .extend_ttl(&seq_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    crate::events::WormholePriceRelayedEvent {
        asset,
        emitter_chain: vaa.emitter_chain,
        price: payload.price,
        sequence: vaa.sequence,
    }
    .publish(env);
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    /// Builds a deterministic Ed25519 keypair from a single-byte seed, for
    /// reproducible mock-VAA tests. EdDSA signing needs no RNG.
    fn make_guardian(seed: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed; 32])
    }

    fn guardian_pubkey(env: &Env, sk: &SigningKey) -> BytesN<32> {
        BytesN::from_array(env, &sk.verifying_key().to_bytes())
    }

    /// Signs the Wormhole-style double-SHA256 digest of `vaa`'s body fields
    /// (signatures/guardian_indices are excluded from the digest, so an unsigned
    /// draft VAA can be passed here before its `signatures` field is filled in).
    fn sign_body(env: &Env, sk: &SigningKey, vaa: &WormholeVaa) -> BytesN<64> {
        let hash = compute_body_hash(env, vaa);
        let sig = sk.sign(&hash.to_array());
        BytesN::from_array(env, &sig.to_bytes())
    }

    fn init(env: &Env) -> Address {
        let admin = Address::generate(env);
        env.ledger().with_mut(|l| l.timestamp = 1000);
        crate::admin::initialize(
            env,
            admin.clone(),
            1,
            100,
            18,
            String::from_str(env, "Oracle"),
        );
        admin
    }

    fn draft_vaa(env: &Env, emitter_chain: u32, sequence: u64, price: i128) -> WormholeVaa {
        WormholeVaa {
            emitter_chain,
            emitter_address: BytesN::from_array(env, &[0x42u8; 32]),
            sequence,
            payload: encode_price_payload(env, price, 18, 1_700_000_000u64),
            signatures: Vec::new(env),
            guardian_indices: Vec::new(env),
        }
    }

    #[test]
    fn test_payload_round_trip() {
        let env = Env::default();
        let payload = encode_price_payload(&env, 123_456_789i128, 18, 1_700_000_000u64);
        let decoded = decode_price_payload(&env, &payload);
        assert_eq!(decoded.price, 123_456_789i128);
        assert_eq!(decoded.decimals, 18);
        assert_eq!(decoded.timestamp, 1_700_000_000u64);
    }

    #[test]
    #[should_panic]
    fn test_decode_rejects_wrong_length() {
        let env = Env::default();
        let bad = Bytes::from_slice(&env, &[0u8; 10]);
        decode_price_payload(&env, &bad);
    }

    #[test]
    fn test_guardian_set_registration_and_quorum_default() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);
        let pk = guardian_pubkey(&env, &make_guardian(1));

        let mut guardians: Vec<BytesN<32>> = Vec::new(&env);
        guardians.push_back(pk.clone());

        set_guardian_set(&env, guardians.clone(), 1);
        let set = get_guardian_set(&env).unwrap();
        assert_eq!(set.guardians.len(), 1);
        assert_eq!(set.set_index, 0);
        assert_eq!(get_guardian_quorum(&env), 1);

        // Rotating bumps the index.
        set_guardian_set(&env, guardians, 1);
        assert_eq!(get_guardian_set(&env).unwrap().set_index, 1);
    }

    #[test]
    fn test_chain_mapping_round_trip() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);
        let ethereum_marker = Address::generate(&env);

        assert!(get_chain_mapping(&env, 2).is_none());
        set_chain_mapping(&env, 2, ethereum_marker.clone());
        assert_eq!(get_chain_mapping(&env, 2), Some(ethereum_marker));
    }

    /// End-to-end: a VAA co-signed by 2-of-3 registered guardians (meeting a
    /// quorum of 2) verifies successfully.
    #[test]
    fn test_verify_vaa_quorum_with_real_signatures() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);

        let g1 = make_guardian(1);
        let g2 = make_guardian(2);
        let g3 = make_guardian(3);
        let mut guardians: Vec<BytesN<32>> = Vec::new(&env);
        guardians.push_back(guardian_pubkey(&env, &g1));
        guardians.push_back(guardian_pubkey(&env, &g2));
        guardians.push_back(guardian_pubkey(&env, &g3));
        set_guardian_set(&env, guardians, 2);

        let mut vaa = draft_vaa(&env, 2, 1, 100_000_000_000_000_000_000);
        let sig1 = sign_body(&env, &g1, &vaa);
        let sig3 = sign_body(&env, &g3, &vaa);
        vaa.signatures.push_back(sig1);
        vaa.signatures.push_back(sig3);
        vaa.guardian_indices.push_back(0);
        vaa.guardian_indices.push_back(2);

        assert!(verify_vaa_quorum(&env, &vaa));
    }

    /// A single valid signature is not enough when quorum is 2.
    #[test]
    fn test_verify_vaa_quorum_not_met() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);

        let g1 = make_guardian(1);
        let g2 = make_guardian(2);
        let mut guardians: Vec<BytesN<32>> = Vec::new(&env);
        guardians.push_back(guardian_pubkey(&env, &g1));
        guardians.push_back(guardian_pubkey(&env, &g2));
        set_guardian_set(&env, guardians, 2);

        let mut vaa = draft_vaa(&env, 2, 1, 100);
        let sig1 = sign_body(&env, &g1, &vaa);
        vaa.signatures.push_back(sig1);
        vaa.guardian_indices.push_back(0);

        assert!(!verify_vaa_quorum(&env, &vaa));
    }

    /// A signature that does not match the claimed guardian index is rejected
    /// (Soroban's `ed25519_verify` panics rather than returning `false`).
    #[test]
    #[should_panic]
    fn test_verify_vaa_rejects_mismatched_signature() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);

        let g1 = make_guardian(1);
        let impostor = make_guardian(99);
        let mut guardians: Vec<BytesN<32>> = Vec::new(&env);
        guardians.push_back(guardian_pubkey(&env, &g1));
        set_guardian_set(&env, guardians, 1);

        let mut vaa = draft_vaa(&env, 2, 1, 100);
        // Signed by a guardian that is NOT in the registered set, but claimed
        // under index 0 (which belongs to `g1`).
        let bad_sig = sign_body(&env, &impostor, &vaa);
        vaa.signatures.push_back(bad_sig);
        vaa.guardian_indices.push_back(0);

        verify_vaa_quorum(&env, &vaa);
    }

    #[test]
    #[should_panic]
    fn test_verify_without_guardian_set_panics() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);
        let vaa = draft_vaa(&env, 2, 1, 100);
        verify_vaa_quorum(&env, &vaa);
    }

    /// Full flow: register guardians + chain mapping + asset, submit a
    /// guardian-quorum-signed VAA, and confirm the price lands in
    /// `cross_chain_verify`'s storage under the mapped oracle-chain address.
    #[test]
    fn test_submit_price_via_wormhole_end_to_end() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = init(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());

        let g1 = make_guardian(1);
        let g2 = make_guardian(2);
        let mut guardians: Vec<BytesN<32>> = Vec::new(&env);
        guardians.push_back(guardian_pubkey(&env, &g1));
        guardians.push_back(guardian_pubkey(&env, &g2));
        set_guardian_set(&env, guardians, 2);

        let ethereum_marker = Address::generate(&env);
        set_chain_mapping(&env, 2, ethereum_marker.clone());
        let _ = admin;

        let price = 3_500_000_000_000_000_000i128; // 3.5 * 1e18
        let mut vaa = draft_vaa(&env, 2, 1, price);
        let sig1 = sign_body(&env, &g1, &vaa);
        let sig2 = sign_body(&env, &g2, &vaa);
        vaa.signatures.push_back(sig1);
        vaa.signatures.push_back(sig2);
        vaa.guardian_indices.push_back(0);
        vaa.guardian_indices.push_back(1);

        submit_price_via_wormhole(&env, asset.clone(), vaa);

        let stored =
            crate::cross_chain_verify::get_cross_chain_price(&env, &asset, &ethereum_marker)
                .expect("price should be stored");
        assert_eq!(stored.price, price);
        assert_eq!(stored.decimals, 18);
    }

    /// The same VAA (same emitter + sequence) cannot be relayed twice.
    #[test]
    #[should_panic]
    fn test_replay_protection_rejects_duplicate_sequence() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());

        let g1 = make_guardian(1);
        let mut guardians: Vec<BytesN<32>> = Vec::new(&env);
        guardians.push_back(guardian_pubkey(&env, &g1));
        set_guardian_set(&env, guardians, 1);

        let oracle_chain = Address::generate(&env);
        set_chain_mapping(&env, 2, oracle_chain);

        let mut vaa = draft_vaa(&env, 2, 1, 100);
        let sig1 = sign_body(&env, &g1, &vaa);
        vaa.signatures.push_back(sig1);
        vaa.guardian_indices.push_back(0);

        submit_price_via_wormhole(&env, asset.clone(), vaa.clone());
        // Same sequence again — must be rejected as a replay.
        submit_price_via_wormhole(&env, asset, vaa);
    }
}
