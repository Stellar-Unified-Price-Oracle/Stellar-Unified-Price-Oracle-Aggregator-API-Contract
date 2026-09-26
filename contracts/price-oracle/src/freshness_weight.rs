//! # Freshness-weighted median
//!
//! Weights each source's submission by its age so a fresh value counts more
//! than a stale one. Aggregation method `4` uses it; the plain median stays
//! available for comparison via [`get_weighted_aggregate`].
//!
//! ## Weight function
//!
//! `age` is measured in seconds between the submission's ledger timestamp and
//! now. With a curve `(window_secs, min_weight)`:
//!
//! ```text
//! weight = W_MAX - (W_MAX - min_weight) * min(age, window_secs) / window_secs
//! ```
//!
//! so weights lie in `[min_weight, W_MAX]` (unit-less, `W_MAX = 1000`). Equal
//! ages give equal weights, which makes the result equal to the plain median.
//!
//! ## Bounded influence
//!
//! Weights are bounded by construction (`max/min <= W_MAX / min_weight`). On
//! top of that, when three or more sources contribute, no single weight may
//! exceed the configurable influence cap (default 50 %, see
//! [`crate::influence_cap`]), so a lone fresh value can never outvote the rest
//! by weight alone.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::storage::{
    compute_median, get_admin, read_oracle_sources, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{DataKey, ErrorCode, FreshnessCurve, PriceEntry, WeightedAggregate};

/// Weight of a submission made in the current second.
pub const W_MAX: u32 = 1000;
pub const DEFAULT_WINDOW_SECS: u64 = 300;
pub const DEFAULT_MIN_WEIGHT: u32 = 100;
pub const MAX_WINDOW_SECS: u64 = 86_400;
const MAX_ENTRIES: usize = 128;

/// Linear decay of weight with age; see the module docs.
pub fn weight_for_age(age_secs: u64, curve: &FreshnessCurve) -> u32 {
    let age = age_secs.min(curve.window_secs);
    let span = (W_MAX - curve.min_weight) as u64;
    W_MAX - (span * age / curve.window_secs) as u32
}

/// Returns the curve for `asset`, or the default when none is configured.
pub fn get_curve(env: &Env, asset: &Address) -> FreshnessCurve {
    let key = DataKey::FreshnessCurve(asset.clone());
    match env.storage().persistent().get(&key) {
        Some(c) => {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            c
        }
        None => FreshnessCurve {
            window_secs: DEFAULT_WINDOW_SECS,
            min_weight: DEFAULT_MIN_WEIGHT,
        },
    }
}

/// Configures the freshness curve of an asset. Admin only.
///
/// `window_secs` must be in `1..=86_400` and `min_weight` in `1..=1000`
/// (`1000` disables decay, i.e. equal weights).
pub fn set_curve(env: &Env, asset: Address, window_secs: u64, min_weight: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if window_secs == 0 || window_secs > MAX_WINDOW_SECS || min_weight == 0 || min_weight > W_MAX {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::FreshnessCurve(asset);
    env.storage().persistent().set(
        &key,
        &FreshnessCurve {
            window_secs,
            min_weight,
        },
    );
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Caps every weight at the sum of the others (50 % share) when `n >= 3`.
pub fn cap_weights(weights: &mut [u32]) {
    let n = weights.len();
    if n < 3 {
        return;
    }
    for _ in 0..n {
        let total: u64 = weights.iter().map(|w| *w as u64).sum();
        let mut changed = false;
        for w in weights.iter_mut() {
            let others = (total - *w as u64) as u32;
            if *w > others {
                *w = others;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
}

/// Weighted median. An even total weight interpolates between the two middle
/// values with the same rounding as `compute_median`, so equal weights give
/// exactly the plain median.
pub fn weighted_median(prices: &[i128], weights: &[u32]) -> i128 {
    let n = prices.len().min(weights.len()).min(MAX_ENTRIES);
    if n == 0 {
        return 0;
    }
    let mut pairs = [(0i128, 0u64); MAX_ENTRIES];
    let mut total: u64 = 0;
    for ((slot, price), weight) in pairs.iter_mut().zip(prices).zip(weights) {
        *slot = (*price, (*weight).max(1) as u64);
        total += slot.1;
    }
    let pairs = &mut pairs[..n];
    pairs.sort_unstable_by_key(|&(p, _)| p);

    let mut cumulative: u64 = 0;
    for i in 0..n {
        cumulative += pairs[i].1;
        if cumulative * 2 >= total {
            if cumulative * 2 == total && i + 1 < n {
                let (lo, hi) = (pairs[i].0, pairs[i + 1].0);
                return lo + (hi - lo) / 2;
            }
            return pairs[i].0;
        }
    }
    pairs[n - 1].0
}

/// Age-derived weight for a stored submission.
pub fn entry_weight(env: &Env, entry: &PriceEntry, curve: &FreshnessCurve) -> u32 {
    weight_for_age(
        env.ledger()
            .timestamp()
            .saturating_sub(entry.ledger_timestamp),
        curve,
    )
}

/// Freshness-weighted median over SDK vectors (already capped weights).
pub fn aggregate(prices: &Vec<i128>, weights: &Vec<u32>) -> i128 {
    let n = (prices.len() as usize).min(MAX_ENTRIES);
    let mut p = [0i128; MAX_ENTRIES];
    let mut w = [0u32; MAX_ENTRIES];
    for (i, (ps, ws)) in p.iter_mut().zip(w.iter_mut()).take(n).enumerate() {
        *ps = prices.get_unchecked(i as u32);
        *ws = weights.get(i as u32).unwrap_or(1);
    }
    weighted_median(&p[..n], &w[..n])
}

/// Caps `weights` (see [`cap_weights`]) and returns them as a new vector.
pub fn capped(env: &Env, weights: &Vec<u32>) -> Vec<u32> {
    let n = (weights.len() as usize).min(MAX_ENTRIES);
    let mut buf = [0u32; MAX_ENTRIES];
    for (i, slot) in buf.iter_mut().take(n).enumerate() {
        *slot = weights.get_unchecked(i as u32);
    }
    crate::influence_cap::apply_cap(&mut buf[..n], crate::influence_cap::get_cap_bps(env));
    let mut out: Vec<u32> = Vec::new(env);
    for w in buf[..n].iter() {
        out.push_back(*w);
    }
    out
}

/// Live raw vs weighted median over the current submissions of `asset`.
pub fn get_weighted_aggregate(env: &Env, asset: &Address) -> Option<WeightedAggregate> {
    let curve = get_curve(env, asset);
    let mut prices: Vec<i128> = Vec::new(env);
    let mut sources: Vec<Address> = Vec::new(env);
    let mut weights: Vec<u32> = Vec::new(env);
    for src in read_oracle_sources(env).sources.iter() {
        if prices.len() as usize >= MAX_ENTRIES {
            break;
        }
        let sub: Option<PriceEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::Submission(asset.clone(), src.clone()));
        if let Some(e) = sub {
            prices.push_back(e.price);
            sources.push_back(src);
            weights.push_back(entry_weight(env, &e, &curve));
        }
    }
    if prices.is_empty() {
        return None;
    }
    let weights = capped(env, &weights);
    Some(WeightedAggregate {
        raw_median: compute_median(&prices),
        weighted_median: aggregate(&prices, &weights),
        sources,
        weights,
    })
}
