//! # Freshness-aware quorum (#489)
//!
//! Quorum computed over *all* historical submissions can be satisfied by stale
//! data: two sources reported an hour ago and a third reported now, and a naive
//! count says "3 sources, quorum met" when in truth only one source is
//! currently participating. This module makes the exclusion explicit.
//!
//! ## What changes
//!
//! Only submissions **inside the asset's freshness window** count toward
//! quorum. A submission that exists but has aged out is counted as *stale* and
//! excluded — never silently served, never quietly counted.
//!
//! ## Window resolution
//!
//! ```text
//! per-asset window  >  tier window (#487)  >  global default
//! ```
//!
//! `overridden` on the returned [`FreshnessStatus`] reports which of those
//! supplied the value, so a consumer can tell a long-window illiquid asset
//! apart from a globally-configured one.
//!
//! ## Measured against ledger time
//!
//! Freshness is always `env.ledger().timestamp() - entry.ledger_timestamp`,
//! never wall-clock and never a source-supplied value. A source cannot make its
//! own submission fresh by reporting a later timestamp.
//!
//! ## Fail closed
//!
//! When the fresh count is below quorum the asset is explicitly
//! [`FreshnessState::Stale`] — visible through
//! `get_freshness_status` and through the `FreshnessFilteredEvent` emitted on
//! every aggregation — rather than being served as if it were current.
//!
//! See `docs/freshness-quorum.md`.

use soroban_sdk::{contractevent, panic_with_error, Address, Env};

use crate::price_bounds::note_freshness_window;
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    DataKey, ErrorCode, FreshnessState, FreshnessStatus, PolicyOverride, PriceEntry,
};

/// Largest accepted freshness window (1 week).
pub const MAX_FRESHNESS_WINDOW_SECS: u64 = 604_800;

/// Emitted on every aggregation pass that applied a freshness window (#489).
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct FreshnessFilteredEvent {
    #[topic]
    pub asset: Address,
    /// Submissions inside the window; the only ones counted toward quorum.
    pub fresh: u32,
    /// Submissions excluded for being older than the window.
    pub stale: u32,
    /// Quorum the fresh count was compared against.
    pub quorum: u32,
    /// The window that was applied.
    pub window_secs: u64,
    /// `true` when the window came from the asset or its tier, not the default.
    pub overridden: bool,
    /// `true` when the fresh count satisfied quorum.
    pub quorum_met: bool,
    /// Ledger time the ages were measured against.
    pub measured_at: u64,
    /// Ledger of the aggregation pass.
    pub ledger: u32,
}

/// The window that applies to `asset`, and where it came from.
///
/// Returns `(window_secs, overridden)`. `window_secs == 0` means no window is
/// configured anywhere, in which case every submission counts.
pub fn window_for(env: &Env, asset: &Address) -> (u64, bool) {
    let key = DataKey::AssetFreshnessWindow(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        let v: u64 = env.storage().persistent().get(&key).unwrap_or(0);
        return (v, true);
    }
    // Tier window (#487) is the second precedence step.
    let no_overrides = PolicyOverride {
        method: None,
        min_sources: None,
        freshness_secs: None,
        max_deviation_bps: None,
    };
    if let Some(resolved) = crate::risk_tier::resolve(env, asset, &no_overrides) {
        if resolved.effective.freshness_secs > 0 {
            return (resolved.effective.freshness_secs, true);
        }
    }
    (default_window(env), false)
}

/// The global default window, `0` when unset.
pub fn default_window(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::DefaultFreshnessWindow)
        .unwrap_or(0)
}

/// Sets the global default freshness window. Admin only; `0` clears it.
pub fn set_default_window(env: &Env, secs: u64) {
    get_admin(env).require_auth();
    validate(env, secs);
    let key = DataKey::DefaultFreshnessWindow;
    if secs == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &secs);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    note_freshness_window(env);
}

/// Sets (or, with `0`, clears) an asset's own freshness window. Admin only.
///
/// This is the documented per-asset override: an asset whose natural update
/// interval is longer than the global default widens its window here rather
/// than forcing the global value on every asset.
pub fn set_asset_window(env: &Env, asset: Address, secs: u64) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    validate(env, secs);
    let key = DataKey::AssetFreshnessWindow(asset.clone());
    if secs == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &secs);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    note_freshness_window(env);
}

fn validate(env: &Env, secs: u64) {
    if secs > MAX_FRESHNESS_WINDOW_SECS {
        panic_with_error!(env, ErrorCode::InvalidFreshnessWindow);
    }
}

/// `true` when `entry` is inside the window, measured against ledger time.
pub fn is_fresh(env: &Env, entry: &PriceEntry, window_secs: u64) -> bool {
    if window_secs == 0 {
        return true;
    }
    env.ledger()
        .timestamp()
        .saturating_sub(entry.ledger_timestamp)
        <= window_secs
}

/// Records the round's fresh/stale split, emitting the filter outcome.
///
/// `fresh` submissions are the only ones counted toward quorum; `stale` ones
/// exist but are excluded. When `fresh < quorum` the asset is recorded as
/// explicitly [`FreshnessState::Stale`] rather than served as current, which is
/// the fail-closed requirement.
pub fn record_round(env: &Env, asset: &Address, fresh: u32, stale: u32, quorum: u32) {
    let (window, overridden) = window_for(env, asset);
    let now = env.ledger().timestamp();
    let state = if fresh == 0 && stale == 0 {
        FreshnessState::Absent
    } else if fresh >= quorum {
        FreshnessState::Fresh
    } else {
        FreshnessState::Stale
    };
    FreshnessFilteredEvent {
        asset: asset.clone(),
        fresh,
        stale,
        quorum,
        window_secs: window,
        overridden,
        quorum_met: state == FreshnessState::Fresh,
        measured_at: now,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
    write_status(
        env,
        asset,
        &FreshnessStatus {
            asset: asset.clone(),
            fresh,
            stale,
            quorum,
            window_secs: window,
            overridden,
            state,
            measured_at: now,
            ledger: env.ledger().sequence(),
        },
    );
}

fn write_status(env: &Env, asset: &Address, status: &FreshnessStatus) {
    let key = DataKey::AssetFreshnessStatus(asset.clone());
    env.storage().persistent().set(&key, status);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// The freshness-filter outcome of the most recent aggregation of `asset`.
///
/// `None` until the asset has been through a pass with a window configured,
/// which is the signal that the asset is being freshness-gated at all.
pub fn get_status(env: &Env, asset: &Address) -> Option<FreshnessStatus> {
    let key = DataKey::AssetFreshnessStatus(asset.clone());
    let v: Option<FreshnessStatus> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}
