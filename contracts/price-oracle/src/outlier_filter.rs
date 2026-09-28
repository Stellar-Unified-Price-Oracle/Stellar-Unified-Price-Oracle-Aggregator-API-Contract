//! # Robust outlier pre-filtering (#491)
//!
//! Applies a robust estimator — median absolute deviation (MAD) or the
//! interquartile range (IQR) — to the round's candidate prices *before*
//! aggregation, so a single grossly wrong source is removed instead of
//! flowing into the median. Both estimators are used because they are
//! themselves robust: one wild value barely moves them, unlike a
//! mean/standard-deviation z-score which the outlier drags along with it.
//!
//! ## Detectors
//!
//! **MAD (detector 1).** With `med` the median and `mad` the median absolute
//! deviation, the score of `x` is the modified z-score
//! `z = (x - med) / (1.4826 * mad)`, reported in basis points as
//! `score_bps = |x - med| * 10_000 / scaled_mad` where
//! `scaled_mad = mad * 14826 / 10_000`. `x` is excluded when
//! `score_bps >= sensitivity_bps`; `35_000` is the Iglewicz–Hoaglin default
//! of 3.5.
//!
//! **IQR (detector 2).** With `q1`, `q3` the nearest-rank quartiles, the
//! classic fence is `q1 - k * IQR` / `q3 + k * IQR` with
//! `k = sensitivity_bps / 10_000`; `15_000` reproduces the 1.5 rule. The
//! score is the distance past the fence in bps of the IQR.
//!
//! ## Guards
//!
//! * **Source-count floor.** Both estimators are unstable on small samples,
//!   so filtering is skipped entirely when the round has fewer than
//!   `min_sources` candidates (configurable per asset, floored at
//!   [`MIN_SOURCES_FLOOR`]).
//! * **Scale collapse.** When every candidate equals the centre the scale
//!   is `0` and no relative score exists. Values are then only excluded by
//!   an *absolute* floor: `|x - center|` must reach
//!   `sensitivity_bps / 10_000` of the centre, so a set of identical clean
//!   prices is never emptied.
//! * **Majority guard.** At most half the candidates may be dropped, so a
//!   lopsided round degrades to the unfiltered median rather than to a
//!   single-source aggregate. The caller re-checks quorum on the survivors.
//! * **Positivity.** Submitted prices are positive, so centre and scale are
//!   positive and no sign handling is needed.
//!
//! Excluded values are removed from *every* downstream statistic (median,
//! confidence band, provenance, latency accounting), and each exclusion is
//! published in [`OutlierExcludedEvent`] with its score, source, centre and
//! scale — enough to reproduce the decision off-chain. See
//! `docs/outlier-filtering.md`.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{OutlierConfigChangedEvent, OutlierExcludedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, OutlierConfig, OutlierExclusion};

/// Filtering disabled.
pub const DETECTOR_NONE: u32 = 0;
/// Median absolute deviation detector.
pub const DETECTOR_MAD: u32 = 1;
/// Interquartile range detector.
pub const DETECTOR_IQR: u32 = 2;

/// Hard floor on the configurable source-count floor: MAD and IQR are not
/// meaningful on fewer than four points.
pub const MIN_SOURCES_FLOOR: u32 = 4;
/// Highest accepted floor.
pub const MAX_SOURCES_FLOOR: u32 = 64;
/// Default Iglewicz–Hoaglin sensitivity for MAD (3.5 sigma, in bps).
pub const DEFAULT_MAD_SENSITIVITY_BPS: u32 = 35_000;
/// Default IQR sensitivity: the classic 1.5 x IQR rule (in 1/10_000 IQR).
pub const DEFAULT_IQR_SENSITIVITY_BPS: u32 = 15_000;
/// Default floor used when a config leaves it implicit.
pub const DEFAULT_MIN_SOURCES: u32 = 5;
/// Upper bound on any sensitivity (100 %).
pub const MAX_SENSITIVITY_BPS: u32 = 1_000_000;
/// Upper bound on the candidates a single round will score.
pub const MAX_CANDIDATES: usize = 128;

/// Returns the configuration in force for `asset`, or the default
/// (filtering disabled) when the asset has no override.
pub fn get_config(env: &Env, asset: &Address) -> OutlierConfig {
    let key = DataKey::OutlierConfig(asset.clone());
    let cfg: Option<OutlierConfig> = env.storage().persistent().get(&key);
    if cfg.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    cfg.unwrap_or(OutlierConfig {
        detector: DETECTOR_NONE,
        sensitivity_bps: DEFAULT_MAD_SENSITIVITY_BPS,
        min_sources: DEFAULT_MIN_SOURCES,
    })
}

/// Sets (or, with `None`, clears) the pre-filter configuration of one
/// asset. Admin only. Bounds are validated on write, so a stored config is
/// always usable.
pub fn set_config(env: &Env, asset: Address, config: Option<OutlierConfig>) {
    get_admin(env).require_auth();
    crate::storage::check_registered_asset(env, &asset);
    if let Some(c) = &config {
        if c.detector > DETECTOR_IQR
            || c.sensitivity_bps > MAX_SENSITIVITY_BPS
            || !(MIN_SOURCES_FLOOR..=MAX_SOURCES_FLOOR).contains(&c.min_sources)
        {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
    }
    let key = DataKey::OutlierConfig(asset.clone());
    let old: Option<OutlierConfig> = env.storage().persistent().get(&key);
    match &config {
        Some(c) => env.storage().persistent().set(&key, c),
        None => env.storage().persistent().remove(&key),
    }
    // Clearing removes the key, so only extend the TTL of one that is there.
    if config.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    OutlierConfigChangedEvent {
        asset,
        old,
        new: config,
    }
    .publish(env);
}

/// The default sensitivity for a detector, in that detector's own units.
pub fn default_sensitivity(detector: u32) -> u32 {
    if detector == DETECTOR_IQR {
        DEFAULT_IQR_SENSITIVITY_BPS
    } else {
        DEFAULT_MAD_SENSITIVITY_BPS
    }
}

/// Whether filtering runs at all for a round of `n` candidates.
///
/// A round below the floor keeps every price: with too few points the
/// estimators are dominated by a single observation, which is exactly the
/// instability this guard prevents.
pub fn filtering_active(cfg: &OutlierConfig, n: u32) -> bool {
    cfg.detector != DETECTOR_NONE && n >= cfg.min_sources.max(MIN_SOURCES_FLOOR)
}

/// 1.4826 scaled by 10_000, the factor that makes a MAD a
/// normal-equivalent sigma.
pub const MAD_SCALE: u32 = 14_826;

/// Copies an SDK `Vec` into a fixed buffer, returning it and its length.
fn to_buf(prices: &Vec<i128>) -> ([i128; MAX_CANDIDATES], usize) {
    let n = (prices.len() as usize).min(MAX_CANDIDATES);
    let mut buf = [0i128; MAX_CANDIDATES];
    for (i, slot) in buf.iter_mut().take(n).enumerate() {
        *slot = prices.get_unchecked(i as u32);
    }
    (buf, n)
}

/// Median of an already-sorted slice.
fn median_of(sorted: &[i128]) -> i128 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        let (a, b) = (sorted[n / 2 - 1], sorted[n / 2]);
        a + (b - a) / 2
    }
}

/// Median absolute deviation of an already-sorted slice, plus the median.
fn median_and_mad(sorted: &[i128]) -> (i128, i128) {
    let med = median_of(sorted);
    let mut devs = [0i128; MAX_CANDIDATES];
    for (i, slot) in devs.iter_mut().take(sorted.len()).enumerate() {
        *slot = (sorted[i] - med).abs();
    }
    devs[..sorted.len()].sort_unstable();
    (med, median_of(&devs[..sorted.len()]))
}

/// Score of `x` for the MAD detector, in bps of the scaled MAD, with the
/// centre and scale used. `scale == 0` signals a collapsed MAD.
fn mad_score(sorted: &[i128], x: i128) -> (u32, i128, i128) {
    let (med, mad) = median_and_mad(sorted);
    let scaled = (mad as i128) * (MAD_SCALE as i128) / 10_000;
    if scaled <= 0 {
        return (0, med, 0);
    }
    let score = ((x - med).abs() as u128) * 10_000u128 / (scaled as u128);
    (score.min(u32::MAX as u128) as u32, med, scaled)
}

/// Score of `x` for the IQR detector: distance past the fence in bps of
/// the IQR, with the fence centre and the IQR. `scale == 0` signals a
/// collapsed IQR.
fn iqr_score(sorted: &[i128], x: i128, sensitivity_bps: u32) -> (u32, i128, i128) {
    let n = sorted.len();
    let q1 = sorted[(n - 1) / 4];
    let q3 = sorted[(3 * (n - 1)).div_ceil(4)];
    let iqr = q3 - q1;
    let center = q1 + iqr / 2;
    if iqr <= 0 {
        return (0, center, 0);
    }
    // Fence offset = k * IQR with k = sensitivity_bps / 10_000.
    let fence = ((iqr as i128) * (sensitivity_bps as i128)) / 10_000;
    let dist = if x > q3 + fence {
        x - (q3 + fence)
    } else if x < q1 - fence {
        (q1 - fence) - x
    } else {
        return (0, center, iqr);
    };
    let score = ((dist as u128) * 10_000u128) / (iqr as u128);
    (score.min(u32::MAX as u128) as u32, center, iqr)
}

/// Whether `x` is excluded, with its score, centre and scale.
///
/// On a collapsed scale (`scale == 0`) the exclusion instead needs the
/// absolute floor: `|x - center|` must reach `sensitivity_bps / 10_000` of
/// the centre. That keeps a perfectly tight clean set intact while still
/// dropping a value orders of magnitude away from it.
pub fn score_of(sorted: &[i128], x: i128, cfg: &OutlierConfig) -> (bool, u32, i128, i128) {
    let (score, center, scale) = if cfg.detector == DETECTOR_IQR {
        iqr_score(sorted, x, cfg.sensitivity_bps)
    } else {
        mad_score(sorted, x)
    };
    if scale > 0 {
        return (score >= cfg.sensitivity_bps, score, center, scale);
    }
    let denom = center.abs();
    if denom == 0 {
        return (x != center, 0, center, 0);
    }
    let dev = (x - center).abs() as u128;
    let rel_bps = (dev * 10_000u128) / (denom as u128);
    (
        rel_bps >= cfg.sensitivity_bps as u128,
        rel_bps.min(u32::MAX as u128) as u32,
        center,
        0,
    )
}

/// Filters one round: returns a keep-mask parallel to `prices`.
///
/// `sources[i]` is the source that produced `prices[i]`. Every exclusion is
/// published in [`OutlierExcludedEvent`] and stored for
/// `get_outlier_exclusions`, so exclusions are auditable and the decision
/// reproducible off-chain.
pub fn filter_round(
    env: &Env,
    asset: &Address,
    prices: &Vec<i128>,
    sources: &Vec<Address>,
) -> Vec<bool> {
    let n = prices.len();
    let mut keep = Vec::new(env);
    for _ in 0..n {
        keep.push_back(true);
    }
    let cfg = get_config(env, asset);
    if !filtering_active(&cfg, n) {
        store_exclusions(env, asset, &Vec::new(env));
        return keep;
    }

    let (buf, len) = to_buf(prices);
    let mut sorted = buf;
    sorted[..len].sort_unstable();

    let mut scored: Vec<(u32, u32, i128, i128)> = Vec::new(env);
    for i in 0..n {
        let x = prices.get_unchecked(i);
        let (excluded, score, center, scale) = score_of(&sorted[..len], x, &cfg);
        if excluded {
            scored.push_back((i, score, center, scale));
        }
    }

    // Majority guard: never drop more than half the round.
    if scored.len() > n / 2 {
        store_exclusions(env, asset, &Vec::new(env));
        return keep;
    }

    let mut exclusions: Vec<OutlierExclusion> = Vec::new(env);
    for k in 0..scored.len() {
        let (i, score, center, scale) = scored.get_unchecked(k);
        keep.set(i, false);
        let price = prices.get_unchecked(i);
        // `sources` is always parallel to `prices` — `aggregate_asset` builds
        // both in the same loop — so an out-of-range index here would be a
        // caller bug, not a runtime condition to paper over.
        let source = sources.get_unchecked(i);
        exclusions.push_back(OutlierExclusion {
            source: source.clone(),
            price,
            score_bps: score,
            center,
            scale,
        });
        OutlierExcludedEvent {
            asset: asset.clone(),
            // 0 flags the absolute floor, not a broken estimator.
            detector: if scale > 0 {
                cfg.detector
            } else {
                DETECTOR_NONE
            },
            source,
            price,
            score_bps: score,
            sensitivity_bps: cfg.sensitivity_bps,
            center,
            scale,
            num_retained: n - scored.len(),
        }
        .publish(env);
    }
    store_exclusions(env, asset, &exclusions);
    keep
}

/// Stores the round's exclusions, removing the key when there are none.
///
/// An empty round is the common case — filtering is off until an operator
/// configures it — and writing an empty list every round would cost a ledger
/// entry per asset for no information.
fn store_exclusions(env: &Env, asset: &Address, exclusions: &Vec<OutlierExclusion>) {
    let key = DataKey::OutlierExclusions(asset.clone());
    if exclusions.is_empty() {
        env.storage().persistent().remove(&key);
        return;
    }
    env.storage().persistent().set(&key, exclusions);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Exclusions produced by the most recent aggregation of `asset`.
pub fn get_exclusions(env: &Env, asset: &Address) -> Vec<OutlierExclusion> {
    let key = DataKey::OutlierExclusions(asset.clone());
    let v: Vec<OutlierExclusion> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    if !v.is_empty() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}
