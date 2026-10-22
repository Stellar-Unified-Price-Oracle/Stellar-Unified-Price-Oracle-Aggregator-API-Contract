#![cfg(test)]
//! #464 — Fixed-point arithmetic overflow and precision-loss exploit hunt.
//!
//! The arithmetic inventory lives in
//! `docs/security/adversarial-review-461-462-464-466.md`; `inventory_is_complete`
//! fails when a scaling site is added or removed without updating it.

use proptest::prelude::*;
use soroban_sdk::{Env, Vec};

use crate::core_pricing::{mean_core, median_core};
use crate::storage::{compute_mean, compute_median};

fn sdk(e: &Env, xs: &[i128]) -> Vec<i128> {
    let mut v = Vec::new(e);
    for x in xs {
        v.push_back(*x);
    }
    v
}

/// (file, source, number of `10i128.pow(` / `10u128.pow(` scaling sites).
const SCALING_SITES: &[(&str, &str, usize)] = &[
    ("bridge_oracle.rs", include_str!("bridge_oracle.rs"), 2),
    ("ibc_oracle.rs", include_str!("ibc_oracle.rs"), 2),
    ("eth_bridge.rs", include_str!("eth_bridge.rs"), 2),
    (
        "cross_chain_verify.rs",
        include_str!("cross_chain_verify.rs"),
        2,
    ),
    ("storage.rs", include_str!("storage.rs"), 0),
    ("core_pricing.rs", include_str!("core_pricing.rs"), 0),
    ("prices.rs", include_str!("prices.rs"), 0),
    (
        "per_asset_decimals.rs",
        include_str!("per_asset_decimals.rs"),
        0,
    ),
    ("challenger.rs", include_str!("challenger.rs"), 0),
];

#[test]
fn inventory_is_complete() {
    for (name, src, expected) in SCALING_SITES {
        let non_test = src.split("#[cfg(test)]").next().unwrap();
        let found =
            non_test.matches("10i128.pow(").count() + non_test.matches("10u128.pow(").count();
        assert_eq!(
            found, *expected,
            "{name}: scaling-site inventory out of date"
        );
    }
}

#[test]
fn mean_never_saturates() {
    let e = Env::default();
    let big = i128::MAX / 2 + 1;
    let xs = [big, big, big];
    // Previously the saturated sum produced i128::MAX / 3, far below every input.
    assert_eq!(mean_core(&xs), big);
    assert_eq!(compute_mean(&sdk(&e, &xs)), big);
    let xs = [i128::MAX, i128::MAX - 1];
    assert_eq!(mean_core(&xs), i128::MAX - 1);
    assert_eq!(compute_mean(&sdk(&e, &xs)), i128::MAX - 1);
}

#[test]
fn median_extremes_do_not_overflow() {
    let e = Env::default();
    for xs in [
        [1, i128::MAX],
        [i128::MAX, i128::MAX],
        [i128::MAX - 1, i128::MAX],
    ] {
        let m = compute_median(&sdk(&e, &xs));
        assert!(m >= xs[0].min(xs[1]) && m <= xs[0].max(xs[1]));
        assert_eq!(m, median_core(&xs));
    }
}

#[test]
fn rounding_direction_is_floor_for_positive_prices() {
    let e = Env::default();
    // Median of an even set rounds toward the lower middle value.
    assert_eq!(compute_median(&sdk(&e, &[1, 2])), 1);
    assert_eq!(compute_median(&sdk(&e, &[10, 13])), 11);
    // Mean truncates toward zero (floor for positive prices).
    assert_eq!(compute_mean(&sdk(&e, &[1, 2])), 1);
    assert_eq!(compute_mean(&sdk(&e, &[2, 2, 3])), 2);
}

#[test]
fn published_median_matches_true_median_within_one_unit() {
    // Rounding boundary: published aggregate is never above the true median and
    // never more than half a unit below it, so repetition cannot accumulate gain.
    let e = Env::default();
    for (a, b) in [(1i128, 2i128), (7, 8), (i128::MAX - 1, i128::MAX)] {
        let m = compute_median(&sdk(&e, &[a, b]));
        let gap = (b - m) - (m - a);
        assert!(
            gap == 0 || gap == 1,
            "median {m} off true midpoint of {a},{b}"
        );
    }
}

proptest! {
    #[test]
    fn median_is_scale_invariant(
        xs in prop::collection::vec(1i128..1_000_000_000_000i128, 1..12),
        k in 0u32..=18,
    ) {
        let e = Env::default();
        let s = 10i128.pow(k);
        let scaled: std::vec::Vec<i128> = xs.iter().map(|x| x * s).collect();
        let base = compute_median(&sdk(&e, &xs));
        let at_scale = compute_median(&sdk(&e, &scaled));
        prop_assert_eq!(at_scale / s, base);
    }

    #[test]
    fn mean_is_scale_invariant(
        xs in prop::collection::vec(1i128..1_000_000_000_000i128, 1..12),
        k in 0u32..=18,
    ) {
        let e = Env::default();
        let s = 10i128.pow(k);
        let scaled: std::vec::Vec<i128> = xs.iter().map(|x| x * s).collect();
        prop_assert_eq!(compute_mean(&sdk(&e, &scaled)) / s, compute_mean(&sdk(&e, &xs)));
    }

    #[test]
    fn mean_fuzz_matches_exact(xs in prop::collection::vec(1i128..=i128::MAX, 1..8)) {
        // Exact truncated mean computed in a wider domain (i128 halves).
        let n = xs.len() as i128;
        let q: i128 = xs.iter().map(|x| x / n).sum();
        let r: i128 = xs.iter().map(|x| x % n).sum();
        prop_assert_eq!(mean_core(&xs), q + r / n);
        let e = Env::default();
        prop_assert_eq!(compute_mean(&sdk(&e, &xs)), mean_core(&xs));
    }
}
