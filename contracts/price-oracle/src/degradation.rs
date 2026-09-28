//! #495 — Degraded-mode serving analytics
//!
//! Every path by which a consumer can receive a *degraded* value is
//! instrumented, and the counts are queryable per asset, per state and per
//! rolling window. Before this, "how many consumer reads last month were
//! degraded?" — the number that matters for trust and SLA — was unanswerable.
//!
//! ## States (mutually exclusive, layered by precedence)
//!
//! [`DegradationState`] is walked in a fixed order by [`classify`], and the
//! first state whose predicate holds is returned, so a read is attributed to
//! **exactly one** reason:
//!
//! | State | Predicate on the read path |
//! |---|---|
//! | `Stale` | the value's timestamp is older than the caller's `max_age`, or than the asset resolution |
//! | `Clamped` | the value came from a freeze or an admin override, not from the live median |
//! | `LowConfidence` | fewer than `min_sources_required` sources contributed |
//! | `Deferred` | the live aggregation path was unavailable (circuit breaker) and a TWAP / last-raw fallback was served |
//!
//! `Fresh` means no predicate held. Precedence is consumer-harm-first: a value
//! that is both stale and overridden is reported as `Stale`, the condition the
//! caller most needs to know about. Because each state is also counted
//! separately, the counts stay reconstructible from the events either way.
//!
//! ## Sampling never hides severe states
//!
//! High-frequency degradations (a stale read on every poll) would otherwise
//! flood the event stream, so `sample_every` emits one event per N counted
//! degradations. **Severe states — `Clamped` and `Deferred` — are never
//! sampled**: they are rare, and rare-and-severe is exactly what sampling must
//! not lose. Window counts are always exact, independently of sampling, and
//! each emitted event carries its own `window` so an off-chain aggregator can
//! reconstruct the per-window rate from the event stream alone.
//!
//! ## Read gas
//!
//! Instrumentation is a single counter read plus a single counter write on the
//! hot path, plus one event for a sampled state. `DegradationConfig::enabled`
//! turns counting off entirely, and `count_windows: false` reduces it to the
//! event only. `issues_495_496_497_498_tests::degradation_read_gas_overhead_is_bounded`
//! measures the overhead against an uninstrumented read.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::DegradedReadEvent;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    AggregatePrice, DataKey, DegradationConfig, DegradationState, DegradationStats, ErrorCode,
};

/// Default rolling window: 2016 ledgers ≈ 24 h at 5 s close times.
pub const DEFAULT_WINDOW_LEDGERS: u32 = 2016;
/// Default sampling rate: emit one event per 20 counted degradations. Severe
/// states bypass this entirely.
pub const DEFAULT_SAMPLE_EVERY: u32 = 20;
/// Number of window buckets retained per asset before the oldest is pruned, so
/// per-asset storage stays bounded no matter how long the contract runs.
pub const RETAINED_WINDOWS: u32 = 4;

fn default_config() -> DegradationConfig {
    DegradationConfig {
        enabled: true,
        window_ledgers: DEFAULT_WINDOW_LEDGERS,
        sample_every: DEFAULT_SAMPLE_EVERY,
        count_windows: true,
    }
}

/// Returns the current configuration, defaulting to instrumented-on.
pub fn get_config(env: &Env) -> DegradationConfig {
    env.storage()
        .persistent()
        .get(&DataKey::DegradationConfig)
        .unwrap_or_else(default_config)
}

/// Sets the instrumentation configuration. Admin-only.
///
/// `sample_every == 0` disables sampling (every degradation is emitted);
/// severe states remain unsampled either way. `window_ledgers == 0`
/// accumulates everything into a single unbounded window.
pub fn set_config(env: &Env, config: DegradationConfig) {
    get_admin(env).require_auth();
    if config.window_ledgers == 0 && config.count_windows {
        // Windowing is what bounds the counter set, so refusing to disable it
        // while counting avoids an unbounded-growth configuration.
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::DegradationConfig;
    env.storage().persistent().set(&key, &config);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Rolling-window index for the current ledger under `config`.
pub fn current_window(env: &Env, config: &DegradationConfig) -> u32 {
    if config.window_ledgers == 0 {
        0
    } else {
        env.ledger().sequence() / config.window_ledgers
    }
}

/// Classifies a served value into exactly one degradation state.
///
/// `frozen` / `override_` record that the value did not come from the live
/// median; `circuit_breaker` that the fallback path was used; `contributing`
/// is the number of sources behind the value. Precedence is
/// stale → clamped → low-confidence → deferred, and `Fresh` when none hold.
#[allow(clippy::too_many_arguments)]
pub fn classify(
    is_stale: bool,
    frozen: bool,
    override_: bool,
    contributing: u32,
    min_sources: u32,
    circuit_breaker: bool,
) -> DegradationState {
    if is_stale {
        return DegradationState::Stale;
    }
    if frozen || override_ {
        return DegradationState::Clamped;
    }
    if contributing < min_sources {
        return DegradationState::LowConfidence;
    }
    if circuit_breaker {
        return DegradationState::Deferred;
    }
    DegradationState::Fresh
}

/// Classifies a live aggregate against the asset's configured quorum.
pub fn classify_aggregate(aggregate: &AggregatePrice, min_sources: u32) -> DegradationState {
    classify(
        false,
        false,
        false,
        aggregate.num_sources,
        min_sources,
        false,
    )
}

fn counters_key(asset: &Address, window: u32) -> DataKey {
    DataKey::DegradationCounters(asset.clone(), window)
}

fn zeroed_states(env: &Env) -> Vec<u32> {
    let mut v = Vec::new(env);
    for _ in 0..DegradationState::ALL.len() {
        v.push_back(0);
    }
    v
}

fn read_counters(env: &Env, asset: &Address, window: u32) -> DegradationStats {
    env.storage()
        .persistent()
        .get(&counters_key(asset, window))
        .unwrap_or(DegradationStats {
            window,
            total_degraded: 0,
            by_state: zeroed_states(env),
            severe: 0,
            emitted_events: 0,
        })
}

/// Prunes the counter bucket that has just aged out.
///
/// Window indices are contiguous, so opening window `n` ages out exactly one
/// bucket — `n - RETAINED_WINDOWS` — and dropping just that key keeps the set
/// bounded at `RETAINED_WINDOWS + 1` buckets. Walking the whole range instead
/// would cost O(window) storage touches on the read path, which at ledger
/// 5 000 000 is thousands of entries and blows the network's 100-entry
/// footprint limit.
///
/// Only called when a *new* window opens, so the common case — many degraded
/// reads inside one window — costs nothing: an existing counter entry proves
/// the window was already pruned when it was created.
fn prune_windows(env: &Env, asset: &Address, window: u32) {
    if window <= RETAINED_WINDOWS {
        // Not enough history yet to have aged anything out.
        return;
    }
    let key = counters_key(asset, window - RETAINED_WINDOWS);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }
}

/// Records one consumer read of `asset` served in `state`.
///
/// Returns `true` when the read was counted, `false` when it was not degraded
/// or instrumentation is off. `Fresh` reads are never counted and never
/// emitted — these counters measure degradation, and a fresh read is not one.
pub fn record_read(env: &Env, asset: &Address, state: DegradationState) -> bool {
    if !state.is_degraded() {
        return false;
    }
    let config = get_config(env);
    if !config.enabled {
        return false;
    }

    let window = current_window(env, &config);
    let idx = state as u32;
    let severe = state.is_severe();

    if !config.count_windows {
        // Events-only mode: every degradation is emitted, since there are no
        // counts to sample against.
        if severe || config.sample_every <= 1 {
            DegradedReadEvent {
                asset: asset.clone(),
                state: idx,
                window,
                severe,
            }
            .publish(env);
        }
        return true;
    }

    // A missing counter entry means this window has just opened, so it is the
    // only moment pruning is needed. An existing entry was already pruned.
    let existing: Option<DegradationStats> =
        env.storage().persistent().get(&counters_key(asset, window));
    let is_new_window = existing.is_none();
    let mut stats = existing.unwrap_or(DegradationStats {
        window,
        total_degraded: 0,
        by_state: zeroed_states(env),
        severe: 0,
        emitted_events: 0,
    });
    stats
        .by_state
        .set(idx, stats.by_state.get(idx).unwrap_or(0) + 1);
    stats.total_degraded = stats.total_degraded.saturating_add(1);
    if severe {
        stats.severe = stats.severe.saturating_add(1);
    }

    // Sample on the cumulative count, folding in the previous window so the
    // sampler does not reset at a window boundary and swallow a rare event.
    // Severe states are never sampled.
    let emit = if severe || config.sample_every <= 1 {
        true
    } else {
        let mut cumulative = stats.total_degraded.saturating_sub(1);
        if window > 0 {
            let prev = read_counters(env, asset, window - 1);
            cumulative = cumulative.saturating_add(prev.total_degraded);
        }
        cumulative.is_multiple_of(config.sample_every)
    };
    if emit {
        stats.emitted_events = stats.emitted_events.saturating_add(1);
    }

    let key = counters_key(asset, window);
    env.storage().persistent().set(&key, &stats);
    if is_new_window {
        // The TTL is set when the window opens; re-extending it on every read
        // inside the window would be pure hot-path overhead.
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        prune_windows(env, asset, window);
    }

    if emit {
        DegradedReadEvent {
            asset: asset.clone(),
            state: idx,
            window,
            severe,
        }
        .publish(env);
    }
    true
}

/// Degradation counts for `asset` over its current rolling window.
pub fn get_stats(env: &Env, asset: &Address) -> DegradationStats {
    let config = get_config(env);
    let window = current_window(env, &config);
    read_counters(env, asset, window)
}

/// Counts of an arbitrary (possibly older) window, for dashboards that render
/// more than one bucket. Unwritten windows read as all-zero.
pub fn get_window_stats(env: &Env, asset: &Address, window: u32) -> DegradationStats {
    read_counters(env, asset, window)
}

/// Sum of the per-state counts, excluding `Fresh`. Equal to
/// `stats.total_degraded` because the states partition the degraded reads —
/// the invariant behind "exactly one reason per read".
pub fn total_from_states(stats: &DegradationStats) -> u32 {
    let mut total = 0u32;
    for i in 1..stats.by_state.len() {
        total = total.saturating_add(stats.by_state.get(i).unwrap_or(0));
    }
    total
}
