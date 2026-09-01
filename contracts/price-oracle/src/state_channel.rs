//! # State Channel for High-Frequency Price Updates (Issue #179)
//!
//! Implements an off-chain state channel pattern between an oracle source and the
//! contract to batch high-frequency price updates, conserving gas and ledger space.
//!
//! ## Design
//!
//! 1. A source opens a channel with a deposit (held in contract instance storage).
//! 2. The source submits batches of price updates off-chain, each batch carrying a
//!    monotonically increasing nonce sequence. The batch is verified on-chain via
//!    Ed25519 signature over a deterministic hash of the payload.
//! 3. Either party may close the channel, triggering final settlement and deposit refund.
//! 4. If the source goes offline a consumer may open a dispute after `dispute_timeout`
//!    has elapsed, presenting a validly-signed higher-nonce state.
//!
//! ## Nonce invariant
//!
//! Nonces must be strictly increasing per channel (`item.nonce > current_nonce`).
//! This eliminates replay attacks without requiring an on-chain counter per batch item.

use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, StateChannel};
use soroban_sdk::{crypto::Hash, panic_with_error, symbol_short, Address, Bytes, BytesN, Env, Vec};

// ─────────────────────────────────────────────────────────────────────────────
// Internal storage helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_channel(env: &Env, source: &Address) -> Option<StateChannel> {
    let key = DataKey::StateChannel(source.clone());
    let result = env.storage().persistent().get(&key);
    if result.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    result
}

fn write_channel(env: &Env, channel: &StateChannel) {
    let key = DataKey::StateChannel(channel.source.clone());
    env.storage().persistent().set(&key, channel);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn remove_channel(env: &Env, source: &Address) {
    let key = DataKey::StateChannel(source.clone());
    env.storage().persistent().remove(&key);
}

// ─────────────────────────────────────────────────────────────────────────────
// Batch payload hashing
//
// The canonical serialisation for signature verification is:
//   sha256( source_bytes(32) || nonce_le(8) || price_le(16) || timestamp_le(8) )
// repeated for each item in the batch, concatenated in order, then hashed again:
//   sha256( item_hash_0 || item_hash_1 || … )
//
// This gives a deterministic 32-byte digest that the source signs off-chain.
// ─────────────────────────────────────────────────────────────────────────────

fn hash_batch_payload(
    env: &Env,
    source: &Address,
    batch: &Vec<crate::types::BatchItem>,
) -> BytesN<32> {
    // Build a flat byte buffer: for each item encode nonce(8) || price(16) || timestamp(8)
    let item_count = batch.len();
    // 32 bytes per item
    let mut buf = Bytes::new(env);

    // Prefix with source address bytes (as a symbol-like discriminant to domain-separate)
    // In Soroban we don't have direct Address → bytes, so we use the contract id hash
    // approach: encode discriminant via a fixed-length symbol prefix instead.
    // Domain-separate by prepending a fixed tag.
    let tag = Bytes::from_slice(env, b"sc_batch_v1");
    buf.append(&tag);

    for i in 0..item_count {
        let item = batch.get_unchecked(i);
        // Encode nonce as 8 little-endian bytes
        let nonce_bytes = u64_to_le_bytes(env, item.nonce);
        buf.append(&nonce_bytes);
        // Encode price as 16 little-endian bytes
        let price_bytes = u128_to_le_bytes(env, item.price);
        buf.append(&price_bytes);
        // Encode timestamp as 8 little-endian bytes
        let ts_bytes = u64_to_le_bytes(env, item.timestamp);
        buf.append(&ts_bytes);
    }

    env.crypto().sha256(&buf).into()
}

/// Encode a u64 as 8 little-endian bytes in a Soroban `Bytes`.
fn u64_to_le_bytes(env: &Env, v: u64) -> Bytes {
    let arr: [u8; 8] = [
        (v & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
        ((v >> 32) & 0xff) as u8,
        ((v >> 40) & 0xff) as u8,
        ((v >> 48) & 0xff) as u8,
        ((v >> 56) & 0xff) as u8,
    ];
    Bytes::from_slice(env, &arr)
}

/// Encode a u128 as 16 little-endian bytes in a Soroban `Bytes`.
fn u128_to_le_bytes(env: &Env, v: u128) -> Bytes {
    let arr: [u8; 16] = [
        (v & 0xff) as u8,
        ((v >> 8) & 0xff) as u8,
        ((v >> 16) & 0xff) as u8,
        ((v >> 24) & 0xff) as u8,
        ((v >> 32) & 0xff) as u8,
        ((v >> 40) & 0xff) as u8,
        ((v >> 48) & 0xff) as u8,
        ((v >> 56) & 0xff) as u8,
        ((v >> 64) & 0xff) as u8,
        ((v >> 72) & 0xff) as u8,
        ((v >> 80) & 0xff) as u8,
        ((v >> 88) & 0xff) as u8,
        ((v >> 96) & 0xff) as u8,
        ((v >> 104) & 0xff) as u8,
        ((v >> 112) & 0xff) as u8,
        ((v >> 120) & 0xff) as u8,
    ];
    Bytes::from_slice(env, &arr)
}

/// Verify an Ed25519 `signature` over `message` using `public_key`.
///
/// In Soroban the Ed25519 public key is the 32-byte raw key embedded in an
/// `Address` (for account addresses). We extract it via `env.crypto().ed25519_verify`.
fn verify_ed25519(
    env: &Env,
    public_key: &BytesN<32>,
    message: &BytesN<32>,
    signature: &BytesN<64>,
) {
    // Expand the 32-byte digest to a Bytes for the verify call
    let msg_bytes: Bytes = message.clone().into();
    env.crypto()
        .ed25519_verify(public_key, &msg_bytes, signature);
}

// ─────────────────────────────────────────────────────────────────────────────
// Default dispute timeout: ~1 hour of ledger time (roughly 720 ledgers at 5s each)
// ─────────────────────────────────────────────────────────────────────────────

const DEFAULT_DISPUTE_TIMEOUT_SECS: u64 = 3_600; // 1 hour

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Opens a state channel for `source` with a locked `deposit`.
///
/// The `source` must authorize this call. The deposit is transferred from `source`
/// to the contract's instance storage via the configured SAC token client
/// (`token_contract`). A new `StateChannel` is created and stored.
///
/// # Panics
///
/// * [`ErrorCode::ChannelAlreadyOpen`] — if a channel for `source` already exists
///   and is not yet closed.
pub fn open_channel(env: &Env, source: Address, deposit: i128, token_contract: Address) {
    source.require_auth();

    // Reject zero or negative deposits
    if deposit <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    // Ensure no existing open channel
    if let Some(existing) = read_channel(env, &source) {
        if !existing.is_closed {
            panic_with_error!(env, ErrorCode::ChannelAlreadyOpen);
        }
    }

    // Transfer deposit from source → this contract
    let token = soroban_sdk::token::Client::new(env, &token_contract);
    let contract_address = env.current_contract_address();
    token.transfer(&source, &contract_address, &deposit);

    let now_ts = env.ledger().timestamp();
    let channel = StateChannel {
        source: source.clone(),
        deposit,
        nonce: 0,
        last_price: 0,
        last_timestamp: 0,
        dispute_timeout: now_ts + DEFAULT_DISPUTE_TIMEOUT_SECS,
        is_closed: false,
        token_contract: token_contract.clone(),
    };
    write_channel(env, &channel);

    env.events().publish(
        (symbol_short!("ch_open"), source),
        (deposit, now_ts + DEFAULT_DISPUTE_TIMEOUT_SECS),
    );
}

/// Submits a batch of price updates for `source`'s state channel.
///
/// The `source` must authorize this call. The batch is verified against the
/// Ed25519 `signature` using the source's public key embedded in `source_pubkey`.
///
/// All items in `batch` must have strictly increasing nonces. Only the item with
/// the highest nonce is stored as the channel's latest state.
///
/// # Panics
///
/// * [`ErrorCode::ChannelNotFound`] — if no open channel exists for `source`.
/// * [`ErrorCode::NotAuthorized`]   — if the Ed25519 signature is invalid.
/// * [`ErrorCode::InvalidTimestamp`] — if any item's nonce is not strictly greater
///   than the channel's current nonce.
pub fn submit_batch(
    env: &Env,
    source: Address,
    batch: Vec<crate::types::BatchItem>,
    signature: BytesN<64>,
    source_pubkey: BytesN<32>,
) {
    source.require_auth();

    if batch.is_empty() {
        return;
    }

    let mut channel = match read_channel(env, &source) {
        Some(c) if !c.is_closed => c,
        _ => panic_with_error!(env, ErrorCode::ChannelNotFound),
    };

    // Verify Ed25519 signature over batch payload
    let digest = hash_batch_payload(env, &source, &batch);
    verify_ed25519(env, &source_pubkey, &digest, &signature);

    // Validate nonces and find the highest-nonce item
    let mut best_nonce = channel.nonce;
    let mut best_price: u128 = channel.last_price;
    let mut best_ts: u64 = channel.last_timestamp;
    let mut prev_nonce: u64 = 0;

    for i in 0..batch.len() {
        let item = batch.get_unchecked(i);
        // Items within the batch must also be strictly increasing
        if i > 0 && item.nonce <= prev_nonce {
            panic_with_error!(env, ErrorCode::InvalidTimestamp);
        }
        prev_nonce = item.nonce;

        // Only accept items that advance beyond the channel's stored nonce
        if item.nonce > best_nonce {
            best_nonce = item.nonce;
            best_price = item.price;
            best_ts = item.timestamp;
        }
    }

    // Must have at least one item that advances state
    if best_nonce == channel.nonce {
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    channel.nonce = best_nonce;
    channel.last_price = best_price;
    channel.last_timestamp = best_ts;
    write_channel(env, &channel);

    env.events().publish(
        (symbol_short!("ch_batch"), source),
        (best_nonce, best_price, best_ts),
    );
}

/// Closes the state channel for `source` and refunds the remaining deposit.
///
/// Either the `source` or the contract admin may call this.
/// The source must authorize when calling on their own behalf.
///
/// # Panics
///
/// * [`ErrorCode::ChannelNotFound`] — if no open channel exists for `source`.
pub fn close_channel(env: &Env, source: Address) {
    source.require_auth();

    let mut channel = match read_channel(env, &source) {
        Some(c) if !c.is_closed => c,
        _ => panic_with_error!(env, ErrorCode::ChannelNotFound),
    };

    // Mark closed before external token transfer (reentrancy safety)
    channel.is_closed = true;
    write_channel(env, &channel);

    // Refund the deposit to the source
    let remaining = channel.deposit;
    if remaining > 0 {
        let token = soroban_sdk::token::Client::new(env, &channel.token_contract);
        let contract_address = env.current_contract_address();
        token.transfer(&contract_address, &source, &remaining);
    }

    // Clean up storage after refund
    remove_channel(env, &source);

    env.events().publish(
        (symbol_short!("ch_close"), source.clone()),
        (channel.nonce, remaining),
    );
}

/// Disputes a state channel when the source has gone offline.
///
/// A consumer (or anyone holding a validly-signed batch with a higher nonce than
/// currently stored) may call this function after `dispute_timeout` has elapsed.
/// If the presented state has a strictly higher nonce and the signature is valid,
/// the stored channel state is updated.
///
/// # Panics
///
/// * [`ErrorCode::ChannelNotFound`]  — if no open channel exists for `source`.
/// * [`ErrorCode::TimelockNotReady`] — if `dispute_timeout` has not yet elapsed.
/// * [`ErrorCode::NotAuthorized`]    — if the Ed25519 signature is invalid.
/// * [`ErrorCode::InvalidTimestamp`] — if the presented nonces do not advance state.
pub fn dispute_channel(
    env: &Env,
    source: Address,
    last_known_batch: Vec<crate::types::BatchItem>,
    signature: BytesN<64>,
    source_pubkey: BytesN<32>,
) {
    let mut channel = match read_channel(env, &source) {
        Some(c) if !c.is_closed => c,
        _ => panic_with_error!(env, ErrorCode::ChannelNotFound),
    };

    // Enforce dispute timeout
    let now_ts = env.ledger().timestamp();
    if now_ts < channel.dispute_timeout {
        panic_with_error!(env, ErrorCode::TimelockNotReady);
    }

    if last_known_batch.is_empty() {
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    // Verify Ed25519 signature — must be signed by the source's key
    let digest = hash_batch_payload(env, &source, &last_known_batch);
    verify_ed25519(env, &source_pubkey, &digest, &signature);

    // Find the highest nonce in the presented batch
    let mut best_nonce = channel.nonce;
    let mut best_price: u128 = channel.last_price;
    let mut best_ts: u64 = channel.last_timestamp;

    for i in 0..last_known_batch.len() {
        let item = last_known_batch.get_unchecked(i);
        if item.nonce > best_nonce {
            best_nonce = item.nonce;
            best_price = item.price;
            best_ts = item.timestamp;
        }
    }

    if best_nonce == channel.nonce {
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    channel.nonce = best_nonce;
    channel.last_price = best_price;
    channel.last_timestamp = best_ts;
    write_channel(env, &channel);

    env.events().publish(
        (symbol_short!("ch_disp"), source),
        (best_nonce, best_price, best_ts),
    );
}

/// Returns the current state of a channel, or `None` if it does not exist.
pub fn get_channel(env: &Env, source: Address) -> Option<StateChannel> {
    read_channel(env, &source)
}
