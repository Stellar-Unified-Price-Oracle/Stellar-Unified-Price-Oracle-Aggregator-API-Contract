//! # Aggregate confidence bands (#476)
//!
//! Publishes an interquartile band around each aggregate so consumers can tell
//! a tight consensus from a wide disagreement.
//!
//! ## Definition
//!
//! For the `n` contributing prices sorted ascending as `s[0..n]`, the band uses
//! nearest-rank quartiles:
//!
//! ```text
//! lower = s[floor((n - 1) / 4)]
//! upper = s[ceil(3 * (n - 1) / 4)]
//! ```
//!
//! Both bounds are contributed prices (no arithmetic), so they carry the
//! aggregate's decimals exactly and scale exactly with them. Because the median
//! indices `floor((n-1)/2)` and `n/2` lie between the two quartile indices, the
//! band always contains the median, and it is exactly zero-width when every
//! source agrees.
//!
//! A band computed from fewer than `max(min_sources_required, 4)` prices is
//! flagged `low_confidence`: with so few points the quartiles collapse onto
//! individual sources. See `docs/confidence-bands.md` for consumer guidance.

use soroban_sdk::{Address, Env, Vec};

use crate::storage::{compute_median, read_oracle_sources};
use crate::types::{ConfidenceBand, DataKey, PriceEntry};

/// Minimum number of prices for quartiles to be considered stable.
pub const MIN_BAND_SOURCES: u32 = 4;
const MAX_ENTRIES: usize = 128;

/// Nearest-rank `(lower, upper)` quartiles of `prices`; `(0, 0)` when empty.
pub fn quartiles(prices: &[i128]) -> (i128, i128) {
    let n = prices.len().min(MAX_ENTRIES);
    if n == 0 {
        return (0, 0);
    }
    let mut buf = [0i128; MAX_ENTRIES];
    buf[..n].copy_from_slice(&prices[..n]);
    let s = &mut buf[..n];
    s.sort_unstable();
    let lo = (n - 1) / 4;
    let hi = (3 * (n - 1)).div_ceil(4);
    (s[lo], s[hi])
}

/// Whether a band over `n` prices is below the confidence floor.
pub fn is_low_confidence(n: u32, min_sources_required: u32) -> bool {
    n < min_sources_required.max(MIN_BAND_SOURCES)
}

/// Builds the band for `prices`, or `None` when there are no prices.
pub fn band_for(env: &Env, prices: &Vec<i128>, decimals: u32) -> Option<ConfidenceBand> {
    let n = (prices.len() as usize).min(MAX_ENTRIES);
    if n == 0 {
        return None;
    }
    let mut buf = [0i128; MAX_ENTRIES];
    for (i, slot) in buf.iter_mut().take(n).enumerate() {
        *slot = prices.get_unchecked(i as u32);
    }
    let (lower, upper) = quartiles(&buf[..n]);
    let min_required = crate::admin::get_min_sources_required(env);
    Some(ConfidenceBand {
        lower,
        upper,
        median: compute_median(prices),
        num_sources: n as u32,
        decimals,
        low_confidence: is_low_confidence(n as u32, min_required),
    })
}

/// Band over the current submissions of `asset`.
pub fn get_confidence_band(env: &Env, asset: &Address) -> Option<ConfidenceBand> {
    let mut prices: Vec<i128> = Vec::new(env);
    for src in read_oracle_sources(env).sources.iter() {
        if prices.len() as usize >= MAX_ENTRIES {
            break;
        }
        let sub: Option<PriceEntry> = env
            .storage()
            .persistent()
            .get(&DataKey::Submission(asset.clone(), src));
        if let Some(e) = sub {
            prices.push_back(e.price);
        }
    }
    band_for(env, &prices, crate::admin::get_decimals(env))
}
