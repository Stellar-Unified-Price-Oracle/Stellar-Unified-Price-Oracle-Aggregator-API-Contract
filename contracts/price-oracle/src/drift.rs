//! #497 — Oracle-vs-benchmark long-horizon drift detection
//!
//! A slow, small bias evades every per-ledger deviation check while
//! systematically mispricing consumers. This module compares the published
//! aggregate against an independent external benchmark over a long rolling
//! window and reports a *sustained directional* bias when one exists.
//!
//! ## Time alignment
//!
//! Oracle and benchmark snapshots are taken at different instants. Comparing an
//! aggregate stamped at `t0` against a benchmark stamped at `t1` measures the
//! price move between the two, not the oracle's error. Samples are therefore
//! only admitted when
//!
//! ```text
//! |benchmark_timestamp - aggregate_timestamp| <= max_alignment_secs
//! ```
//!
//! and are otherwise counted in `misaligned_skipped` and dropped. This is a
//! deliberate trade-off: a tight alignment window discards real samples during
//! fast markets, a loose one lets snapshot skew masquerade as bias. The default
//! of 300 s matches the default `resolution`; see `docs/drift-detection.md`.
//!
//! ## Sustained drift vs transient divergence
//!
//! A single large divergence — a benchmark print on a thin book, a flash move —
//! is not drift. Drift is *directional and repeated*, so the report carries
//! `directional_consistency_bps`: the share of in-window samples whose sign
//! matches the mean bias. Sustained drift requires **all three** of:
//!
//! 1. at least `min_samples` aligned samples,
//! 2. `|mean_bias_bps| >= bias_threshold_bps`, and
//! 3. `directional_consistency_bps >= 7000` — at least 70 % of samples
//!    individually breach the threshold *on the same side* as the mean.
//!
//! A transient divergence fails (3): a single 100 % spike among on-benchmark
//! samples scores ~8 %, not ~92 %, because only one sample individually
//! breaches the threshold. A whipsaw fails it too, since its large samples
//! sit on opposite sides.
//!
//! ## Divergence is a signal, not proof
//!
//! The benchmark can be wrong or manipulated. This module publishes metrics to
//! operators and **never** mutates an on-chain price, never changes a source
//! set, and never triggers remediation — an automated response to a possibly
//! wrong benchmark is how a benchmark attack becomes a price attack.

use soroban_sdk::{contracttype, panic_with_error, Address, Env, Vec};

use crate::events::DriftSampleRecordedEvent;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, DriftReport, ErrorCode};

/// Default sustained-bias threshold, in bps (1 %).
pub const DEFAULT_BIAS_THRESHOLD_BPS: i128 = 100;
/// Default minimum aligned samples before drift may be declared.
pub const DEFAULT_MIN_SAMPLES: u32 = 8;
/// Default snapshot-alignment tolerance, in seconds.
pub const DEFAULT_MAX_ALIGNMENT_SECS: u64 = 300;
/// Share of samples that must agree in sign before drift is declared (70 %).
pub const DIRECTIONAL_CONSISTENCY_FLOOR_BPS: u32 = 7_000;
/// Rolling window capacity, in samples. Bounded so the storage footprint per
/// asset is fixed regardless of how long the contract runs.
pub const WINDOW_CAPACITY: u32 = 64;

/// Drift alert thresholds, stored under [`DataKey::DriftThresholds`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub struct DriftThresholds {
    /// Minimum aligned samples before drift may be declared.
    pub min_samples: u32,
    /// Absolute mean-bias threshold, in bps.
    pub bias_threshold_bps: i128,
    /// Maximum tolerated gap between the oracle and benchmark snapshot
    /// timestamps, in seconds.
    pub max_alignment_secs: u64,
}

fn default_thresholds() -> DriftThresholds {
    DriftThresholds {
        min_samples: DEFAULT_MIN_SAMPLES,
        bias_threshold_bps: DEFAULT_BIAS_THRESHOLD_BPS,
        max_alignment_secs: DEFAULT_MAX_ALIGNMENT_SECS,
    }
}

/// Current drift thresholds.
pub fn get_thresholds(env: &Env) -> DriftThresholds {
    env.storage()
        .persistent()
        .get(&DataKey::DriftThresholds)
        .unwrap_or_else(default_thresholds)
}

/// Sets the drift thresholds. Admin-only.
pub fn set_thresholds(env: &Env, thresholds: DriftThresholds) {
    get_admin(env).require_auth();
    if thresholds.min_samples == 0 || thresholds.min_samples > WINDOW_CAPACITY {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    if thresholds.bias_threshold_bps <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::DriftThresholds;
    env.storage().persistent().set(&key, &thresholds);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Signed bias of the oracle against the benchmark, in bps.
///
/// `oracle / benchmark - 1`, scaled by 10 000. Returns `None` for a
/// non-positive benchmark, which cannot produce a meaningful ratio.
pub fn bias_bps(oracle: i128, benchmark: i128) -> Option<i128> {
    if benchmark <= 0 {
        return None;
    }
    // Guard against an overflowing product: a benchmark that small makes the
    // ratio astronomically large, which is a benchmark fault, not a bias.
    let scaled = oracle.checked_mul(10_000)?;
    let diff = scaled - benchmark.saturating_mul(10_000);
    Some(diff / benchmark)
}

/// Aligned samples currently in the rolling window for `asset`.
pub fn get_window(env: &Env, asset: &Address) -> Vec<i128> {
    env.storage()
        .persistent()
        .get(&DataKey::DriftWindow(asset.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// Pushes `bias` onto the window, dropping the oldest sample when full.
fn push_sample(env: &Env, asset: &Address, bias: i128) {
    let key = DataKey::DriftWindow(asset.clone());
    let mut window = get_window(env, asset);
    window.push_back(bias);
    while window.len() > WINDOW_CAPACITY {
        window.remove(0);
    }
    env.storage().persistent().set(&key, &window);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn bump_misaligned(env: &Env, asset: &Address) {
    let key = DataKey::DriftMisaligned(asset.clone());
    let n: u32 = env.storage().persistent().get(&key).unwrap_or(0);
    env.storage().persistent().set(&key, &n.saturating_add(1));
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Samples rejected for snapshot skew since the window was created.
pub fn get_misaligned(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::DriftMisaligned(asset.clone()))
        .unwrap_or(0)
}

/// Records one oracle-vs-benchmark comparison for `asset`.
///
/// `oracle_timestamp` is the aggregate's own timestamp and `benchmark_timestamp`
/// the benchmark snapshot's; a gap wider than `max_alignment_secs` is counted
/// as misaligned and discarded rather than folded into the bias.
///
/// Read-only with respect to prices: the aggregate is never written here.
pub fn record_sample(
    env: &Env,
    asset: &Address,
    oracle_price: i128,
    oracle_timestamp: u64,
    benchmark_price: i128,
    benchmark_timestamp: u64,
) -> Option<i128> {
    let thresholds = get_thresholds(env);
    let skew = oracle_timestamp.abs_diff(benchmark_timestamp);
    if skew > thresholds.max_alignment_secs {
        // Too far apart in time for the difference to be attributable to the
        // oracle; counting it would turn snapshot skew into apparent bias.
        bump_misaligned(env, asset);
        DriftSampleRecordedEvent {
            asset: asset.clone(),
            bias_bps: 0,
            aligned: false,
        }
        .publish(env);
        return None;
    }

    let bias = bias_bps(oracle_price, benchmark_price)?;
    push_sample(env, asset, bias);
    DriftSampleRecordedEvent {
        asset: asset.clone(),
        bias_bps: bias,
        aligned: true,
    }
    .publish(env);
    Some(bias)
}

/// Directional consistency of `window`, in bps (0–10 000).
///
/// The share of samples that **individually** breach `threshold_bps` *and* sit
/// on the same side as the mean bias. Both halves matter:
///
/// * requiring each sample to breach the threshold is what separates sustained
///   drift from a single large transient divergence — one 100 % spike among
///   twelve on-benchmark samples would score 8 % on this metric, not 92 %;
/// * requiring the same sign is what separates drift from a whipsaw, where
///   every sample is large but they cancel out.
pub fn directional_consistency(
    window: &Vec<i128>,
    mean_bias_bps: i128,
    threshold_bps: i128,
) -> u32 {
    let n = window.len();
    if n == 0 {
        return 0;
    }
    let mut agreeing: u64 = 0;
    for i in 0..n {
        let b = window.get_unchecked(i);
        let abs = if b < 0 { -b } else { b };
        if abs < threshold_bps {
            continue;
        }
        if (b > 0 && mean_bias_bps > 0) || (b < 0 && mean_bias_bps < 0) {
            agreeing += 1;
        }
    }
    ((agreeing * 10_000) / n as u64) as u32
}

/// Builds the drift report for `asset` from the samples currently in window.
///
/// Pure with respect to on-chain state: it writes nothing, so the same inputs
/// always yield the same report and no price can be affected by reading it.
pub fn get_report(env: &Env, asset: &Address) -> DriftReport {
    let thresholds = get_thresholds(env);
    let window = get_window(env, asset);
    let n = window.len();

    let mut sum: i128 = 0;
    let mut max_abs: i128 = 0;
    for i in 0..n {
        let b = window.get_unchecked(i);
        sum += b;
        let abs = if b < 0 { -b } else { b };
        if abs > max_abs {
            max_abs = abs;
        }
    }
    let mean = if n == 0 { 0 } else { sum / n as i128 };
    let consistency = directional_consistency(&window, mean, thresholds.bias_threshold_bps);

    let sustained_drift = n >= thresholds.min_samples
        && (if mean < 0 { -mean } else { mean }) >= thresholds.bias_threshold_bps
        && consistency >= DIRECTIONAL_CONSISTENCY_FLOOR_BPS;

    DriftReport {
        samples: n,
        misaligned_skipped: get_misaligned(env, asset),
        mean_bias_bps: mean,
        max_abs_divergence_bps: max_abs,
        directional_consistency_bps: consistency,
        sustained_drift,
        bias_threshold_bps: thresholds.bias_threshold_bps,
        min_samples: thresholds.min_samples,
    }
}

/// Clears the rolling window and misaligned counter for `asset`. Admin-only.
///
/// Metrics are retained over long windows by design; this exists for the
/// deliberate "start measuring from here" case, not as routine maintenance.
pub fn reset_window(env: &Env, asset: Address) {
    get_admin(env).require_auth();
    let window_key = DataKey::DriftWindow(asset.clone());
    if env.storage().persistent().has(&window_key) {
        env.storage().persistent().remove(&window_key);
    }
    let mis_key = DataKey::DriftMisaligned(asset.clone());
    if env.storage().persistent().has(&mis_key) {
        env.storage().persistent().remove(&mis_key);
    }
}
