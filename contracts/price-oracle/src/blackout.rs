//! #400 — Price blackout / quiet-period mechanism
//!
//! A blackout suspends *aggregation* for one asset: submissions are still
//! accepted and stored, but no new aggregate is published while the window is
//! active, and the previously published aggregate stays readable with its
//! original timestamp so consumers can see it ageing.
//!
//! ## Authority
//!
//! * **Scheduled** windows are created and extended by the admin only, with
//!   at least [`MIN_NOTICE_SECS`] of notice — a blackout cannot be opened
//!   reactively to hide a manipulation that is already under way.
//! * **Volatility** windows open only when [`get_volatility_quorum`] distinct
//!   registered sources signal within [`SIGNAL_WINDOW_SECS`]. A single source
//!   can neither open nor extend a window.
//!
//! ## Griefing bound
//!
//! Every window lasts at most [`MAX_BLACKOUT_SECS`] including extensions, and
//! a new window may only start [`COOLDOWN_SECS`] after the previous one ended.
//! Continuous blackout is therefore impossible: the asset aggregates for at
//! least `COOLDOWN_SECS` out of every `MAX_BLACKOUT_SECS + COOLDOWN_SECS`. The
//! escalation path beyond that is the existing pause / freeze machinery.
//!
//! ## Boundaries and precedence
//!
//! A window is the half-open interval `[start, end)` of ledger time.
//! Aggregation triggered at `start` is withheld; aggregation triggered at
//! `end` publishes. Submissions stored during the window are not discarded:
//! the first aggregation at or after `end` uses them, subject to the normal
//! freshness and DQ checks. Precedence is `pause > freeze > blackout`: a paused
//! contract rejects submissions outright, a frozen asset serves its frozen
//! snapshot, and only otherwise does a blackout withhold aggregation.

use soroban_sdk::{contractevent, contracttype, panic_with_error, Address, Env, Vec};

use crate::storage::{
    check_registered_asset, check_source, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::ErrorCode;

/// Longest a window may last, extensions included (6 h).
pub const MAX_BLACKOUT_SECS: u64 = 6 * 3_600;
/// Minimum notice for an admin-scheduled window (1 h).
pub const MIN_NOTICE_SECS: u64 = 3_600;
/// Minimum gap between the end of one window and the start of the next (6 h).
pub const COOLDOWN_SECS: u64 = 6 * 3_600;
/// Window in which volatility signals are counted towards quorum (5 min).
pub const SIGNAL_WINDOW_SECS: u64 = 300;
/// Duration of a volatility-triggered window (30 min).
pub const VOLATILITY_BLACKOUT_SECS: u64 = 1_800;
/// Default number of distinct sources required to trigger a window.
pub const DEFAULT_VOLATILITY_QUORUM: u32 = 3;

pub const ORIGIN_SCHEDULED: u32 = 1;
pub const ORIGIN_VOLATILITY: u32 = 2;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BlackoutWindow {
    pub start: u64,
    pub end: u64,
    /// [`ORIGIN_SCHEDULED`] or [`ORIGIN_VOLATILITY`].
    pub origin: u32,
}

#[contracttype]
#[derive(Clone)]
struct VolatilitySignals {
    since: u64,
    sources: Vec<Address>,
}

#[contracttype]
#[derive(Clone)]
enum BlackoutKey {
    Window(Address),
    LastEnd(Address),
    Signals(Address),
    Quorum,
}

#[contractevent]
#[derive(Clone)]
pub struct BlackoutScheduledEvent {
    #[topic]
    pub asset: Address,
    pub start: u64,
    pub end: u64,
    pub origin: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct BlackoutExtendedEvent {
    #[topic]
    pub asset: Address,
    pub start: u64,
    pub old_end: u64,
    pub new_end: u64,
}

/// Emitted when a window ends, either by expiry (observed on the first
/// aggregation at or after `end`) or by admin cancellation.
#[contractevent]
#[derive(Clone)]
pub struct BlackoutExitedEvent {
    #[topic]
    pub asset: Address,
    pub start: u64,
    pub end: u64,
    pub cancelled: bool,
}

/// Emitted each time an aggregation (including a correction of the previous
/// value) is withheld because a window is active.
#[contractevent]
#[derive(Clone)]
pub struct BlackoutWithheldEvent {
    #[topic]
    pub asset: Address,
    pub at: u64,
    pub window_end: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct VolatilitySignalEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub signals: u32,
    pub quorum: u32,
}

pub fn get_window(env: &Env, asset: &Address) -> Option<BlackoutWindow> {
    env.storage()
        .persistent()
        .get(&BlackoutKey::Window(asset.clone()))
}

pub fn get_volatility_quorum(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&BlackoutKey::Quorum)
        .unwrap_or(DEFAULT_VOLATILITY_QUORUM)
}

/// Sets the volatility quorum. Admin-only; must be at least 2 so no single
/// source can trigger a window.
pub fn set_volatility_quorum(env: &Env, quorum: u32) {
    get_admin(env).require_auth();
    if quorum < 2 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&BlackoutKey::Quorum, &quorum);
}

fn store_window(env: &Env, asset: &Address, w: &BlackoutWindow) {
    let key = BlackoutKey::Window(asset.clone());
    env.storage().persistent().set(&key, w);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Rejects a new window starting at `start` when one is still pending/active or
/// the cooldown since the previous window has not elapsed.
fn check_can_open(env: &Env, asset: &Address, start: u64) {
    let now = env.ledger().timestamp();
    if let Some(w) = get_window(env, asset) {
        if now < w.end {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
        close(env, asset, &w, false);
    }
    let last_end: Option<u64> = env
        .storage()
        .persistent()
        .get(&BlackoutKey::LastEnd(asset.clone()));
    if let Some(last_end) = last_end {
        if start < last_end.saturating_add(COOLDOWN_SECS) {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
    }
}

fn open(env: &Env, asset: &Address, w: BlackoutWindow) {
    store_window(env, asset, &w);
    BlackoutScheduledEvent {
        asset: asset.clone(),
        start: w.start,
        end: w.end,
        origin: w.origin,
    }
    .publish(env);
}

fn close(env: &Env, asset: &Address, w: &BlackoutWindow, cancelled: bool) {
    env.storage()
        .persistent()
        .remove(&BlackoutKey::Window(asset.clone()));
    let end = if cancelled {
        env.ledger().timestamp().min(w.end)
    } else {
        w.end
    };
    env.storage()
        .persistent()
        .set(&BlackoutKey::LastEnd(asset.clone()), &end);
    BlackoutExitedEvent {
        asset: asset.clone(),
        start: w.start,
        end,
        cancelled,
    }
    .publish(env);
}

/// Schedules a blackout window `[start, end)`. Admin-only.
pub fn schedule(env: &Env, asset: Address, start: u64, end: u64) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    let now = env.ledger().timestamp();
    if start < now.saturating_add(MIN_NOTICE_SECS)
        || end <= start
        || end - start > MAX_BLACKOUT_SECS
    {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    check_can_open(env, &asset, start);
    open(
        env,
        &asset,
        BlackoutWindow {
            start,
            end,
            origin: ORIGIN_SCHEDULED,
        },
    );
}

/// Extends a pending or active window. Admin-only; the total duration stays
/// capped at [`MAX_BLACKOUT_SECS`].
pub fn extend(env: &Env, asset: Address, new_end: u64) {
    get_admin(env).require_auth();
    let mut w = get_window(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::InvalidConfiguration));
    if env.ledger().timestamp() >= w.end
        || new_end <= w.end
        || new_end - w.start > MAX_BLACKOUT_SECS
    {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let old_end = w.end;
    w.end = new_end;
    store_window(env, &asset, &w);
    BlackoutExtendedEvent {
        asset,
        start: w.start,
        old_end,
        new_end,
    }
    .publish(env);
}

/// Cancels a pending or active window immediately. Admin-only.
pub fn cancel(env: &Env, asset: Address) {
    get_admin(env).require_auth();
    let w = get_window(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::InvalidConfiguration));
    close(env, &asset, &w, true);
}

/// Records a volatility signal from a registered source. Opens a
/// [`VOLATILITY_BLACKOUT_SECS`] window once quorum distinct sources signal
/// within [`SIGNAL_WINDOW_SECS`]. Returns `true` when this call opened one.
pub fn signal_volatility(env: &Env, source: Address, asset: Address) -> bool {
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);
    if crate::source_lifecycle::on_probation(env, &source) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
    let now = env.ledger().timestamp();
    let key = BlackoutKey::Signals(asset.clone());
    let mut signals: VolatilitySignals = env
        .storage()
        .persistent()
        .get(&key)
        .filter(|s: &VolatilitySignals| now.saturating_sub(s.since) < SIGNAL_WINDOW_SECS)
        .unwrap_or(VolatilitySignals {
            since: now,
            sources: Vec::new(env),
        });
    if !signals.sources.contains(&source) {
        signals.sources.push_back(source.clone());
    }
    let quorum = get_volatility_quorum(env);
    let count = signals.sources.len();
    VolatilitySignalEvent {
        asset: asset.clone(),
        source,
        signals: count,
        quorum,
    }
    .publish(env);

    if count < quorum {
        env.storage().persistent().set(&key, &signals);
        return false;
    }
    env.storage().persistent().remove(&key);
    check_can_open(env, &asset, now);
    open(
        env,
        &asset,
        BlackoutWindow {
            start: now,
            end: now + VOLATILITY_BLACKOUT_SECS,
            origin: ORIGIN_VOLATILITY,
        },
    );
    true
}

/// Aggregation hook: `true` when aggregation for `asset` must be withheld now.
/// Closes an expired window (emitting its exit event) as a side effect.
pub fn suppresses(env: &Env, asset: &Address) -> bool {
    let Some(w) = get_window(env, asset) else {
        return false;
    };
    let now = env.ledger().timestamp();
    if now < w.start {
        return false;
    }
    if now >= w.end {
        close(env, asset, &w, false);
        return false;
    }
    BlackoutWithheldEvent {
        asset: asset.clone(),
        at: now,
        window_end: w.end,
    }
    .publish(env);
    true
}
