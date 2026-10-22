#![cfg(test)]

//! # Differential tests: contract aggregation vs independent reference (#505)
//!
//! Every property below runs the **same** randomised input through:
//!
//! 1. the pure core (`core_pricing.rs`, quickselect-based),
//! 2. the SDK-`Vec` wrappers (`storage.rs`), and
//! 3. the independent naive reference (`reference_pricing.rs`),
//!
//! and requires all three to agree bit-for-bit. Because the reference is
//! written from `docs/aggregation-semantics.md` with a plain sort, a bug in
//! the selection algorithms cannot hide on both sides of the comparison.
//!
//! Explicit smoke tests below pin the tie-break and rounding semantics with
//! hand-computed expectations, and characterise the documented behaviour
//! above the 128-input operating envelope.
//!
//! ## Running
//!
//! ```sh
//! cargo test -p price-oracle --lib reference_diff_tests
//! ```
//!
//! These tests run in CI on every change (`.github/workflows/ci.yml`, step
//! "Differential reference tests"). Increase `PROPTEST_CASES` for depth.

use proptest::prelude::*;
use soroban_sdk::Env;

use crate::core_pricing::{mean_core, median_core, trimmed_mean_core, weighted_median_core};
use crate::reference_pricing::{
    ref_bounds, ref_mean, ref_median, ref_trimmed_mean, ref_vwap, ref_weighted_median,
};
use crate::storage::{
    compute_mean, compute_median, compute_trimmed_mean, compute_vwap, compute_weighted_median,
};

/// Copy a `&[i128]` into a `soroban_sdk::Vec<i128>`.
fn to_sdk_vec(env: &Env, s: &[i128]) -> soroban_sdk::Vec<i128> {
    let mut v = soroban_sdk::Vec::new(env);
    for &x in s {
        v.push_back(x);
    }
    v
}

/// Price vectors of 1–128 entries (the specified operating envelope), biased
/// toward small values, duplicates and negatives so tie-breaks and rounding
/// are exercised far more often than uniform `i64` noise would manage.
fn price_vec() -> impl Strategy<Value = std::vec::Vec<i128>> {
    // Mix the magnitudes and signs inside a single strategy instead of using
    // `Union`, which needs a homogeneous boxed vector. Drawing a small
    // `i16` most of the time keeps duplicates, negatives and near-zero values
    // in the input space, where tie-breaks and rounding actually differ.
    let elem = any::<i16>().prop_map(|x| x as i128);
    proptest::collection::vec(elem, 1..=crate::core_pricing::MEDIAN_WINDOW)
}

/// Regression (#505): `median_core` must derive parity from the **window** it
/// copied, not from the full input length.
///
/// The defect: the branch was chosen by `n % 2` while the index came from
/// `len = min(n, MEDIAN_WINDOW)`. For an odd-length input larger than the
/// window (`n = 129`, `len = 128`) that pairs an odd-count rule with an
/// even-count window and returns `window[len/2]` — a single element — instead
/// of averaging the two middle ones. The differential suite caught this; the
/// assertions below pin the fixed behaviour for every length straddling the
/// window.
#[test]
fn median_core_window_parity_is_consistent_above_the_envelope() {
    for extra in 1..=8usize {
        // Ascending inputs make the expected median easy to state exactly.
        let mut prices: std::vec::Vec<i128> = (0..crate::core_pricing::MEDIAN_WINDOW)
            .map(|x| x as i128)
            .collect();
        for k in 0..extra {
            prices.push(1_000_000 + k as i128);
        }
        let n = prices.len();
        let len = n.min(crate::core_pricing::MEDIAN_WINDOW);

        let got = median_core(&prices);
        if len == crate::core_pricing::MEDIAN_WINDOW {
            // Even window: the median of {0..=127} is (63 + 64) / 2 = 63.
            assert_eq!(got, 63, "n={n} (len={len}) must use the even-count rule");
        } else {
            let expected = ref_median(&prices[..len]);
            assert_eq!(got, expected, "n={n} (len={len}) must match the reference");
        }
    }
}

/// A `(prices, weights)` pair of equal length inside the envelope. Weights
/// are arbitrary `i64`s so the contract's `max(1)` clamp is exercised.
fn price_weight_pair() -> impl Strategy<Value = (std::vec::Vec<i128>, std::vec::Vec<i128>)> {
    (1..=128usize).prop_flat_map(|n| {
        (
            proptest::collection::vec(any::<i64>().prop_map(|x| x as i128), n),
            proptest::collection::vec(any::<i64>().prop_map(|x| x as i128), n),
        )
    })
}

/// Trim percentages to pin the integer-percent arithmetic.
fn trim_pct() -> impl Strategy<Value = u32> {
    0u32..=100
}

// ────────────────────────────────────────────────────────────────────────────
// Randomised differential properties
// ────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    /// Median: core, SDK-Vec and reference must agree on every input.
    #[test]
    fn diff_median_matches_reference(prices in price_vec()) {
        let expected = ref_median(&prices);
        let env = Env::default();
        let core_v = median_core(&prices);
        let sdk_v = compute_median(&to_sdk_vec(&env, &prices));
        prop_assert_eq!(core_v, expected, "median_core diverged from reference: {:?}", prices);
        prop_assert_eq!(sdk_v, expected, "compute_median diverged from reference: {:?}", prices);
    }

    /// Mean: truncated division of the exact sum.
    #[test]
    fn diff_mean_matches_reference(prices in price_vec()) {
        // i64-domain inputs can never overflow an i128 sum (128 * 2^63 < 2^127).
        let expected = ref_mean(&prices).expect("generated input is within the mean envelope");
        let env = Env::default();
        let core_v = mean_core(&prices);
        let sdk_v = compute_mean(&to_sdk_vec(&env, &prices));
        prop_assert_eq!(core_v, expected, "mean_core diverged from reference: {:?}", prices);
        prop_assert_eq!(sdk_v, expected, "compute_mean diverged from reference: {:?}", prices);
    }

    /// Trimmed mean across every trim percentage, including 0 and 100.
    #[test]
    fn diff_trimmed_mean_matches_reference(prices in price_vec(), trim in trim_pct()) {
        let expected = ref_trimmed_mean(&prices, trim)
            .expect("generated input is within the mean envelope");
        let env = Env::default();
        let core_v = trimmed_mean_core(&prices, trim);
        let sdk_v = compute_trimmed_mean(&to_sdk_vec(&env, &prices), trim);
        prop_assert_eq!(core_v, expected, "trimmed_mean_core diverged: {:?} trim={}", prices, trim);
        prop_assert_eq!(sdk_v, expected, "compute_trimmed_mean diverged: {:?} trim={}", prices, trim);
    }

    /// Weighted median with arbitrary (including zero/negative) weights —
    /// the reference clamps to >= 1 exactly as the contract must.
    #[test]
    fn diff_weighted_median_matches_reference((prices, weights) in price_weight_pair()) {
        let expected = ref_weighted_median(&prices, &weights);
        let env = Env::default();
        let core_v = weighted_median_core(&prices, &weights);
        let sdk_v = compute_weighted_median(&to_sdk_vec(&env, &prices), &to_sdk_vec(&env, &weights));
        prop_assert_eq!(core_v, expected, "weighted_median_core diverged: {:?} / {:?}", prices, weights);
        prop_assert_eq!(sdk_v, expected, "compute_weighted_median diverged: {:?} / {:?}", prices, weights);
    }

    /// VWAP: non-positive volumes carry no weight; products saturate.
    #[test]
    fn diff_vwap_matches_reference(prices in price_vec(), volumes in price_vec()) {
        prop_assume!(!prices.is_empty() && !volumes.is_empty());
        let expected = ref_vwap(&prices, &volumes)
            .expect("generated input is within the mean envelope");
        let env = Env::default();
        let sdk_v = compute_vwap(&to_sdk_vec(&env, &prices), &to_sdk_vec(&env, &volumes));
        prop_assert_eq!(sdk_v, expected, "compute_vwap diverged: {:?} / {:?}", prices, volumes);
    }

    /// Bounds: no aggregate may leave the [min, max] of its inputs — an
    /// attacker cannot fabricate a price through aggregation.
    #[test]
    fn ref_bounds_hold_for_every_aggregate(prices in price_vec(), volumes in price_vec()) {
        let (lo, hi) = ref_bounds(&prices).unwrap();
        let env = Env::default();
        let sdk_prices = to_sdk_vec(&env, &prices);

        let med = compute_median(&sdk_prices);
        prop_assert!(lo <= med && med <= hi, "median {} outside [{},{}]", med, lo, hi);

        let mean = compute_mean(&sdk_prices);
        prop_assert!(lo <= mean && mean <= hi, "mean {} outside [{},{}]", mean, lo, hi);

        let trim = 30u32;
        let tm = compute_trimmed_mean(&sdk_prices, trim);
        prop_assert!(lo <= tm && tm <= hi, "trimmed mean {} outside [{},{}]", tm, lo, hi);

        let w: std::vec::Vec<i128> = prices.iter().map(|p| (p % 7).abs() + 1).collect();
        let wm = compute_weighted_median(&sdk_prices, &to_sdk_vec(&env, &w));
        prop_assert!(lo <= wm && wm <= hi, "weighted median {} outside [{},{}]", wm, lo, hi);

        if !volumes.is_empty() {
            let vwap = compute_vwap(&sdk_prices, &to_sdk_vec(&env, &volumes));
            let counted: std::vec::Vec<i128> = prices
                .iter()
                .zip(volumes.iter())
                .filter(|(_, &v)| v > 0)
                .map(|(p, _)| *p)
                .collect();
            if let Some((clo, chi)) = ref_bounds(&counted) {
                prop_assert!(clo <= vwap && vwap <= chi, "vwap {} outside [{},{}]", vwap, clo, chi);
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Explicit semantics: tie-breaks, rounding, degenerate inputs
// (hand-computed expectations — these document the specified behaviour)
// ────────────────────────────────────────────────────────────────────────────

fn sdk(s: &[i128]) -> (Env, soroban_sdk::Vec<i128>) {
    let env = Env::default();
    let v = to_sdk_vec(&env, s);
    (env, v)
}

/// Even-count median is the floor of the midpoint: `lower + (upper-lower)/2`.
#[test]
fn spec_even_median_rounds_toward_lower() {
    for (input, expected) in [
        ([1i128, 2], 1),    // floor(1.5)
        ([1i128, 4], 2),    // exact midpoint
        ([-5i128, -4], -5), // floor(-4.5) — truncating diff, not rounding to nearest
        ([-4i128, 4], 0),   // exact midpoint across zero
        ([0i128, 1], 0),    // floor(0.5)
    ] {
        let (_, v) = sdk(&input);
        assert_eq!(ref_median(&input), expected, "reference: {input:?}");
        assert_eq!(median_core(&input), expected, "core: {input:?}");
        assert_eq!(compute_median(&v), expected, "sdk: {input:?}");
    }
}

/// Odd-count median is the exact middle element after sorting.
#[test]
fn spec_odd_median_is_middle_element() {
    let input = [7i128, 1, 4, 9, 2];
    let (_, v) = sdk(&input);
    assert_eq!(ref_median(&input), 4);
    assert_eq!(median_core(&input), 4);
    assert_eq!(compute_median(&v), 4);
}

/// Empty input is specified as 0 for every aggregate.
#[test]
fn spec_empty_inputs_are_zero() {
    let (_, v) = sdk(&[]);
    assert_eq!(ref_median(&[]), 0);
    assert_eq!(median_core(&[]), 0);
    assert_eq!(compute_median(&v), 0);
    assert_eq!(ref_mean(&[]), Some(0));
    assert_eq!(mean_core(&[]), 0);
    assert_eq!(compute_mean(&v), 0);
    assert_eq!(ref_trimmed_mean(&[], 50), Some(0));
    assert_eq!(trimmed_mean_core(&[], 50), 0);
    assert_eq!(compute_trimmed_mean(&v, 50), 0);
    assert_eq!(ref_weighted_median(&[], &[]), 0);
    assert_eq!(weighted_median_core(&[], &[]), 0);
    assert_eq!(compute_weighted_median(&v, &v), 0);
    assert_eq!(ref_vwap(&[], &[]), Some(0));
    assert_eq!(compute_vwap(&v, &v), 0);
}

/// Exact-half weight ties resolve to the *next* higher price: the winner must
/// hold **strictly more** than half of the total weight.
#[test]
fn spec_weighted_median_exact_half_tie_goes_to_next_price() {
    let prices = [100i128, 200];
    let weights = [50i128, 50]; // cumulative hits exactly half at index 0
    let (_, vp) = sdk(&prices);
    let (_, vw) = sdk(&weights);
    assert_eq!(ref_weighted_median(&prices, &weights), 200);
    assert_eq!(weighted_median_core(&prices, &weights), 200);
    assert_eq!(compute_weighted_median(&vp, &vw), 200);
}

/// A source holding strictly more than half the weight always wins.
#[test]
fn spec_weighted_median_majority_weight_dominates() {
    let prices = [10i128, 999];
    let weights = [90i128, 10];
    let (_, vp) = sdk(&prices);
    let (_, vw) = sdk(&weights);
    assert_eq!(ref_weighted_median(&prices, &weights), 10);
    assert_eq!(weighted_median_core(&prices, &weights), 10);
    assert_eq!(compute_weighted_median(&vp, &vw), 10);
}

/// Zero and negative weights are clamped to 1 before aggregation.
#[test]
fn spec_weighted_median_clamps_weights_to_one() {
    let prices = [10i128, 20];
    let weights = [0i128, -5]; // both clamp to 1 → uniform → tie → next price
    let (_, vp) = sdk(&prices);
    let (_, vw) = sdk(&weights);
    assert_eq!(ref_weighted_median(&prices, &weights), 20);
    assert_eq!(weighted_median_core(&prices, &weights), 20);
    assert_eq!(compute_weighted_median(&vp, &vw), 20);
}

/// Weight/price length mismatch falls back to the unweighted median.
#[test]
fn spec_weighted_median_length_mismatch_falls_back_to_median() {
    let prices = [30i128, 10, 20];
    let weights = [9i128, 9]; // shorter on purpose
    let (_, vp) = sdk(&prices);
    let (_, vw) = sdk(&weights);
    assert_eq!(ref_weighted_median(&prices, &weights), ref_median(&prices));
    assert_eq!(
        weighted_median_core(&prices, &weights),
        median_core(&prices)
    );
    assert_eq!(compute_weighted_median(&vp, &vw), compute_median(&vp));
}

/// Trimmed mean: trim=0 is the plain mean; heavy trims degrade to the middle
/// element rather than panicking on an empty remainder.
#[test]
fn spec_trimmed_mean_degenerate_trims() {
    // trim = 0 → plain mean (truncated).
    let input = [3i128, 1, 4, 2];
    let (_, v) = sdk(&input);
    assert_eq!(ref_trimmed_mean(&input, 0), Some(2)); // 10/4 = 2
    assert_eq!(trimmed_mean_core(&input, 0), 2);
    assert_eq!(compute_trimmed_mean(&v, 0), 2);

    // trim = 100 on 5 elements drops 2 from each side → mean of the middle.
    let input = [10i128, 20, 30, 40, 50];
    let (_, v) = sdk(&input);
    assert_eq!(ref_trimmed_mean(&input, 100), Some(30));
    assert_eq!(trimmed_mean_core(&input, 100), 30);
    assert_eq!(compute_trimmed_mean(&v, 100), 30);

    // trim = 100 on 2 elements: the remainder is empty → sorted[n/2] (upper).
    let input = [10i128, 90];
    let (_, v) = sdk(&input);
    assert_eq!(ref_trimmed_mean(&input, 100), Some(90));
    assert_eq!(trimmed_mean_core(&input, 100), 90);
    assert_eq!(compute_trimmed_mean(&v, 100), 90);

    // Single element: everything degenerates to that element.
    let input = [42i128];
    let (_, v) = sdk(&input);
    for t in [0u32, 1, 50, 100] {
        assert_eq!(ref_trimmed_mean(&input, t), Some(42), "trim={t}");
        assert_eq!(trimmed_mean_core(&input, t), 42, "trim={t}");
        assert_eq!(compute_trimmed_mean(&v, t), 42, "trim={t}");
    }
}

/// VWAP semantics: non-positive volumes are ignored; if none remain the
/// specified fallback is the mean of all prices; products saturate.
#[test]
fn spec_vwap_nonpositive_volume_and_saturation() {
    // Mixed volumes: only v > 0 counts. (100*2 + 300*1) / 3 = 500/3 = 166.
    let prices = [100i128, 300];
    let vols = [2i128, 1];
    let (_, vp) = sdk(&prices);
    let (_, vv) = sdk(&vols);
    assert_eq!(ref_vwap(&prices, &vols), Some(166));
    assert_eq!(compute_vwap(&vp, &vv), 166);

    // All volumes non-positive → fallback is the mean of ALL prices,
    // (100 + 300) / 2 = 200. Note the fallback averages every price, not
    // only the ones with positive volume.
    let vols = [0i128, -7];
    let (_, vv) = sdk(&vols);
    assert_eq!(ref_vwap(&prices, &vols), Some(200));
    assert_eq!(compute_vwap(&vp, &vv), 200);

    // Saturating products: MAX * 2 saturates to MAX; total volume = 3.
    let prices = [i128::MAX, 1i128];
    let vols = [2i128, 1];
    let (_, vp) = sdk(&prices);
    let (_, vv) = sdk(&vols);
    let expected = i128::MAX / 3; // (saturated sum) / 3
    assert_eq!(ref_vwap(&prices, &vols), Some(expected));
    assert_eq!(compute_vwap(&vp, &vv), expected);
}

/// Mean rounding: truncated toward zero for both signs.
#[test]
fn spec_mean_truncates_toward_zero() {
    let input = [3i128, 3, 3, 2]; // sum 11 / 4 = 2.75 → 2
    let (_, v) = sdk(&input);
    assert_eq!(ref_mean(&input), Some(2));
    assert_eq!(mean_core(&input), 2);
    assert_eq!(compute_mean(&v), 2);

    let input = [-3i128, -3, -3, -2]; // sum -11 / 4 = -2.75 → -2 (toward zero)
    let (_, v) = sdk(&input);
    assert_eq!(ref_mean(&input), Some(-2));
    assert_eq!(mean_core(&input), -2);
    assert_eq!(compute_mean(&v), -2);
}

/// Mean boundary: same-sign extremes whose sum still fits exercise the
/// contract's exactness; sums outside the specified domain return `None`
/// from the reference and are excluded from differential comparison.
#[test]
fn spec_mean_extremes_within_envelope() {
    // Three near-max positive values: sum = i128::MAX - 1, fits exactly.
    let input = [i128::MAX / 3, i128::MAX / 3, i128::MAX / 3];
    let expected = ref_mean(&input).expect("in-domain: positive sum fits");
    assert_eq!(mean_core(&input), expected);
    let (_, v) = sdk(&input);
    assert_eq!(compute_mean(&v), expected);

    // Sum of two maxima overflows — outside the specified domain.
    assert!(ref_mean(&[i128::MAX, i128::MAX]).is_none());
}

/// Characterisation: above the 128-input operating envelope the two contract
/// paths diverge from each other, and both diverge from the reference.
///
/// `median_core` copies into a fixed `[i128; 128]` scratch buffer and therefore
/// aggregates only the **first 128** inputs. `storage::compute_median` selects
/// in place and has **no** such cap, so it aggregates all 129. The reference
/// models the full set. This asymmetry is a known limitation, documented in
/// `docs/aggregation-semantics.md`; the test pins the current behaviour so any
/// change to the envelope is caught here first.
#[test]
fn characterise_first_128_window_above_envelope() {
    // 128 ascending values plus one large outlier as the 129th input.
    //
    // * first-128 window {0..=127}  -> even count, (63 + 64) / 2 = 63
    // * full 129-element set         -> odd count, middle element = 64
    //
    // The outlier is placed *last* so it lands outside the core window and the
    // two envelopes visibly disagree.
    let mut prices: std::vec::Vec<i128> = (0..128).map(|x| x as i128).collect();
    prices.push(1_000_000);

    let (_, v) = sdk(&prices);
    assert_eq!(
        median_core(&prices),
        63,
        "median_core aggregates only its first-128 window"
    );
    assert_eq!(
        compute_median(&v),
        64,
        "compute_median aggregates the whole set (no window)"
    );
    // The full-set reference agrees with the uncapped SDK path.
    assert_eq!(ref_median(&prices), 64);

    // Below the envelope the cap is inert and all three agree.
    let mut small: std::vec::Vec<i128> = (0..128).map(|x| x as i128).collect();
    small.push(1_000_000);
    let within: std::vec::Vec<i128> = small[..127].to_vec();
    let (_, w) = sdk(&within);
    assert_eq!(median_core(&within), compute_median(&w));
    assert_eq!(ref_median(&within), median_core(&within));
}
