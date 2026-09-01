//! # Shared Cross-Chain Bridge Plumbing
//!
//! Code shared by every bridge integration ([`crate::axelar_gmp`],
//! [`crate::layerzero`]):
//!
//! * A single wire format ([`crate::types::CrossChainPricePayload`]) for the
//!   price-update message carried over any transport.
//! * A single "apply this bridged price" path that mirrors the pre-signed
//!   submission flow in [`crate::signed_submission`] (min-price / staleness /
//!   circuit-breaker checks, aggregation trigger) and additionally records
//!   the observation into the existing cross-chain reference-price
//!   verification store ([`crate::cross_chain_verify`]) so it can be
//!   compared against locally aggregated prices from other sources.

use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, String};

use crate::admin::{get_decimals, get_timestamp_threshold};
use crate::assets::get_min_price;
use crate::pause::check_not_paused;
use crate::prices::{check_deviation_circuit_breaker, do_aggregate, record_successful_submission};
use crate::sources::{is_source_suspended, record_invalid_submission};
use crate::storage::{
    check_registered_asset, check_source, check_source_asset, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{CrossChainPricePayload, DataKey, ErrorCode, PriceEntry};

/// Wire length of the canonical [`CrossChainPricePayload`] encoding:
/// `foreign_asset(32) || price_le(16) || decimals_le(4) || timestamp_le(8) || nonce_le(8)`.
pub const PRICE_PAYLOAD_LEN: u32 = 32 + 16 + 4 + 8 + 8;

fn read_array<const N: usize>(bytes: &Bytes, offset: u32) -> [u8; N] {
    let mut arr = [0u8; N];
    for i in 0..N {
        arr[i] = bytes.get_unchecked(offset + i as u32);
    }
    arr
}

/// Encodes a [`CrossChainPricePayload`] into its canonical wire format.
pub fn encode_price_payload(env: &Env, payload: &CrossChainPricePayload) -> Bytes {
    let mut buf = Bytes::new(env);
    buf.append(&payload.foreign_asset.clone().into());
    buf.append(&Bytes::from_slice(env, &(payload.price as u128).to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &payload.decimals.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &payload.timestamp.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &payload.nonce.to_le_bytes()));
    buf
}

/// Decodes the canonical wire format into a [`CrossChainPricePayload`].
///
/// # Errors
///
/// * [`ErrorCode::InvalidProof`] — `bytes` is not exactly [`PRICE_PAYLOAD_LEN`] long.
pub fn decode_price_payload(env: &Env, bytes: &Bytes) -> CrossChainPricePayload {
    if bytes.len() != PRICE_PAYLOAD_LEN {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }

    let foreign_asset: [u8; 32] = read_array(bytes, 0);
    let price_bytes: [u8; 16] = read_array(bytes, 32);
    let decimals_bytes: [u8; 4] = read_array(bytes, 48);
    let timestamp_bytes: [u8; 8] = read_array(bytes, 52);
    let nonce_bytes: [u8; 8] = read_array(bytes, 60);

    CrossChainPricePayload {
        foreign_asset: BytesN::from_array(env, &foreign_asset),
        price: u128::from_le_bytes(price_bytes) as i128,
        decimals: u32::from_le_bytes(decimals_bytes),
        timestamp: u64::from_le_bytes(timestamp_bytes),
        nonce: u64::from_le_bytes(nonce_bytes),
    }
}

/// Applies a price update that has already been authenticated by a bridge
/// integration, treating `bridge_source` (a registered oracle source
/// dedicated to that bridge pathway) as the submitter.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — `bridge_source` is not a registered source,
///   is not authorized for `asset`, or has been suspended.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::InvalidPrice`] — `price` is `<= 0`.
/// * [`ErrorCode::PriceBelowMinimum`] — `price` is below the asset's minimum price floor.
/// * [`ErrorCode::InvalidTimestamp`] — `timestamp` is too far in the future.
pub(crate) fn apply_bridged_price(
    env: &Env,
    bridge_source: Address,
    asset: Address,
    price: i128,
    timestamp: u64,
    payload_decimals: u32,
    chain_id: String,
) {
    check_not_paused(env);
    check_source(env, &bridge_source);
    check_registered_asset(env, &asset);
    check_source_asset(env, &bridge_source, &asset);

    if is_source_suspended(env, bridge_source.clone()) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    if price <= 0 {
        record_invalid_submission(env, bridge_source.clone());
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let min_price = get_min_price(env, asset.clone());
    if price < min_price {
        panic_with_error!(env, ErrorCode::PriceBelowMinimum);
    }

    let ledger_time = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    if timestamp > ledger_time.saturating_add(threshold) {
        record_invalid_submission(env, bridge_source.clone());
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    if check_deviation_circuit_breaker(env, &bridge_source, &asset, price) {
        return;
    }

    // Bridge into the existing cross-chain reference-price verification store
    // (#226) so `verify_cross_chain_price` can compare this bridged
    // observation against the price aggregated from other sources.
    crate::cross_chain_verify::record_reference_price(
        env,
        asset.clone(),
        bridge_source.clone(),
        price,
        payload_decimals,
        chain_id,
        timestamp,
    );

    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();
    let entry = PriceEntry {
        price,
        timestamp,
        source: bridge_source.clone(),
        decimals,
        last_updated: current_ledger,
        ledger_timestamp: ledger_time,
        volume: None,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), bridge_source.clone()), &entry);
    env.storage().persistent().extend_ttl(
        &DataKey::Submission(asset.clone(), bridge_source.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    record_successful_submission(env, bridge_source);
    crate::triggers::record_submission_for_triggers(env, &asset, price);

    do_aggregate(env, &asset);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_decode_round_trip() {
        let env = Env::default();
        let payload = CrossChainPricePayload {
            foreign_asset: BytesN::from_array(&env, &[9u8; 32]),
            price: 123_456_789_000_000_000i128,
            decimals: 18,
            timestamp: 1_700_000_000,
            nonce: 42,
        };

        let encoded = encode_price_payload(&env, &payload);
        assert_eq!(encoded.len(), PRICE_PAYLOAD_LEN);

        let decoded = decode_price_payload(&env, &encoded);
        assert_eq!(decoded, payload);
    }

    #[test]
    #[should_panic]
    fn test_decode_rejects_wrong_length() {
        let env = Env::default();
        let bad = Bytes::from_slice(&env, &[0u8; 10]);
        decode_price_payload(&env, &bad);
    }
}
