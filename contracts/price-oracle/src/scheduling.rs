//! # Price Submission Scheduling (Issue #290)
//!
//! Allows each (source, asset) pair to register a **submission schedule** —
//! a target interval expressed in ledgers or seconds.  The contract
//! automatically checks liveness on each `submit_price` call and flags
//! sources that are behind schedule.
//!
//! ## Schedule types
//!
//! | Variant | Meaning |
//! |---------|---------|
//! | `EveryNLedgers(n)` | Submit at most every `n` ledgers |
//! | `EveryNSeconds(n)` | Submit at most every `n` seconds |
//!
//! ## Enforcement
//!
//! On each `submit_price` the contract:
//! 1. Records the current ledger/timestamp as the last-seen values.
//! 2. Computes the gap since the *previous* submission.
//! 3. If the gap exceeds `deadline_multiplier × interval`, the source is
//!    considered **late** and a schedule violation event is emitted.
//!
//! ## Storage layout
//!
//! | Key | Type | Description |
//! |-----|------|-------------|
//! | `SubmissionSchedule(source, asset)` | `SubmissionSchedule` | Per-(source,asset) schedule |
//! | `LastScheduledSubmission(source, asset)` | `LastSubmissionRecord` | When the last submission occurred |

use soroban_sdk::{contracttype, panic_with_error, Address, Env};

use crate::events::{emit_schedule_registered, emit_schedule_violation, ScheduleRemovedEvent};
use crate::storage::{
    check_registered_asset, check_source, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{DataKey, ErrorCode};

// ─── Data structures ────────────────────────────────────────────────────────

/// The two supported schedule modes.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleKind {
    /// Submit approximately every `n` ledgers.
    EveryNLedgers = 0,
    /// Submit approximately every `n` seconds.
    EveryNSeconds = 1,
}

/// A registered submission schedule for a (source, asset) pair.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SubmissionSchedule {
    /// Address of the oracle source.
    pub source: Address,
    /// Address of the asset.
    pub asset: Address,
    /// Schedule mode.
    pub kind: ScheduleKind,
    /// Target interval (ledgers or seconds depending on `kind`).
    pub interval: u64,
    /// How many intervals the source may be late before a violation is emitted.
    /// For example `2` means "flag if gap > 2 × interval".
    pub deadline_multiplier: u32,
}

/// Lightweight record of the most-recent price submission for liveness tracking.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LastSubmissionRecord {
    /// Ledger sequence number of the last submission.
    pub ledger: u32,
    /// Unix timestamp of the last submission.
    pub timestamp: u64,
}

// ─── Storage helpers ────────────────────────────────────────────────────────

fn schedule_key(source: &Address, asset: &Address) -> DataKey {
    DataKey::SubmissionSchedule(source.clone(), asset.clone())
}

fn last_sub_key(source: &Address, asset: &Address) -> DataKey {
    DataKey::LastScheduledSubmission(source.clone(), asset.clone())
}

fn write_schedule(env: &Env, schedule: &SubmissionSchedule) {
    let key = schedule_key(&schedule.source, &schedule.asset);
    env.storage().persistent().set(&key, schedule);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_schedule(env: &Env, source: &Address, asset: &Address) -> Option<SubmissionSchedule> {
    let key = schedule_key(source, asset);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

fn write_last_submission(
    env: &Env,
    source: &Address,
    asset: &Address,
    record: &LastSubmissionRecord,
) {
    let key = last_sub_key(source, asset);
    env.storage().persistent().set(&key, record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_last_submission(
    env: &Env,
    source: &Address,
    asset: &Address,
) -> Option<LastSubmissionRecord> {
    let key = last_sub_key(source, asset);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

// ─── Public API ─────────────────────────────────────────────────────────────

/// Registers (or overwrites) a submission schedule for a (source, asset) pair.
///
/// The source must authorize this call.
///
/// # Arguments
///
/// * `env` — The Soroban execution environment.
/// * `source` — The oracle source address.
/// * `asset` — The asset being priced.
/// * `kind` — Schedule kind (`0` = every-N-ledgers, `1` = every-N-seconds).
/// * `interval` — Target interval (must be ≥ 1).
/// * `deadline_multiplier` — Allowed lateness factor (must be ≥ 1).
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the source is not registered.
/// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
/// * [`ErrorCode::InvalidConfiguration`] — if `interval` or `deadline_multiplier` is `0`.
pub fn register_schedule(
    env: &Env,
    source: Address,
    asset: Address,
    kind: u32,
    interval: u64,
    deadline_multiplier: u32,
) {
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);

    if interval == 0 || deadline_multiplier == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let schedule_kind = match kind {
        0 => ScheduleKind::EveryNLedgers,
        1 => ScheduleKind::EveryNSeconds,
        _ => panic_with_error!(env, ErrorCode::InvalidConfiguration),
    };

    let schedule = SubmissionSchedule {
        source: source.clone(),
        asset: asset.clone(),
        kind: schedule_kind,
        interval,
        deadline_multiplier,
    };
    write_schedule(env, &schedule);

    emit_schedule_registered(
        env,
        source.clone(),
        asset.clone(),
        kind as u64,
        interval,
        deadline_multiplier,
    );
}

/// Removes the submission schedule for a (source, asset) pair.
///
/// The source must authorize this call.
///
/// # Errors
///
/// * [`ErrorCode::NoData`] — if no schedule exists.
pub fn remove_schedule(env: &Env, source: Address, asset: Address) {
    source.require_auth();

    let key = schedule_key(&source, &asset);
    if !env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::NoData);
    }
    env.storage().persistent().remove(&key);

    // Also clean up last-submission record
    let lsk = last_sub_key(&source, &asset);
    if env.storage().persistent().has(&lsk) {
        env.storage().persistent().remove(&lsk);
    }

    ScheduleRemovedEvent {
        source: source.clone(),
        asset: asset.clone(),
    }
    .publish(env);
}

/// Admin variant of schedule removal — the admin can force-remove any schedule.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
/// * [`ErrorCode::NoData`] — if no schedule exists.
pub fn admin_remove_schedule(env: &Env, source: Address, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = schedule_key(&source, &asset);
    if !env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::NoData);
    }
    env.storage().persistent().remove(&key);

    let lsk = last_sub_key(&source, &asset);
    if env.storage().persistent().has(&lsk) {
        env.storage().persistent().remove(&lsk);
    }

    ScheduleRemovedEvent {
        source: source.clone(),
        asset: asset.clone(),
    }
    .publish(env);
}

/// Returns the submission schedule for a (source, asset) pair, or `None`.
pub fn get_schedule(env: &Env, source: Address, asset: Address) -> Option<SubmissionSchedule> {
    read_schedule(env, &source, &asset)
}

/// Records a price submission event for scheduling purposes and checks whether the
/// source is overdue.
///
/// Called automatically from `prices::submit_price`.
///
/// # Returns
///
/// `true` if the submission is on time (or no schedule is registered); `false` if
/// it is a schedule violation (a schedule violation event is also emitted).
pub fn record_submission(env: &Env, source: &Address, asset: &Address) -> bool {
    let current_ledger = env.ledger().sequence();
    let current_timestamp = env.ledger().timestamp();

    let schedule = match read_schedule(env, source, asset) {
        None => {
            // No schedule registered — just record the submission time
            write_last_submission(
                env,
                source,
                asset,
                &LastSubmissionRecord {
                    ledger: current_ledger,
                    timestamp: current_timestamp,
                },
            );
            return true;
        }
        Some(s) => s,
    };

    let previous = read_last_submission(env, source, asset);

    // Compute on-time status and gap before we move `previous`
    let (on_time, expected_interval, actual_gap) = match &previous {
        None => (true, schedule.interval, 0u64), // first-ever submission, cannot be late
        Some(prev) => {
            let deadline_interval = schedule
                .interval
                .saturating_mul(schedule.deadline_multiplier as u64);

            match schedule.kind {
                ScheduleKind::EveryNLedgers => {
                    let gap = (current_ledger as u64).saturating_sub(prev.ledger as u64);
                    (gap <= deadline_interval, schedule.interval, gap)
                }
                ScheduleKind::EveryNSeconds => {
                    let gap = current_timestamp.saturating_sub(prev.timestamp);
                    (gap <= deadline_interval, schedule.interval, gap)
                }
            }
        }
    };

    // Update last-submission record
    write_last_submission(
        env,
        source,
        asset,
        &LastSubmissionRecord {
            ledger: current_ledger,
            timestamp: current_timestamp,
        },
    );

    if !on_time {
        let kind_code = match schedule.kind {
            ScheduleKind::EveryNLedgers => 0u32,
            ScheduleKind::EveryNSeconds => 1u32,
        };
        emit_schedule_violation(
            env,
            source.clone(),
            asset.clone(),
            expected_interval,
            actual_gap,
            kind_code,
        );
    }

    on_time
}

/// Explicit liveness check — can be called by anyone to verify whether a source
/// is currently on schedule for a given asset.
///
/// Emits a schedule violation event if the source is overdue.
///
/// # Returns
///
/// `true` if the source is on schedule (or no schedule registered); `false` if overdue.
pub fn check_liveness(env: &Env, source: Address, asset: Address) -> bool {
    let schedule = match read_schedule(env, &source, &asset) {
        None => return true, // no schedule = always "live"
        Some(s) => s,
    };

    let previous = match read_last_submission(env, &source, &asset) {
        None => return true, // never submitted under this schedule
        Some(p) => p,
    };

    let current_ledger = env.ledger().sequence();
    let current_timestamp = env.ledger().timestamp();
    let deadline_interval = schedule
        .interval
        .saturating_mul(schedule.deadline_multiplier as u64);

    let (on_time, expected_interval, actual_gap) = match schedule.kind {
        ScheduleKind::EveryNLedgers => {
            let gap = (current_ledger as u64).saturating_sub(previous.ledger as u64);
            (gap <= deadline_interval, schedule.interval, gap)
        }
        ScheduleKind::EveryNSeconds => {
            let gap = current_timestamp.saturating_sub(previous.timestamp);
            (gap <= deadline_interval, schedule.interval, gap)
        }
    };

    if !on_time {
        let kind_code = match schedule.kind {
            ScheduleKind::EveryNLedgers => 0u32,
            ScheduleKind::EveryNSeconds => 1u32,
        };
        emit_schedule_violation(
            env,
            source.clone(),
            asset.clone(),
            expected_interval,
            actual_gap,
            kind_code,
        );
    }

    on_time
}

/// Returns the last-submission record for a (source, asset) pair, or `None`.
pub fn get_last_submission(
    env: &Env,
    source: Address,
    asset: Address,
) -> Option<LastSubmissionRecord> {
    read_last_submission(env, &source, &asset)
}
