//! # LayerZero Integration
//!
//! Delivers cross-chain price data via a LayerZero OApp `lzReceive`-style
//! handler, with the endpoint's ordered-delivery guarantee re-checked at
//! this contract for defense-in-depth (mirroring LayerZero V2's
//! `OAppReceiver._acceptNonce`).
//!
//! ## Trust model
//!
//! Mirrors [`crate::axelar_gmp`]: the admin-configured Endpoint address is
//! the sole trusted caller. A real LayerZero Endpoint has already verified
//! DVN/executor attestations for the pathway before invoking `lz_receive`,
//! so this contract only needs to confirm the call truly came from that
//! Endpoint (`endpoint.require_auth()`) and that `(src_eid, sender)` is an
//! allow-listed remote OApp.
//!
//! ## Ordering
//!
//! Each `(src_eid, sender)` pathway has a strictly increasing nonce. A
//! delivery is accepted only when `nonce == last_nonce + 1`; a replay, gap,
//! or reorder is rejected with [`ErrorCode::LzNonceOutOfOrder`].
//!
//! ## Message format
//!
//! `message` bytes use the same [`crate::types::CrossChainPricePayload`]
//! wire format as the Axelar integration (see [`crate::bridge_common`]).
//! Because LayerZero identifies source chains by a numeric `src_eid` rather
//! than the string chain names used elsewhere in the
//! [`crate::asset_registry`], each `src_eid` is mapped once to its canonical
//! registry chain name via [`set_lz_chain_name`].
//!
//! ## Bridging into existing cross-chain verification
//!
//! Applied prices flow through [`crate::bridge_common::apply_bridged_price`],
//! which records the observation into the existing cross-chain reference
//! price store ([`crate::cross_chain_verify`]) alongside Axelar-sourced and
//! admin-submitted observations.

use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, String};

use crate::asset_registry::resolve_enabled_mapping;
use crate::bridge_common::{apply_bridged_price, decode_price_payload};
use crate::events::{LzEndpointSetEvent, LzMessageReceivedEvent, LzTrustedRemoteSetEvent};
use crate::pause::check_not_paused;
use crate::storage::{check_source, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode};

/// Configures the trusted LayerZero Endpoint contract address. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
pub fn set_layerzero_endpoint(env: &Env, endpoint: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage().persistent().set(&DataKey::LzEndpoint, &endpoint);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::LzEndpoint, LEDGER_THRESHOLD, LEDGER_BUMP);

    LzEndpointSetEvent { endpoint }.publish(env);
}

/// Returns the currently configured LayerZero Endpoint address, if any.
pub fn get_layerzero_endpoint(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&DataKey::LzEndpoint)
}

/// Maps a LayerZero source endpoint id onto a canonical
/// [`crate::asset_registry`] chain name (e.g. `30101 -> "ethereum"`).
/// Admin-only.
pub fn set_lz_chain_name(env: &Env, src_eid: u32, chain: String) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::LzEidChainName(src_eid);
    env.storage().persistent().set(&key, &chain);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_chain_name(env: &Env, src_eid: u32) -> String {
    env.storage()
        .persistent()
        .get(&DataKey::LzEidChainName(src_eid))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::LzChainNameNotConfigured))
}

/// Registers `bridge_source` — an already-registered oracle source (see
/// [`crate::sources::add_source`]) — as the attribution target for messages
/// from the `(src_eid, sender)` pathway. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::NotAuthorized`] (via [`check_source`]) — `bridge_source` is
///   not a registered oracle source.
pub fn set_trusted_remote(env: &Env, src_eid: u32, sender: BytesN<32>, bridge_source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    check_source(env, &bridge_source);

    let key = DataKey::LzTrustedRemote(src_eid, sender.clone());
    env.storage().persistent().set(&key, &bridge_source);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    LzTrustedRemoteSetEvent {
        bridge_source,
        src_eid,
        sender,
    }
    .publish(env);
}

/// Revokes a trusted LayerZero remote pathway. Admin-only.
pub fn remove_trusted_remote(env: &Env, src_eid: u32, sender: BytesN<32>) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .remove(&DataKey::LzTrustedRemote(src_eid, sender));
}

fn read_trusted_remote(env: &Env, src_eid: u32, sender: &BytesN<32>) -> Address {
    env.storage()
        .persistent()
        .get(&DataKey::LzTrustedRemote(src_eid, sender.clone()))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::LzRemoteNotTrusted))
}

/// Returns the last accepted inbound nonce for a `(src_eid, sender)` pathway,
/// or `0` if none has been delivered yet.
pub fn get_inbound_nonce(env: &Env, src_eid: u32, sender: BytesN<32>) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::LzInboundNonce(src_eid, sender))
        .unwrap_or(0)
}

/// Delivers a price update via a LayerZero `lzReceive`-style call.
///
/// `endpoint` must equal the configured trusted Endpoint address and must
/// authorize this call — satisfied automatically when the real LayerZero
/// Endpoint contract invokes this function directly after its own
/// DVN/executor verification for the pathway.
///
/// # Errors
///
/// * [`ErrorCode::LzEndpointNotConfigured`] — no Endpoint has been configured.
/// * [`ErrorCode::NotAuthorized`] — `endpoint` does not match the configured Endpoint.
/// * [`ErrorCode::LzNonceOutOfOrder`] — `nonce` is not exactly one greater than
///   the last accepted nonce for this `(src_eid, sender)` pathway.
/// * [`ErrorCode::LzRemoteNotTrusted`] — no bridge source is registered for
///   `(src_eid, sender)`.
/// * [`ErrorCode::LzChainNameNotConfigured`] — `src_eid` has no registry chain name.
/// * [`ErrorCode::ForeignAssetNotMapped`] / [`ErrorCode::ForeignAssetMappingDisabled`] —
///   the payload's foreign asset id has no active registry mapping on that chain.
pub fn lz_receive(
    env: &Env,
    endpoint: Address,
    src_eid: u32,
    sender: BytesN<32>,
    nonce: u64,
    guid: BytesN<32>,
    message: Bytes,
) {
    check_not_paused(env);

    let configured = get_layerzero_endpoint(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::LzEndpointNotConfigured));
    if endpoint != configured {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
    endpoint.require_auth();

    let nonce_key = DataKey::LzInboundNonce(src_eid, sender.clone());
    let last_nonce: u64 = env.storage().persistent().get(&nonce_key).unwrap_or(0);
    if nonce != last_nonce + 1 {
        panic_with_error!(env, ErrorCode::LzNonceOutOfOrder);
    }
    env.storage().persistent().set(&nonce_key, &nonce);
    env.storage()
        .persistent()
        .extend_ttl(&nonce_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let bridge_source = read_trusted_remote(env, src_eid, &sender);
    let chain = read_chain_name(env, src_eid);
    let payload = decode_price_payload(env, &message);
    let mapping = resolve_enabled_mapping(env, &chain, &payload.foreign_asset);

    apply_bridged_price(
        env,
        bridge_source.clone(),
        mapping.stellar_asset.clone(),
        payload.price,
        payload.timestamp,
        payload.decimals,
        chain,
    );

    LzMessageReceivedEvent {
        asset: mapping.stellar_asset,
        bridge_source,
        src_eid,
        sender,
        nonce,
        guid,
        price: payload.price,
    }
    .publish(env);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge_common::encode_price_payload;
    use crate::types::CrossChainPricePayload;
    use soroban_sdk::testutils::{Address as _, Ledger};

    const ETH_EID: u32 = 30101;

    fn setup(env: &Env) -> (Address, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let asset = Address::generate(env);
        crate::admin::initialize(env, admin.clone(), 1, 100, 18, String::from_str(env, "Oracle"));
        crate::assets::register_asset(env, asset.clone());
        env.ledger().with_mut(|l| l.timestamp = 1_000_000);
        (admin, asset, Address::generate(env))
    }

    fn wire_lz(
        env: &Env,
        asset: &Address,
        endpoint: &Address,
        bridge_source: &Address,
        sender: &BytesN<32>,
        chain: &String,
        foreign_address: &BytesN<32>,
    ) {
        crate::sources::add_source(env, bridge_source.clone(), String::from_str(env, "LayerZero"));
        crate::sources::add_source_asset(env, bridge_source.clone(), asset.clone());
        set_layerzero_endpoint(env, endpoint.clone());
        set_lz_chain_name(env, ETH_EID, chain.clone());
        set_trusted_remote(env, ETH_EID, sender.clone(), bridge_source.clone());
        crate::asset_registry::register_foreign_asset_mapping(
            env,
            asset.clone(),
            chain.clone(),
            foreign_address.clone(),
            18,
        );
    }

    #[test]
    fn test_lz_receive_updates_price_and_nonce() {
        let env = Env::default();
        let (_, asset, endpoint) = setup(&env);
        let bridge_source = Address::generate(&env);
        let sender = BytesN::from_array(&env, &[21u8; 32]);
        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[22u8; 32]);
        wire_lz(&env, &asset, &endpoint, &bridge_source, &sender, &chain, &foreign_address);

        let payload = CrossChainPricePayload {
            foreign_asset: foreign_address,
            price: 2_000_000_000_000_000_000i128,
            decimals: 18,
            timestamp: env.ledger().timestamp(),
            nonce: 1,
        };
        let encoded = encode_price_payload(&env, &payload);

        lz_receive(
            &env,
            endpoint,
            ETH_EID,
            sender.clone(),
            1,
            BytesN::from_array(&env, &[1u8; 32]),
            encoded,
        );

        assert_eq!(get_inbound_nonce(&env, ETH_EID, sender.clone()), 1);
        let entry: crate::types::PriceEntry = env
            .storage()
            .persistent()
            .get(&DataKey::Submission(asset, bridge_source))
            .unwrap();
        assert_eq!(entry.price, 2_000_000_000_000_000_000i128);
    }

    #[test]
    #[should_panic]
    fn test_lz_receive_rejects_out_of_order_nonce() {
        let env = Env::default();
        let (_, asset, endpoint) = setup(&env);
        let bridge_source = Address::generate(&env);
        let sender = BytesN::from_array(&env, &[23u8; 32]);
        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[24u8; 32]);
        wire_lz(&env, &asset, &endpoint, &bridge_source, &sender, &chain, &foreign_address);

        let payload = CrossChainPricePayload {
            foreign_asset: foreign_address,
            price: 1_000_000_000_000_000_000i128,
            decimals: 18,
            timestamp: env.ledger().timestamp(),
            nonce: 1,
        };
        let encoded = encode_price_payload(&env, &payload);

        // Skips nonce 1, jumps straight to 2 — must be rejected.
        lz_receive(
            &env,
            endpoint,
            ETH_EID,
            sender,
            2,
            BytesN::from_array(&env, &[2u8; 32]),
            encoded,
        );
    }

    #[test]
    #[should_panic]
    fn test_lz_receive_rejects_untrusted_endpoint() {
        let env = Env::default();
        let (_, asset, endpoint) = setup(&env);
        let bridge_source = Address::generate(&env);
        let sender = BytesN::from_array(&env, &[25u8; 32]);
        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[26u8; 32]);
        wire_lz(&env, &asset, &endpoint, &bridge_source, &sender, &chain, &foreign_address);

        let payload = CrossChainPricePayload {
            foreign_asset: foreign_address,
            price: 1_000_000_000_000_000_000i128,
            decimals: 18,
            timestamp: env.ledger().timestamp(),
            nonce: 1,
        };
        let encoded = encode_price_payload(&env, &payload);
        let impostor = Address::generate(&env);

        lz_receive(
            &env,
            impostor,
            ETH_EID,
            sender,
            1,
            BytesN::from_array(&env, &[3u8; 32]),
            encoded,
        );
    }
}
