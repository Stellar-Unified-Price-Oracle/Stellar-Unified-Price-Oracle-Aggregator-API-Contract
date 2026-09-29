//! # Configurable Aggregation Triggers (Issue #218)
//!
//! Complements the unconditional per-submission aggregation already performed
//! by [`crate::prices`] with three independently configurable, opt-in
//! triggers that keep an asset's aggregate price fresh during low-submission
//! periods and refresh it immediately when a submission looks materially
//! different from the current aggregate:
//!
//! * **Time-based** — [`poke_time_trigger`] may be called by anyone (e.g. a
//!   keeper bot); it only re-aggregates once at least the configured
//!   `interval` seconds have elapsed since the last trigger-driven
//!   aggregation for that asset.
//! * **Threshold-based** — once `threshold` new submissions have accumulated
//!   for an asset since the last trigger-driven aggregation, the next
//!   submission automatically re-aggregates.
//! * **Deviation-based** — if an incoming submission's price differs from the
//!   current aggregate by at least `deviation_bps` (basis points),
//!   aggregation runs immediately instead of waiting for the next scheduled
//!   aggregation.
//!
//! All three triggers default to disabled (`0`) and must be explicitly
//! configured per-asset by the admin, so existing behavior is unchanged
//! unless a trigger is opted into.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{AutoTriggerFiredEvent, TriggerConfigChangedEvent};
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregatePrice, DataKey, ErrorCode};

/// `trigger_type` discriminant: time interval elapsed.
pub const TRIGGER_TYPE_TIME: u32 = 0;
/// `trigger_type` discriminant: submission count threshold reached.
pub const TRIGGER_TYPE_THRESHOLD: u32 = 1;
/// `trigger_type` discriminant: price deviation threshold exceeded.
pub const TRIGGER_TYPE_DEVIATION: u32 = 2;

// ---------------------------------------------------------------------------
// Admin configuration: time-based trigger
// ---------------------------------------------------------------------------

pub fn set_time_trigger(env: &Env, asset: Address, interval_seconds: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let key = DataKey::TriggerTimeInterval(asset.clone());
    env.storage().persistent().set(&key, &interval_seconds);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    TriggerConfigChangedEvent {
        asset,
        trigger_type: TRIGGER_TYPE_TIME,
        value: interval_seconds as i128,
    }
    .publish(env);
}

pub fn get_time_trigger(env: &Env, asset: Address) -> u64 {
    let key = DataKey::TriggerTimeInterval(asset);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Admin configuration: submission-count threshold trigger
// ---------------------------------------------------------------------------

pub fn set_submission_threshold_trigger(env: &Env, asset: Address, threshold: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let key = DataKey::TriggerSubmissionThreshold(asset.clone());
    env.storage().persistent().set(&key, &threshold);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    TriggerConfigChangedEvent {
        asset,
        trigger_type: TRIGGER_TYPE_THRESHOLD,
        value: threshold as i128,
    }
    .publish(env);
}

pub fn get_submission_threshold_trigger(env: &Env, asset: Address) -> u32 {
    let key = DataKey::TriggerSubmissionThreshold(asset);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Admin configuration: deviation threshold trigger
// ---------------------------------------------------------------------------

pub fn set_deviation_trigger(env: &Env, asset: Address, deviation_bps: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);
    if deviation_bps > 100_000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let key = DataKey::TriggerDeviationBps(asset.clone());
    env.storage().persistent().set(&key, &deviation_bps);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    TriggerConfigChangedEvent {
        asset,
        trigger_type: TRIGGER_TYPE_DEVIATION,
        value: deviation_bps as i128,
    }
    .publish(env);
}

pub fn get_deviation_trigger(env: &Env, asset: Address) -> u32 {
    let key = DataKey::TriggerDeviationBps(asset);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Internal bookkeeping
// ---------------------------------------------------------------------------

fn read_submission_count(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::TriggerSubmissionCount(asset.clone()))
        .unwrap_or(0)
}

fn write_submission_count(env: &Env, asset: &Address, count: u32) {
    let key = DataKey::TriggerSubmissionCount(asset.clone());
    env.storage().persistent().set(&key, &count);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_last_agg_time(env: &Env, asset: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::TriggerLastAggTime(asset.clone()))
        .unwrap_or(0)
}

fn write_last_agg_time(env: &Env, asset: &Address, now: u64) {
    let key = DataKey::TriggerLastAggTime(asset.clone());
    env.storage().persistent().set(&key, &now);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Called after every successful price submission to feed the threshold- and
/// deviation-based triggers. A no-op unless the admin has configured a
/// non-zero threshold or deviation for `asset`.
pub(crate) fn record_submission_for_triggers(env: &Env, asset: &Address, price: i128) {
    let threshold = get_submission_threshold_trigger(env, asset.clone());
    let deviation_bps = get_deviation_trigger(env, asset.clone());
    if threshold == 0 && deviation_bps == 0 {
        return;
    }

    let mut fired = false;
    let mut trigger_type = TRIGGER_TYPE_THRESHOLD;

    if deviation_bps > 0 {
        if let Some(agg) = env
            .storage()
            .persistent()
            .get::<_, AggregatePrice>(&DataKey::Aggregate(asset.clone()))
        {
            if agg.price > 0 {
                let diff = if price > agg.price {
                    price - agg.price
                } else {
                    agg.price - price
                };
                let change_bps = diff.saturating_mul(10_000) / agg.price;
                if change_bps >= deviation_bps as i128 {
                    fired = true;
                    trigger_type = TRIGGER_TYPE_DEVIATION;
                }
            }
        }
    }

    if !fired && threshold > 0 {
        let count = read_submission_count(env, asset).saturating_add(1);
        if count >= threshold {
            fired = true;
            trigger_type = TRIGGER_TYPE_THRESHOLD;
        } else {
            write_submission_count(env, asset, count);
        }
    }

    if fired {
        write_submission_count(env, asset, 0);
        write_last_agg_time(env, asset, env.ledger().timestamp());
        crate::prices::do_aggregate(env, asset);
        AutoTriggerFiredEvent {
            asset: asset.clone(),
            trigger_type,
        }
        .publish(env);
    }
}

/// Permissionless keeper endpoint: re-aggregates `asset` if at least the
/// configured time interval has elapsed since the last trigger-driven
/// aggregation. Returns `true` if aggregation ran.
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
pub fn poke_time_trigger(env: &Env, asset: Address) -> bool {
    check_registered_asset(env, &asset);
    let interval = get_time_trigger(env, asset.clone());
    if interval == 0 {
        return false;
    }

    let now = env.ledger().timestamp();
    let last = read_last_agg_time(env, &asset);
    if now.saturating_sub(last) < interval {
        return false;
    }

    write_last_agg_time(env, &asset, now);
    write_submission_count(env, &asset, 0);
    crate::prices::do_aggregate(env, &asset);
    AutoTriggerFiredEvent {
        asset,
        trigger_type: TRIGGER_TYPE_TIME,
    }
    .publish(env);
    true
}
