//! # Off-Chain Signature-Verified Price Submission (Issue #216)
//!
//! Lets an oracle source pre-sign a price observation off-chain with an
//! Ed25519 keypair and have *anyone* relay it on-chain. The contract verifies
//! the signature via the Soroban host's `ed25519_verify` crypto primitive
//! instead of requiring the source's Soroban account to authorize the
//! transaction (`require_auth`) — enabling gas-optimized relayed submissions
//! where the source never has to submit a transaction itself.
//!
//! ## Message format
//!
//! Mirrors the convention used by [`crate::state_channel`]: the signed
//! digest is
//! `sha256("price_proof_v1" || nonce_le(8) || price_le(16) || timestamp_le(8) || expiration_ledger_le(4))`.
//!
//! ## Replay protection & expiry
//!
//! Each source has a strictly-increasing `nonce` counter tracked on-chain; a
//! submission is rejected unless its `nonce` exceeds the last accepted one.
//! `expiration_ledger` additionally bounds how long a signed price may be
//! relayed before it goes stale.

use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env};

use crate::admin::{get_decimals, get_timestamp_threshold};
use crate::assets::get_min_price;
use crate::events::{PriceSubmittedWithProofEvent, SubmissionKeyRegisteredEvent};
use crate::pause::check_not_paused;
use crate::prices::{check_deviation_circuit_breaker, do_aggregate, record_successful_submission};
use crate::sources::{is_source_suspended, record_invalid_submission};
use crate::storage::{
    check_registered_asset, check_source, check_source_asset, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{DataKey, ErrorCode, PriceEntry};

/// Registers (or rotates) the Ed25519 public key used to verify pre-signed
/// price submissions on behalf of `source`. Only `source` itself may call
/// this, via a normal Soroban-authorized transaction.
///
/// # Errors
///
/// * [`ErrorCode::SourceNotFound`] — `source` is not a registered oracle source.
pub fn register_submission_key(env: &Env, source: Address, public_key: BytesN<32>) {
    source.require_auth();
    check_source(env, &source);

    let key = DataKey::SignedSubmitPubKey(source.clone());
    env.storage().persistent().set(&key, &public_key);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SubmissionKeyRegisteredEvent { source, public_key }.publish(env);
}

pub(crate) fn read_submission_key(env: &Env, source: &Address) -> BytesN<32> {
    env.storage()
        .persistent()
        .get(&DataKey::SignedSubmitPubKey(source.clone()))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::SigningKeyNotRegistered))
}

fn hash_proof_payload(
    env: &Env,
    nonce: u64,
    price: i128,
    timestamp: u64,
    expiration_ledger: u32,
) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, b"price_proof_v1"));
    buf.append(&Bytes::from_slice(env, &nonce.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &(price as u128).to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &timestamp.to_le_bytes()));
    buf.append(&Bytes::from_slice(env, &expiration_ledger.to_le_bytes()));
    env.crypto().sha256(&buf).into()
}

/// Submits a price on behalf of `source` using a pre-signed Ed25519 proof
/// instead of the source's Soroban transaction authorization. Any address
/// may call this (typically a relayer bundling submissions from many sources).
///
/// # Arguments
///
/// * `source` - Oracle source the signed price is attributed to.
/// * `asset` - Contract address of the asset being priced.
/// * `price` - Raw price value scaled by `10^decimals`. Must be > 0.
/// * `timestamp` - Unix timestamp (seconds) of the price observation.
/// * `nonce` - Must be strictly greater than the source's last accepted nonce.
/// * `expiration_ledger` - Ledger sequence after which this proof is no longer valid.
/// * `signature` - Ed25519 signature over the payload described above, produced
///   by the key registered via [`register_submission_key`].
///
/// # Errors
///
/// * [`ErrorCode::ContractPaused`] — the contract is currently paused.
/// * [`ErrorCode::SourceNotFound`] — `source` is not a registered oracle source.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::SigningKeyNotRegistered`] — `source` has no registered submission key.
/// * [`ErrorCode::SignatureExpired`] — `expiration_ledger` has already passed.
/// * [`ErrorCode::InvalidNonce`] — `nonce` does not exceed the source's last accepted nonce.
/// * [`ErrorCode::NotAuthorized`] — the Ed25519 signature is invalid, or `source` is suspended.
/// * [`ErrorCode::InvalidPrice`] — `price` is `<= 0`.
/// * [`ErrorCode::PriceBelowMinimum`] — `price` is below the asset's minimum price floor.
/// * [`ErrorCode::InvalidTimestamp`] — `timestamp` is too far in the future.
pub fn submit_price_with_proof(
    env: &Env,
    source: Address,
    asset: Address,
    price: i128,
    timestamp: u64,
    nonce: u64,
    expiration_ledger: u32,
    signature: BytesN<64>,
) {
    check_not_paused(env);
    check_source(env, &source);
    check_registered_asset(env, &asset);
    check_source_asset(env, &source, &asset);

    if is_source_suspended(env, source.clone()) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    if env.ledger().sequence() > expiration_ledger {
        panic_with_error!(env, ErrorCode::SignatureExpired);
    }

    let nonce_key = DataKey::SignedSubmitNonce(source.clone());
    let last_nonce: u64 = env.storage().persistent().get(&nonce_key).unwrap_or(0);
    if nonce <= last_nonce {
        panic_with_error!(env, ErrorCode::InvalidNonce);
    }

    if price <= 0 {
        record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let public_key = read_submission_key(env, &source);
    let digest = hash_proof_payload(env, nonce, price, timestamp, expiration_ledger);
    let digest_bytes: Bytes = digest.into();
    env.crypto()
        .ed25519_verify(&public_key, &digest_bytes, &signature);

    let min_price = get_min_price(env, asset.clone());
    if price < min_price {
        panic_with_error!(env, ErrorCode::PriceBelowMinimum);
    }

    let ledger_time = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    if timestamp > ledger_time.saturating_add(threshold) {
        record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    if check_deviation_circuit_breaker(env, &source, &asset, price) {
        return;
    }

    env.storage().persistent().set(&nonce_key, &nonce);
    env.storage()
        .persistent()
        .extend_ttl(&nonce_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();
    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
        ledger_timestamp: ledger_time,
        volume: None,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

    record_successful_submission(env, source.clone());

    PriceSubmittedWithProofEvent {
        asset: asset.clone(),
        source: source.clone(),
        price,
        timestamp,
        nonce,
    }
    .publish(env);

    crate::triggers::record_submission_for_triggers(env, &asset, price);

    do_aggregate(env, &asset);
}
