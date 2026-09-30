#![cfg(test)]

//! # Independent reference implementation of the aggregation math (#505)
//!
//! This module is a deliberately **naive, obviously-correct** model of the
//! contract's aggregation semantics. It shares **no code** with
//! [`crate::core_pricing`] (quickselect-based) or [`crate::storage`]
//! (SDK-`Vec` based): every function here is written from the specification
//! in `docs/aggregation-semantics.md` using the simplest possible algorithm
//! (a full sort and a linear scan), so a bug in the contract's selection
//! algorithms cannot be mirrored here by construction.
//!
//! The differential suite (`reference_diff_tests.rs`) feeds randomised inputs
//! to the contract functions **and** to this reference and fails on any
//! divergence. Degenerate inputs, tie-breaks and rounding are pinned by
//! explicit hand-computed tests in the same suite.
//!
//! ## Specified semantics (summary — full text in `docs/aggregation-semantics.md`)
//!
//! * **Operating envelope** — the aggregation functions consider at most the
//!   first 128 inputs; callers must keep the source count below that
//!   (`set_max_sources`). Tests cover `1..=128` inputs and characterise the
//!   behaviour above 128 explicitly.
//! * **Median** — sort ascending; odd count → middle element; even count →
//!   `lower + (upper - lower) / 2` (integer division, i.e. the floor of the
//!   midpoint — rounds toward the *lower* middle element).
//! * **Mean** — truncated division `sum / n` toward zero, defined for inputs
//!   whose exact sum fits in `i128` (guaranteed by the asset price bounds).
//!   Returns `None` outside that domain so tests can skip it explicitly
//!   instead of silently agreeing on a saturated value.
//! * **Trimmed mean** — sort; drop `trim / 2` percent from each end
//!   (integer percent arithmetic, each side floored, capped at `n - 1`);
//!   average the remainder; if the remainder is empty fall back to the
//!   middle element `sorted[n / 2]`.
//! * **Weighted median** — clamp every weight to `>= 1`, sort
//!   `(price, weight)` pairs by price, walk cumulative weight; the winner is
//!   the first price at which the cumulative weight is **strictly greater**
//!   than half the total (an exact-half tie therefore resolves to the *next*
//!   higher price). Equal prices are interchangeable, so the unstable sort
//!   order is unobservable. Length mismatch falls back to the unweighted
//!   median.
//! * **VWAP** — ignore non-positive volumes, saturating `price * volume`
//!   products; if no positive volume remains, fall back to the mean of all
//!   prices.

/// Independent reference median. See the module docs for the exact spec.
pub fn ref_median(prices: &[i128]) -> i128 {
    if prices.is_empty() {
        return 0;
    }
    let mut sorted = prices.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        let lower = sorted[n / 2 - 1];
        let upper = sorted[n / 2];
        // (upper - lower) >= 0 after sorting, so `/ 2` floors: the result is
        // the floor of the true midpoint (rounds toward the lower element).
        lower + (upper - lower) / 2
    }
}

/// Independent reference mean: truncated division of the exact sum by `n`.
///
/// The specified domain is order-independent: the sum of the positive terms
/// and the sum of the negative terms must each fit in `i128` (implied by the
/// asset price bounds and source cap). Returns `None` outside that domain so
/// tests skip it explicitly instead of silently agreeing on a saturated or
/// partially-overflowed value. `n == 0` is specified as `0`.
pub fn ref_mean(prices: &[i128]) -> Option<i128> {
    let mut pos: i128 = 0;
    let mut neg: i128 = 0;
    for &p in prices {
        if p >= 0 {
            pos = pos.checked_add(p)?;
        } else {
            neg = neg.checked_add(p)?;
        }
    }
    if prices.is_empty() {
        return Some(0);
    }
    let sum = pos.checked_add(neg)?;
    Some(sum / prices.len() as i128)
}

/// Independent reference trimmed mean. `trim_percent` is in `0..=100`.
///
/// Returns `None` when the mean of the trimmed slice is outside the
/// specified envelope (its exact sum does not fit in `i128`).
pub fn ref_trimmed_mean(prices: &[i128], trim_percent: u32) -> Option<i128> {
    if prices.is_empty() {
        return Some(0);
    }
    if trim_percent == 0 {
        return ref_mean(prices);
    }
    let mut sorted = prices.to_vec();
    sorted.sort_unstable();
    let n = sorted.len();
    // Integer percent arithmetic: floor(n * trim / 100) / 2, capped at n - 1.
    let trim_count = ((n as u32).saturating_mul(trim_percent) / 100 / 2).min(n as u32 - 1) as usize;
    if trim_count == 0 {
        return ref_mean(&sorted);
    }
    let trimmed = &sorted[trim_count..n - trim_count];
    if trimmed.is_empty() {
        // Specified fallback: the middle element of the sorted slice.
        return Some(sorted[n / 2]);
    }
    ref_mean(trimmed)
}

/// Independent reference weighted median. See the module docs for the spec.
pub fn ref_weighted_median(prices: &[i128], weights: &[i128]) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    if n == 1 {
        return prices[0];
    }
    if weights.len() != n {
        return ref_median(prices);
    }
    let mut pairs: std::vec::Vec<(i128, i128)> = prices
        .iter()
        .zip(weights.iter())
        .map(|(&p, &w)| (p, w.max(1)))
        .collect();
    // Sort by price. Ties are unobservable: equal prices compare equal in
    // value regardless of their relative order.
    pairs.sort_unstable_by_key(|&(p, _)| p);

    let total: i128 = pairs
        .iter()
        .fold(0i128, |acc, &(_, w)| acc.saturating_add(w));
    let half = total / 2;
    let mut cumulative: i128 = 0;
    let mut median_idx = pairs.len() - 1;
    for (i, &(_, w)) in pairs.iter().enumerate() {
        cumulative = cumulative.saturating_add(w);
        if cumulative > half {
            median_idx = i;
            break;
        }
    }
    // Exact-half tie at loop exhaustion interpolates with the next price
    // (kept literal so the reference documents the full specified rule).
    let price_at = pairs[median_idx].0;
    if total % 2 == 0 && cumulative == half && median_idx + 1 < pairs.len() {
        let next = pairs[median_idx + 1].0;
        return price_at + (next - price_at) / 2;
    }
    price_at
}

/// Independent reference VWAP over positive volumes with saturating products.
///
/// Returns `None` only when the specified fallback (mean of *all* prices —
/// not just the counted ones) falls outside the mean's envelope.
pub fn ref_vwap(prices: &[i128], volumes: &[i128]) -> Option<i128> {
    let n = prices.len().min(volumes.len());
    if n == 0 {
        return Some(0);
    }
    let mut weighted_sum: i128 = 0;
    let mut total_volume: i128 = 0;
    for i in 0..n {
        let v = volumes[i];
        if v <= 0 {
            continue;
        }
        weighted_sum = weighted_sum.saturating_add(prices[i].saturating_mul(v));
        total_volume = total_volume.saturating_add(v);
    }
    if total_volume == 0 {
        // Specified fallback: the mean of *all* prices (not only the
        // counted ones) — mirrors `storage::compute_vwap`.
        ref_mean(prices)
    } else {
        Some(weighted_sum / total_volume)
    }
}

/// Reference bounds check: every aggregate must lie within `[min, max]` of
/// the inputs that were allowed to influence it. `None` for empty input.
pub fn ref_bounds(xs: &[i128]) -> Option<(i128, i128)> {
    xs.iter().min().copied().zip(xs.iter().max().copied())
}
