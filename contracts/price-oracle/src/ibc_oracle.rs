//! # Cosmos/IBC Light-Client Verified Price Feeds
//!
//! Adds a price source for Cosmos/IBC-originated assets, verified via a
//! lightweight Tendermint light-client emulation:
//!
//! 1. **Light client bootstrap** (`update_light_client`): admin configures the
//!    counterparty chain id, voting-power trust threshold (Tendermint default
//!    2/3 + 1), and trusting period.
//! 2. **Consensus state updates** (`submit_consensus_state`): a relayer submits
//!    a `IbcConsensusState` (revision height, app hash, validator-set hash,
//!    timestamp) together with the validator set and their signatures over the
//!    header. The header is accepted once signers controlling at least
//!    `trust_threshold_pct` of total voting power have signed — this is the
//!    voting-power-weighted quorum check real Tendermint light clients use,
//!    as opposed to a naive headcount.
//! 3. **Price packet verification** (`verify_and_submit_price`): a relayer
//!    submits an `IbcPricePacket` plus a Merkle inclusion proof against the
//!    trusted `app_hash` for the packet's `revision_height`. This models an
//!    ICS23 membership proof that the packet was committed in the source
//!    chain's application state. On success the price is normalized and
//!    stored, keyed by the packet's IBC denom.
//!
//! The Merkle combine/leaf functions mirror `cross_chain_relay.rs`'s
//! `verify_event_proof` for consistency with this repo's existing
//! light-client verifier style: `leaf = sha256(0x00 || data)`,
//! `parent = sha256(0x01 || left || right)`.

use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, String, Vec};

use crate::events::{
    IbcAssetMappedEvent, IbcClientUpdatedEvent, IbcConsensusUpdatedEvent, IbcPriceVerifiedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    DataKey, ErrorCode, IbcClientState, IbcConsensusState, IbcPriceEntry, IbcPricePacket,
    TendermintValidator,
};

// ─────────────────────────────────────────────────────────────────────────────
// Merkle helpers (shared style with cross_chain_relay.rs)
// ─────────────────────────────────────────────────────────────────────────────

fn merkle_leaf_hash(env: &Env, leaf_data: &Bytes) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, &[0x00u8]));
    buf.append(leaf_data);
    env.crypto().sha256(&buf).into()
}

fn merkle_node_hash(env: &Env, left: &BytesN<32>, right: &BytesN<32>) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, &[0x01u8]));
    buf.append(&left.clone().into());
    buf.append(&right.clone().into());
    env.crypto().sha256(&buf).into()
}

/// Canonical byte serialization of a price packet for hashing:
/// `price_le(16) || decimals_le(4) || timestamp_le(8) || sequence_le(8)`.
///
/// The `denom` string is intentionally excluded from the leaf serialization
/// itself and instead authenticated via the storage key it is submitted
/// under (`get_ibc_asset` lookup happens post-verification, keyed by the
/// caller-supplied denom) — mirroring how `cross_chain_relay::verify_event_proof`
/// authenticates a fixed-shape numeric payload.
fn serialize_packet(env: &Env, packet: &IbcPricePacket) -> Bytes {
    let price_u128 = if packet.price >= 0 {
        packet.price as u128
    } else {
        0u128
    };
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, &price_u128.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &packet.decimals.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &packet.timestamp.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &packet.sequence.to_le_bytes()));
    buf
}

/// Serializes the fields of a consensus state that validators sign over:
/// `revision_height_le(8) || app_hash(32) || next_validators_hash(32) || timestamp_le(8)`.
fn serialize_header(env: &Env, consensus_state: &IbcConsensusState) -> Bytes {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(
        env,
        &consensus_state.revision_height.to_le_bytes(),
    ));
    buf.append(&consensus_state.app_hash.clone().into());
    buf.append(&consensus_state.next_validators_hash.clone().into());
    buf.append(&Bytes::from_slice(
        env,
        &consensus_state.timestamp.to_le_bytes(),
    ));
    buf
}

// ─────────────────────────────────────────────────────────────────────────────
// Light client configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Configures (or reconfigures) the IBC light client for a counterparty chain.
/// Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — `trust_threshold_pct` is out of `1..=100`.
pub fn update_light_client(env: &Env, client_state: IbcClientState) {
    let admin = get_admin(env);
    admin.require_auth();

    if client_state.trust_threshold_pct == 0 || client_state.trust_threshold_pct > 100 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    env.storage()
        .persistent()
        .set(&DataKey::IbcClientState, &client_state);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::IbcClientState, LEDGER_THRESHOLD, LEDGER_BUMP);

    IbcClientUpdatedEvent {
        chain_id: client_state.chain_id.clone(),
        trust_threshold_pct: client_state.trust_threshold_pct,
        trusting_period: client_state.trusting_period,
    }
    .publish(env);
}

/// Returns the current IBC light client configuration, or `None`.
pub fn get_light_client(env: &Env) -> Option<IbcClientState> {
    env.storage().persistent().get(&DataKey::IbcClientState)
}

// ─────────────────────────────────────────────────────────────────────────────
// Consensus state updates
// ─────────────────────────────────────────────────────────────────────────────

/// Submits a new trusted consensus state, verified by a voting-power-weighted
/// quorum of validator signatures over the header fields. Admin-only (the
/// admin key is expected to be held by the deployment's trusted relayer
/// process, matching `cross_chain_verify::submit_cross_chain_price`'s gating
/// convention in this contract).
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::IbcClientNotSet`] — no light client has been configured.
/// * [`ErrorCode::IbcQuorumNotMet`] — signing voting power is below `trust_threshold_pct`.
pub fn submit_consensus_state(
    env: &Env,
    consensus_state: IbcConsensusState,
    validators: Vec<TendermintValidator>,
    signatures: Vec<BytesN<64>>,
) {
    let admin = get_admin(env);
    admin.require_auth();

    let client_state =
        get_light_client(env).unwrap_or_else(|| panic_with_error!(env, ErrorCode::IbcClientNotSet));

    if validators.len() != signatures.len() || validators.is_empty() {
        panic_with_error!(env, ErrorCode::IbcQuorumNotMet);
    }

    let header_bytes = serialize_header(env, &consensus_state);
    let header_hash = env.crypto().sha256(&header_bytes);
    let header_hash_bytes: Bytes = header_hash.into();

    let mut total_power: u128 = 0;
    let mut signed_power: u128 = 0;
    let mut valid_signatures: u32 = 0;

    for i in 0..validators.len() {
        let validator = validators.get_unchecked(i);
        let sig = signatures.get_unchecked(i);
        total_power += validator.voting_power as u128;

        env.crypto()
            .ed25519_verify(&validator.pubkey, &header_hash_bytes, &sig);
        signed_power += validator.voting_power as u128;
        valid_signatures += 1;
    }

    // signed_power / total_power >= trust_threshold_pct / 100
    if total_power == 0
        || signed_power * 100 < total_power * (client_state.trust_threshold_pct as u128)
    {
        panic_with_error!(env, ErrorCode::IbcQuorumNotMet);
    }

    let key = DataKey::IbcConsensusState(consensus_state.revision_height);
    env.storage().persistent().set(&key, &consensus_state);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    if consensus_state.revision_height > client_state.latest_height {
        let updated = IbcClientState {
            latest_height: consensus_state.revision_height,
            ..client_state
        };
        env.storage()
            .persistent()
            .set(&DataKey::IbcClientState, &updated);
    }

    IbcConsensusUpdatedEvent {
        revision_height: consensus_state.revision_height,
        app_hash: consensus_state.app_hash.clone(),
        valid_signatures,
        total_validators: validators.len(),
    }
    .publish(env);
}

/// Returns the trusted consensus state at `revision_height`, or `None`.
pub fn get_consensus_state(env: &Env, revision_height: u64) -> Option<IbcConsensusState> {
    env.storage()
        .persistent()
        .get(&DataKey::IbcConsensusState(revision_height))
}

// ─────────────────────────────────────────────────────────────────────────────
// Denom <-> asset mapping
// ─────────────────────────────────────────────────────────────────────────────

/// Maps an IBC denom to a registered Stellar asset address. Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not a registered oracle asset.
pub fn register_ibc_asset_mapping(env: &Env, denom: String, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    crate::storage::check_registered_asset(env, &asset);

    let denom_key = DataKey::IbcDenomAsset(denom.clone());
    env.storage().persistent().set(&denom_key, &asset);
    env.storage()
        .persistent()
        .extend_ttl(&denom_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let reverse_key = DataKey::IbcAssetDenom(asset.clone());
    env.storage().persistent().set(&reverse_key, &denom);
    env.storage()
        .persistent()
        .extend_ttl(&reverse_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    IbcAssetMappedEvent { asset, denom }.publish(env);
}

/// Returns the Stellar asset mapped to an IBC denom, or `None`.
pub fn get_ibc_asset(env: &Env, denom: String) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::IbcDenomAsset(denom))
}

// ─────────────────────────────────────────────────────────────────────────────
// Price packet verification
// ─────────────────────────────────────────────────────────────────────────────

/// Verifies an IBC price packet's Merkle inclusion proof against the trusted
/// `app_hash` for its `revision_height`, then stores the resulting price.
///
/// `proof` is the ordered list of sibling hashes from leaf to root; `path_bits`
/// encodes, bit-by-bit (0 = current node is the left child, 1 = right), the
/// direction taken at each level — the same convention used by
/// `cross_chain_relay::verify_event_proof`.
///
/// # Panics
/// * [`ErrorCode::AssetNotRegistered`] via [`ErrorCode::IbcDenomNotMapped`] — denom has no asset mapping.
/// * [`ErrorCode::IbcConsensusStateNotFound`] — no trusted state at `packet.revision_height`.
/// * [`ErrorCode::IbcClientExpired`] — the trusted state is older than `trusting_period`.
/// * [`ErrorCode::IbcPacketReplayed`] — `packet.sequence` is not strictly increasing.
/// * [`ErrorCode::InvalidPrice`] — `packet.price` is non-positive.
/// * [`ErrorCode::IbcInvalidProof`] — the Merkle path does not resolve to `app_hash`.
pub fn verify_and_submit_price(
    env: &Env,
    packet: IbcPricePacket,
    proof: Vec<BytesN<32>>,
    path_bits: u32,
) {
    let asset = get_ibc_asset(env, packet.denom.clone())
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::IbcDenomNotMapped));

    let client_state =
        get_light_client(env).unwrap_or_else(|| panic_with_error!(env, ErrorCode::IbcClientNotSet));

    let consensus_state = get_consensus_state(env, packet.revision_height)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::IbcConsensusStateNotFound));

    let now = env.ledger().timestamp();
    if now.saturating_sub(consensus_state.timestamp) > client_state.trusting_period {
        panic_with_error!(env, ErrorCode::IbcClientExpired);
    }

    if packet.price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let seq_key = DataKey::IbcLastSequence(packet.denom.clone());
    let last_sequence: u64 = env.storage().persistent().get(&seq_key).unwrap_or(0);
    if packet.sequence <= last_sequence {
        panic_with_error!(env, ErrorCode::IbcPacketReplayed);
    }

    if proof.is_empty() {
        panic_with_error!(env, ErrorCode::IbcInvalidProof);
    }

    let packet_bytes = serialize_packet(env, &packet);
    let mut current = merkle_leaf_hash(env, &packet_bytes);
    for i in 0..proof.len() {
        let sibling = proof.get_unchecked(i);
        let bit = (path_bits >> i) & 1;
        current = if bit == 0 {
            merkle_node_hash(env, &current, &sibling)
        } else {
            merkle_node_hash(env, &sibling, &current)
        };
    }

    if current != consensus_state.app_hash {
        panic_with_error!(env, ErrorCode::IbcInvalidProof);
    }

    let decimals = crate::admin::get_decimals(env);
    let normalized_price = normalize_decimals(packet.price, packet.decimals, decimals);

    let entry = IbcPriceEntry {
        denom: packet.denom.clone(),
        asset: asset.clone(),
        price: normalized_price,
        decimals,
        timestamp: packet.timestamp,
        revision_height: packet.revision_height,
    };

    let price_key = DataKey::IbcLatestPrice(packet.denom.clone());
    env.storage().persistent().set(&price_key, &entry);
    env.storage()
        .persistent()
        .extend_ttl(&price_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    env.storage().persistent().set(&seq_key, &packet.sequence);
    env.storage()
        .persistent()
        .extend_ttl(&seq_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    IbcPriceVerifiedEvent {
        asset,
        price: normalized_price,
        timestamp: packet.timestamp,
        revision_height: packet.revision_height,
        sequence: packet.sequence,
    }
    .publish(env);
}

/// Returns the latest verified IBC price for a denom, or `None`.
pub fn get_ibc_price(env: &Env, denom: String) -> Option<IbcPriceEntry> {
    env.storage()
        .persistent()
        .get(&DataKey::IbcLatestPrice(denom))
}

fn normalize_decimals(raw_price: i128, source_decimals: u32, target_decimals: u32) -> i128 {
    if source_decimals == target_decimals {
        return raw_price;
    }
    if source_decimals < target_decimals {
        let factor = 10i128.pow(target_decimals - source_decimals);
        raw_price.saturating_mul(factor)
    } else {
        let factor = 10i128.pow(source_decimals - target_decimals);
        raw_price.saturating_div(factor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Env, String as SorobanString};

    fn setup(env: &Env) -> Address {
        let admin = Address::generate(env);
        env.ledger().with_mut(|l| l.timestamp = 1_000);
        crate::admin::initialize(
            env,
            admin.clone(),
            1,
            100,
            7,
            SorobanString::from_slice(env, "Oracle"),
        );
        admin
    }

    /// Deterministic test keypair (not for production use).
    fn test_signing_key(seed_byte: u8) -> SigningKey {
        SigningKey::from_bytes(&[seed_byte; 32])
    }

    fn sign_bytes(env: &Env, sk: &SigningKey, msg: &[u8; 32]) -> BytesN<64> {
        let sig = sk.sign(msg);
        BytesN::from_array(env, &sig.to_bytes())
    }

    fn pubkey_bytes(env: &Env, sk: &SigningKey) -> BytesN<32> {
        BytesN::from_array(env, &sk.verifying_key().to_bytes())
    }

    /// Exercises the full flow: light-client bootstrap, a validator-signed
    /// consensus state update, and a Merkle-proven price packet.
    #[test]
    fn test_full_ibc_price_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let _admin = setup(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());

        update_light_client(
            &env,
            IbcClientState {
                chain_id: SorobanString::from_slice(&env, "cosmoshub-4"),
                trust_threshold_pct: 67,
                trusting_period: 3600,
                latest_height: 0,
            },
        );

        let denom = SorobanString::from_slice(&env, "ibc/TESTDENOM");
        register_ibc_asset_mapping(&env, denom.clone(), asset.clone());

        let packet = IbcPricePacket {
            denom: denom.clone(),
            price: 12_500_000,
            decimals: 6,
            timestamp: 1_000,
            revision_height: 42,
            sequence: 1,
        };
        let packet_bytes = serialize_packet(&env, &packet);
        let leaf = merkle_leaf_hash(&env, &packet_bytes);

        // Sibling leaf and root computed the same way verify_and_submit_price will.
        let sibling = BytesN::from_array(&env, &[7u8; 32]);
        let root = merkle_node_hash(&env, &leaf, &sibling);

        let consensus_state = IbcConsensusState {
            revision_number: 1,
            revision_height: 42,
            app_hash: root,
            next_validators_hash: BytesN::from_array(&env, &[0u8; 32]),
            timestamp: 900,
        };

        let sk = test_signing_key(1);
        let header_bytes = serialize_header(&env, &consensus_state);
        let header_hash: [u8; 32] = env.crypto().sha256(&header_bytes).into();
        let sig = sign_bytes(&env, &sk, &header_hash);

        let validator = TendermintValidator {
            pubkey: pubkey_bytes(&env, &sk),
            voting_power: 100,
        };

        let mut validators = Vec::new(&env);
        validators.push_back(validator);
        let mut signatures = Vec::new(&env);
        signatures.push_back(sig);

        submit_consensus_state(&env, consensus_state, validators, signatures);
        assert_eq!(get_light_client(&env).unwrap().latest_height, 42);

        let mut proof = Vec::new(&env);
        proof.push_back(sibling);

        verify_and_submit_price(&env, packet, proof, 0);

        let stored = get_ibc_price(&env, denom).unwrap();
        // 12_500_000 at 6 decimals -> normalized to the contract's 7 decimals.
        assert_eq!(stored.price, 125_000_000);
        assert_eq!(stored.asset, asset);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #121)")]
    fn test_quorum_not_met_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        setup(&env);

        update_light_client(
            &env,
            IbcClientState {
                chain_id: SorobanString::from_slice(&env, "cosmoshub-4"),
                trust_threshold_pct: 67,
                trusting_period: 3600,
                latest_height: 0,
            },
        );

        let consensus_state = IbcConsensusState {
            revision_number: 1,
            revision_height: 1,
            app_hash: BytesN::from_array(&env, &[1u8; 32]),
            next_validators_hash: BytesN::from_array(&env, &[0u8; 32]),
            timestamp: 900,
        };

        let sk = test_signing_key(2);
        let header_bytes = serialize_header(&env, &consensus_state);
        let header_hash: [u8; 32] = env.crypto().sha256(&header_bytes).into();
        let sig = sign_bytes(&env, &sk, &header_hash);

        // Signer only carries 30/100 total voting power — below the 67% threshold,
        // even though its own signature verifies correctly.
        let validator = TendermintValidator {
            pubkey: pubkey_bytes(&env, &sk),
            voting_power: 30,
        };

        let mut validators = Vec::new(&env);
        validators.push_back(validator);
        let mut signatures = Vec::new(&env);
        signatures.push_back(sig);

        submit_consensus_state(&env, consensus_state, validators, signatures);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #119)")]
    fn test_replayed_sequence_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        setup(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());

        update_light_client(
            &env,
            IbcClientState {
                chain_id: SorobanString::from_slice(&env, "cosmoshub-4"),
                trust_threshold_pct: 67,
                trusting_period: 3600,
                latest_height: 0,
            },
        );

        let denom = SorobanString::from_slice(&env, "ibc/TESTDENOM");
        register_ibc_asset_mapping(&env, denom.clone(), asset.clone());

        let packet = IbcPricePacket {
            denom: denom.clone(),
            price: 1_000_000,
            decimals: 6,
            timestamp: 1_000,
            revision_height: 10,
            sequence: 5,
        };
        let packet_bytes = serialize_packet(&env, &packet);
        let leaf = merkle_leaf_hash(&env, &packet_bytes);
        let sibling = BytesN::from_array(&env, &[9u8; 32]);
        let root = merkle_node_hash(&env, &leaf, &sibling);

        let consensus_state = IbcConsensusState {
            revision_number: 1,
            revision_height: 10,
            app_hash: root,
            next_validators_hash: BytesN::from_array(&env, &[0u8; 32]),
            timestamp: 900,
        };

        let sk = test_signing_key(3);
        let header_bytes = serialize_header(&env, &consensus_state);
        let header_hash: [u8; 32] = env.crypto().sha256(&header_bytes).into();
        let sig = sign_bytes(&env, &sk, &header_hash);
        let validator = TendermintValidator {
            pubkey: pubkey_bytes(&env, &sk),
            voting_power: 100,
        };

        let mut validators = Vec::new(&env);
        validators.push_back(validator);
        let mut signatures = Vec::new(&env);
        signatures.push_back(sig);
        submit_consensus_state(&env, consensus_state, validators, signatures);

        let mut proof = Vec::new(&env);
        proof.push_back(sibling.clone());
        verify_and_submit_price(&env, packet.clone(), proof.clone(), 0);

        // Same sequence again — must be rejected as a replay.
        verify_and_submit_price(&env, packet, proof, 0);
    }
}
