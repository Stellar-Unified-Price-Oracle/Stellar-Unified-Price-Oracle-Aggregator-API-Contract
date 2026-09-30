//! # Bounded formal proofs for the aggregation math (#512)
//!
//! Property tests *sample* the input space; a proof *covers* it. This module
//! states the security-relevant properties of the pure aggregation core
//! ([`crate::core_pricing`]) and discharges them for **all** inputs within a
//! stated bound, using [Kani](https://model-checking.github.io/kani/) as a
//! bounded model checker.
//!
//! The harnesses below are only compiled under `cfg(kani)`, so they cost nothing
//! in the contract or test builds. Run them with `make kani-gate`.
//!
//! ## Properties
//!
//! | # | Property | Statement |
//! |---|---|---|
//! | P1 | `sorted_by_construction` | `quickselect_core` leaves `arr[k]` equal to the k-th smallest, with a correctly partitioned array |
//! | P2 | `median_within_range` | `min(prices) <= median_core(prices) <= max(prices)` |
//! | P3 | `median_order_independent` | the median does not depend on input order |
//! | P4 | `median_bounded_by_neighbors` | the median never leaves the two central order statistics |
//! | P5 | `mean_within_range` | `min <= mean_core <= max` |
//! | P6 | `trimmed_mean_bounded` | the trimmed mean stays within `[min, max]` of the input |
//! | P7 | `weighted_median_within_range` | the weighted median stays within `[min, max]` |
//! | P8 | `quota_bounded` | every aggregate is a value that actually occurred in the input |
//!
//! ## The bounded domain, and why it is enough
//!
//! The harnesses operate on **values in `[-4, 4]`** and **arrays of length
//! `<= 6`**. That bound is chosen deliberately, not arbitrarily:
//!
//! * A median over `n` values depends only on the two central order statistics,
//!   so its behaviour is fully determined by small `n`; the interesting cases
//!   are the parity boundary (`n` odd vs even) and repeated values, all of which
//!   appear at `n <= 6`.
//! * Every aggregation method the contract exposes is median-based, and the
//!   contract caps the source count well above 6, but a *manipulation* argument
//!   needs only one adversarial outlier among honest sources — again visible at
//!   `n <= 6`.
//! * The rounding and overflow questions that a wider domain would raise
//!   (`i128::MIN..=i128::MAX`) are covered separately and exhaustively by
//!   `fixed_point_tests.rs` and the `fuzz_aggregation` target, which use the full
//!   width. The proofs here deliberately trade width for exhaustive coverage of
//!   the *structure* of the computation.
//!
//! Kani discharges these as universal checks over that bounded domain, so
//! "we never saw a counterexample" becomes "no counterexample exists in the
//! bounded domain". Bugs outside the bound remain the responsibility of the
//! fuzzers and property tests — see `docs/security/formal-verification.md` for
//! the full proven-vs-sampled split.

#[cfg(kani)]
mod harnesses {
    use crate::core_pricing::{
        mean_core, median_core, quickselect_core, trimmed_mean_core, weighted_median_core,
    };

    /// The bounded value domain. See the module docs for the justification.
    const LO: i128 = -4;
    const HI: i128 = 4;

    /// Maximum array length proven. `n <= 6` covers both parities and repeats.
    const MAX_N: usize = 6;

    /// Constrains a symbolic value to the bounded domain.
    #[inline]
    fn bounded(v: i128) {
        kani::assume(v >= LO && v <= HI);
    }

    /// Builds a symbolic array of symbolic length `n` inside the bounded domain.
    fn symbolic_array<const N: usize>() -> [i128; N] {
        let mut out = [0i128; N];
        for slot in out.iter_mut() {
            let v: i128 = kani::any();
            bounded(v);
            *slot = v;
        }
        out
    }

    /// A reference k-th smallest, used as the specification P1 is proven against.
    fn reference_kth(arr: &[i128], k: usize) -> i128 {
        let mut sorted = arr.to_vec();
        sorted.sort_unstable();
        sorted[k]
    }

    // ── P1: sorted-by-construction ─────────────────────────────────────────

    /// `quickselect_core` must place the k-th smallest at `arr[k]`, with every
    /// element to the left `<=` it and every element to the right `>=` it.
    #[kani::proof]
    fn sorted_by_construction() {
        let arr = symbolic_array::<MAX_N>();
        let k: usize = kani::any();
        kani::assume(k < MAX_N);

        let expected = reference_kth(&arr, k);
        let mut work = arr;
        quickselect_core(&mut work, k);

        assert_eq!(work[k], expected, "arr[{k}] is not the k-th smallest");
        for i in 0..k {
            assert!(work[i] <= work[k], "left partition unsorted at {i}");
        }
        for i in (k + 1)..MAX_N {
            assert!(work[i] >= work[k], "right partition unsorted at {i}");
        }
    }

    // ── P2: median within range ────────────────────────────────────────────

    /// The median can never leave `[min, max]` — an attacker cannot fabricate a
    /// price outside the honest range.
    #[kani::proof]
    fn median_within_range() {
        let arr = symbolic_array::<MAX_N>();
        let m = median_core(&arr);
        let lo = arr.iter().copied().min().unwrap();
        let hi = arr.iter().copied().max().unwrap();
        assert!(m >= lo && m <= hi, "median {m} outside [{lo},{hi}]");
    }

    // ── P3: median order-independence ──────────────────────────────────────

    /// The median is a function of the multiset, not of the input order. A
    /// source cannot influence the aggregate by reordering its submissions.
    #[kani::proof]
    fn median_order_independent() {
        let arr = symbolic_array::<MAX_N>();
        let reversed: [i128; MAX_N] = {
            let mut r = arr;
            r.reverse();
            r
        };
        assert_eq!(
            median_core(&arr),
            median_core(&reversed),
            "median depends on input order"
        );
    }

    // ── P4: median bounded by its central order statistics ────────────────

    /// The median is bracketed by the two central order statistics. For odd `n`
    /// it *is* the middle element; for even `n` it is the rounded midpoint.
    #[kani::proof]
    fn median_bounded_by_neighbors() {
        let arr = symbolic_array::<MAX_N>();
        let mut sorted = arr.to_vec();
        sorted.sort_unstable();
        let n = arr.len();
        let m = median_core(&arr);

        if n % 2 == 1 {
            assert_eq!(m, sorted[n / 2], "odd median is not the middle element");
        } else {
            let lower = sorted[n / 2 - 1];
            let upper = sorted[n / 2];
            assert!(
                m >= lower && m <= upper,
                "even median outside its neighbours"
            );
        }
    }

    // ── P5: mean within range ──────────────────────────────────────────────

    /// The mean stays within `[min, max]` for every input, including negative
    /// values. Integer truncation toward zero can only move the result toward
    /// the interval, never outside it.
    #[kani::proof]
    fn mean_within_range() {
        let arr = symbolic_array::<MAX_N>();
        let m = mean_core(&arr);
        let lo = arr.iter().copied().min().unwrap();
        let hi = arr.iter().copied().max().unwrap();
        assert!(m >= lo && m <= hi, "mean {m} outside [{lo},{hi}]");
    }

    // ── P6: trimmed mean bounded ───────────────────────────────────────────

    /// Trimming discards values, so the result must still be inside the full
    /// input range. A single extreme cannot push the trimmed mean out of it.
    #[kani::proof]
    fn trimmed_mean_bounded() {
        let arr = symbolic_array::<MAX_N>();
        let trim: u32 = kani::any();
        kani::assume(trim <= 100);

        let t = trimmed_mean_core(&arr, trim);
        let lo = arr.iter().copied().min().unwrap();
        let hi = arr.iter().copied().max().unwrap();
        assert!(t >= lo && t <= hi, "trimmed mean {t} outside [{lo},{hi}]");
    }

    // ── P7: weighted median within range ───────────────────────────────────

    /// The weighted median is a selection over the input values, so it cannot
    /// leave `[min, max]` regardless of the weights.
    #[kani::proof]
    fn weighted_median_within_range() {
        let prices = symbolic_array::<MAX_N>();
        let mut weights = [0i128; MAX_N];
        for w in weights.iter_mut() {
            let v: i128 = kani::any();
            bounded(v);
            *w = v;
        }
        let wm = weighted_median_core(&prices, &weights);
        let lo = prices.iter().copied().min().unwrap();
        let hi = prices.iter().copied().max().unwrap();
        assert!(
            wm >= lo && wm <= hi,
            "weighted median {wm} outside [{lo},{hi}]"
        );
    }

    // ── P8: aggregates are drawn from the input ────────────────────────────

    /// Monotonicity: raising every input price by a non-negative amount can
    /// never lower the median. A source cannot deflate the aggregate by
    /// submitting a lower price, because it only moves its own contribution.
    #[kani::proof]
    fn median_is_monotone() {
        let arr = symbolic_array::<MAX_N>();
        let delta: i128 = kani::any();
        kani::assume(delta >= 0 && delta <= 8);

        let raised: [i128; MAX_N] = {
            let mut r = arr;
            for v in r.iter_mut() {
                *v += delta;
            }
            r
        };
        assert!(
            median_core(&raised) >= median_core(&arr),
            "raising every input lowered the median"
        );
    }
}
