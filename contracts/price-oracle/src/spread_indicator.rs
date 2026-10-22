//! # Consumer-Visible Uncertainty and Confidence Semantics (#713)
//!
//! This module computes and stores a [`SpreadIndicator`] for every asset,
//! updated on every successful price aggregation. The indicator gives consumers
//! a quantified view of how much the contributing oracle sources disagree.
//!
//! ## What this measures
//!
//! The spread indicator is based on the **interquartile range (IQR)** of the
//! contributing source prices. The IQR is the difference between the upper
//! quartile (Q3) and lower quartile (Q1) of the sorted price set.
//!
//! ```text
//! sorted_prices = [p0, p1, ..., p_{n-1}]
//! Q1 = sorted_prices[floor((n-1)/4)]
//! Q3 = sorted_prices[ceil(3*(n-1)/4)]
//! IQR = Q3 - Q1
//! relative_spread_bps = IQR * 10_000 / median
//! ```
//!
//! The **relative spread** in basis points (bps) is the primary signal exposed
//! to consumers. It is scale-invariant: the same disagreement looks the same
//! regardless of whether prices are expressed in 8- or 18-decimal units.
//!
//! ## What this does NOT measure
//!
//! - **Accuracy:** Sources can agree on an incorrect price. A tight spread is
//!   evidence of consensus, not correctness.
//! - **Probability:** `relative_spread_bps` is NOT a probability of being
//!   within some tolerance of the true price. There is no probabilistic
//!   interpretation.
//! - **Outlier resistance:** The IQR naturally downweights extreme outliers
//!   (they move Q1/Q3 only slightly), but it does not detect all manipulation.
//!
//! ## Classification thresholds
//!
//! | Band | Relative spread | Meaning |
//! |---|---|---|
//! | `Tight` | < 50 bps | Sources closely agree |
//! | `Normal` | 50–200 bps | Typical market disagreement |
//! | `Wide` | 200–500 bps | Notable divergence; widen risk margins |
//! | `VeryWide` | > 500 bps OR `low_confidence` | High uncertainty; avoid risk-sensitive operations |
//!
//! These thresholds are advisory. Risk-sensitive consumers SHOULD define their
//! own acceptance criteria appropriate to their use case.
//!
//! ## Low-confidence flag
//!
//! When `num_sources < 3`, Q1 and Q3 both collapse to single data points and
//! the IQR width is not statistically meaningful. The `low_confidence` flag is
//! set to `true` in this case. Consumers MUST NOT treat `low_confidence = true`
//! indicators as reliable dispersion estimates.

use soroban_sdk::{Address, Env, Vec};

use crate::storage::{compute_median, sort_prices, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, SpreadIndicator, SpreadInterpretation};

// ─────────────────────────────────────────────────────────────────────────────
// Classification thresholds (in basis points)
// ─────────────────────────────────────────────────────────────────────────────

/// Relative spread below this value (bps) → `Tight` (0.5%)
pub const TIGHT_THRESHOLD_BPS: u32 = 50;
/// Relative spread below this value (bps) → `Normal` (2%)
pub const NORMAL_THRESHOLD_BPS: u32 = 200;
/// Relative spread below this value (bps) → `Wide` (5%)
pub const WIDE_THRESHOLD_BPS: u32 = 500;
// Above WIDE_THRESHOLD_BPS → `VeryWide`

/// Minimum source count for the spread to be statistically meaningful.
pub const MIN_SOURCES_FOR_SPREAD: u32 = 3;

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the most-recently computed spread indicator for `asset`.
///
/// Returns `None` if no aggregation has been performed for this asset yet.
pub fn get_spread_indicator(env: &Env, asset: &Address) -> Option<SpreadIndicator> {
    let key = DataKey::SpreadIndicator(asset.clone());
    let indicator: Option<SpreadIndicator> = env.storage().persistent().get(&key);
    if indicator.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    indicator
}

/// Computes and stores a new spread indicator from the current contributing prices.
///
/// `prices` is the slice of prices from all sources that contributed to the
/// current aggregation (not all registered sources — only those with
/// unexpired submissions). `median` is the already-computed aggregate price.
/// `decimals` is the decimal precision in effect. `timestamp` is the
/// most-recent contributing submission's Unix timestamp.
///
/// This is called from `prices.rs` immediately after a successful aggregate is
/// published, so the indicator is always aligned with the current aggregate.
pub fn compute_and_store(
    env: &Env,
    asset: &Address,
    prices: &Vec<i128>,
    median: i128,
    decimals: u32,
    timestamp: u64,
) {
    let n = prices.len();
    let ledger = env.ledger().sequence();

    if n == 0 {
        // No contributors → store a sentinel with VeryWide / low_confidence.
        let indicator = SpreadIndicator {
            price: median,
            q1: 0,
            q3: 0,
            iqr: 0,
            relative_spread_bps: 0,
            spread_interpretation: SpreadInterpretation::VeryWide,
            num_sources: 0,
            decimals,
            low_confidence: true,
            ledger,
            timestamp,
        };
        store_indicator(env, asset, &indicator);
        return;
    }

    // Sort a local copy to compute quartiles.
    let mut sorted = prices.clone();
    sort_prices(&mut sorted);

    let low_confidence = n < MIN_SOURCES_FOR_SPREAD;

    // Nearest-rank quartile indices (same formula as confidence_band.rs).
    let q1_idx = (n - 1) / 4;
    let q3_idx = (3 * (n - 1) + 3) / 4; // ceil(3*(n-1)/4)
    let q3_idx = q3_idx.min(n - 1);

    let q1 = sorted.get_unchecked(q1_idx);
    let q3 = sorted.get_unchecked(q3_idx);
    let iqr = q3.saturating_sub(q1).max(0);

    let relative_spread_bps: u32 = if median <= 0 || iqr == 0 {
        0
    } else {
        // iqr * 10_000 / median, capped at u32::MAX
        let bps = (iqr as u128)
            .saturating_mul(10_000)
            .saturating_div(median.unsigned_abs());
        bps.min(u32::MAX as u128) as u32
    };

    let spread_interpretation = if low_confidence || relative_spread_bps > WIDE_THRESHOLD_BPS {
        SpreadInterpretation::VeryWide
    } else if relative_spread_bps > NORMAL_THRESHOLD_BPS {
        SpreadInterpretation::Wide
    } else if relative_spread_bps >= TIGHT_THRESHOLD_BPS {
        SpreadInterpretation::Normal
    } else {
        SpreadInterpretation::Tight
    };

    let indicator = SpreadIndicator {
        price: median,
        q1,
        q3,
        iqr,
        relative_spread_bps,
        spread_interpretation,
        num_sources: n,
        decimals,
        low_confidence,
        ledger,
        timestamp,
    };

    store_indicator(env, asset, &indicator);
}

/// Convenience re-export: computes the median of prices (used by callers who
/// need to compute both the aggregate and the spread from the same price list).
pub fn median_of(prices: &Vec<i128>) -> i128 {
    compute_median(prices)
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

fn store_indicator(env: &Env, asset: &Address, indicator: &SpreadIndicator) {
    let key = DataKey::SpreadIndicator(asset.clone());
    env.storage().persistent().set(&key, indicator);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

// ─────────────────────────────────────────────────────────────────────────────
// Emitting the spread event
// ─────────────────────────────────────────────────────────────────────────────

/// Emits a `SpreadIndicatorUpdatedEvent` for the given asset.
///
/// Called from the same place as `compute_and_store` so the event is always
/// consistent with the stored indicator.
#[allow(deprecated)]
pub fn emit_spread_event(env: &Env, asset: &Address, indicator: &SpreadIndicator) {
    let sym = soroban_sdk::symbol_short!("spread");
    env.events().publish(
        (sym, asset.clone()),
        (
            indicator.price,
            indicator.q1,
            indicator.q3,
            indicator.iqr,
            indicator.relative_spread_bps,
            indicator.spread_interpretation as u32,
            indicator.low_confidence,
        ),
    );
}
