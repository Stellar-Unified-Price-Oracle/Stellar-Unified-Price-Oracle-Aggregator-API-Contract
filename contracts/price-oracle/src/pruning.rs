//! # Timestamp-Based Price Pruning (Issue #288)
//!
//! Extends the existing ledger-count–based history pruning with an optional
//! **timestamp-based retention window** configurable per asset.
//!
//! When a retention window is set for an asset, price history entries whose
//! `timestamp` is older than `current_time - retention_seconds` are pruned
//! automatically on the next aggregation cycle.  The two modes can be used
//! together: entries are pruned when they violate *either* the ledger count
//! limit *or* the timestamp window.
//!
//! ## Storage layout
//!
//! | Key | Type | Description |
//! |-----|------|-------------|
//! | `AssetRetentionWindow(asset)` | `u64` | Per-asset retention in seconds |
//!
//! ## Functions
//!
//! - [`set_asset_retention_window`] — admin: configure per-asset retention window.
//! - [`get_asset_retention_window`] — query the configured window.
//! - [`prune_by_timestamp`] — prune stale history entries for an asset.
//! - [`prune_combined`] — apply both ledger-count and timestamp pruning.
//! - [`prune_history`] — admin: explicitly prune the oldest entries for an asset
//!   down to a target entry count, separate from auto-pruning on aggregation.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::admin::get_max_history_length;
use crate::events::{emit_history_pruned_by_timestamp, HistoryPrunedEvent};
use crate::history::remove_history_shard_entry;
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, PriceHistoryEntry};

// ─── Storage helpers ────────────────────────────────────────────────────────

/// Writes the retention window (in seconds) for a specific asset.
fn write_retention_window(env: &Env, asset: &Address, seconds: u64) {
    let key = DataKey::AssetRetentionWindow(asset.clone());
    env.storage().persistent().set(&key, &seconds);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Reads the retention window for an asset. Returns `0` when not set
/// (meaning no timestamp-based pruning is applied).
fn read_retention_window(env: &Env, asset: &Address) -> u64 {
    let key = DataKey::AssetRetentionWindow(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0u64)
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Configures the timestamp-based retention window for a specific asset.
///
/// Only the admin may call this function.
///
/// # Arguments
///
/// * `env` — The Soroban execution environment.
/// * `asset` — Asset contract address.
/// * `retention_seconds` — How many seconds of history to keep. Pass `0` to
///   disable timestamp-based pruning for this asset.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
/// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
pub fn set_asset_retention_window(env: &Env, asset: Address, retention_seconds: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);
    write_retention_window(env, &asset, retention_seconds);
}

/// Returns the timestamp-based retention window for an asset in seconds.
///
/// Returns `0` when no window has been configured (no timestamp pruning).
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
pub fn get_asset_retention_window(env: &Env, asset: Address) -> u64 {
    check_registered_asset(env, &asset);
    read_retention_window(env, &asset)
}

/// Prunes all history entries for `asset` whose `timestamp` is older than
/// `current_ledger_time - retention_seconds`.
///
/// This function iterates the stored `PriceHistoryLedgers` index, removes
/// entries that are too old from both temporary storage and the index, and
/// emits a [`HistoryPrunedByTimestampEvent`] for each removed entry.
///
/// If no retention window is configured (`0`), the function is a no-op.
///
/// # Arguments
///
/// * `env` — The Soroban execution environment.
/// * `asset` — Asset whose history should be pruned.
///
/// # Returns
///
/// Number of entries that were pruned.
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
pub fn prune_by_timestamp(env: &Env, asset: Address) -> u32 {
    check_registered_asset(env, &asset);

    let retention = read_retention_window(env, &asset);
    if retention == 0 {
        return 0;
    }

    let now = env.ledger().timestamp();
    let cutoff = now.saturating_sub(retention);

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: soroban_sdk::Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(soroban_sdk::Vec::new(env));

    if ledger_list.is_empty() {
        return 0;
    }

    let mut kept: soroban_sdk::Vec<u32> = soroban_sdk::Vec::new(env);
    let mut pruned_count: u32 = 0;

    for i in 0..ledger_list.len() {
        let ledger_seq = ledger_list.get_unchecked(i);
        let hist_key = DataKey::PriceHistory(asset.clone(), ledger_seq);

        let entry: Option<PriceHistoryEntry> = env.storage().temporary().get(&hist_key);
        match entry {
            Some(e) if e.timestamp < cutoff => {
                // Prune this entry
                env.storage().temporary().remove(&hist_key);
                pruned_count += 1;
                emit_history_pruned_by_timestamp(
                    env,
                    asset.clone(),
                    ledger_seq,
                    e.timestamp,
                    cutoff,
                );
            }
            _ => {
                kept.push_back(ledger_seq);
            }
        }
    }

    if pruned_count > 0 {
        env.storage().persistent().set(&ledgers_key, &kept);
        env.storage()
            .persistent()
            .extend_ttl(&ledgers_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    pruned_count
}

/// Applies **combined** pruning to an asset's history:
///
/// 1. First prunes by timestamp window (if configured).
/// 2. Then prunes by ledger-count limit (enforces `max_history_length`).
///
/// This is called automatically from `prices::submit_price` when a new
/// aggregate is recorded.
///
/// # Returns
///
/// Total number of entries pruned.
pub fn prune_combined(env: &Env, asset: &Address) -> u32 {
    // Step 1: timestamp pruning
    let ts_pruned = prune_by_timestamp(env, asset.clone());

    // Step 2: ledger-count pruning (mirrors existing logic in prices.rs, but
    // operates on the possibly-already-trimmed index)
    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let mut ledger_list: soroban_sdk::Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(soroban_sdk::Vec::new(env));

    let max_history = get_max_history_length(env);
    let mut count_pruned: u32 = 0;

    while ledger_list.len() > max_history {
        let oldest_ledger = ledger_list.get_unchecked(0);
        ledger_list.remove(0);
        env.storage()
            .temporary()
            .remove(&DataKey::PriceHistory(asset.clone(), oldest_ledger));
        count_pruned += 1;
        HistoryPrunedEvent {
            asset: asset.clone(),
            pruned_ledger: oldest_ledger,
            remaining: ledger_list.len(),
        }
        .publish(env);
    }

    if count_pruned > 0 {
        env.storage().persistent().set(&ledgers_key, &ledger_list);
        env.storage()
            .persistent()
            .extend_ttl(&ledgers_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    ts_pruned + count_pruned
}

/// Removes the retention window configuration for an asset (admin only).
///
/// After calling this, timestamp-based pruning is disabled for the asset
/// until a new window is configured.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
/// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
pub fn remove_asset_retention_window(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);
    let key = DataKey::AssetRetentionWindow(asset.clone());
    if !env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::NoData);
    }
    env.storage().persistent().remove(&key);
}

// ─── Explicit operator-triggered pruning ───────────────────────────────────

/// Explicitly prunes the **oldest** history entries for `asset` down to
/// `target_entries`, removing entries one at a time from both the ledger
/// index and their underlying storage (per-ledger temporary storage and any
/// weekly shard bucket).
///
/// This is distinct from the automatic pruning applied during aggregation
/// (which enforces `max_history_length`/`max_history_per_asset`): it lets an
/// operator proactively reclaim storage for an asset on demand, regardless
/// of the currently configured limits.
///
/// Admin-only. A `target_entries` at or above the current entry count is a
/// no-op and returns `0`.
///
/// # Arguments
///
/// * `env` — The Soroban execution environment.
/// * `asset` — Asset whose history should be pruned.
/// * `target_entries` — Desired number of history entries to retain after
///   pruning. Entries beyond this count (oldest first) are removed.
///
/// # Returns
///
/// Number of entries that were pruned.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
/// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
pub fn prune_history(env: &Env, asset: Address, target_entries: u32) -> u32 {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let mut ledger_list: soroban_sdk::Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(soroban_sdk::Vec::new(env));

    let mut pruned_count: u32 = 0;
    while ledger_list.len() > target_entries {
        let oldest_ledger = ledger_list.get_unchecked(0);
        ledger_list.remove(0);
        env.storage()
            .temporary()
            .remove(&DataKey::PriceHistory(asset.clone(), oldest_ledger));
        remove_history_shard_entry(env, &asset, oldest_ledger);
        pruned_count += 1;
        HistoryPrunedEvent {
            asset: asset.clone(),
            pruned_ledger: oldest_ledger,
            remaining: ledger_list.len(),
        }
        .publish(env);
    }

    if pruned_count > 0 {
        env.storage().persistent().set(&ledgers_key, &ledger_list);
        env.storage()
            .persistent()
            .extend_ttl(&ledgers_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    pruned_count
}
