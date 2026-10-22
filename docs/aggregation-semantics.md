# Aggregation Semantics (#505)

The specification the contract's aggregation math is held to. This document
exists so that "what does the median do on an even count?" has one answer that
is written down rather than re-derived from the implementation.

Every rule below is pinned by a test:

| Rule | Test |
|---|---|
| Median, odd / even count | `spec_median_*`, `diff_median_matches_reference` |
| Mean truncation | `spec_mean_truncates_toward_zero` |
| Trimmed mean | `spec_trimmed_mean_*`, `diff_trimmed_mean_matches_reference` |
| Weighted median tie-break | `spec_weighted_median_*`, `diff_weighted_median_matches_reference` |
| VWAP volumes and saturation | `spec_vwap_nonpositive_volume_and_saturation` |
| Operating envelope | `characterise_first_128_window_above_envelope`, `median_core_window_parity_is_consistent_above_the_envelope` |

The independent reference implementation of these rules is
`contracts/price-oracle/src/reference_pricing.rs`; the differential suite that
compares it against the contract is `reference_diff_tests.rs`.

## 1. Two implementations, one specification

Aggregation exists twice in the crate:

| Implementation | Input | Used by |
|---|---|---|
| `core_pricing.rs` | `&[i128]` (no `Env`, no allocator) | fuzz targets, WASM-side hot paths |
| `storage.rs` | `soroban_sdk::Vec<i128>` | the contract's published-price path |

They must agree, and `reference_diff_tests` asserts that they do — each property
runs one input through `core_*`, through the `storage::*` wrapper, and through
the naive reference, and requires all three to match.

## 2. Operating envelope

`core_pricing::MEDIAN_WINDOW = 128`. The `core_*` functions copy their input
into a fixed `[i128; MEDIAN_WINDOW]` stack buffer, so **only the first 128
inputs are considered**; anything beyond is ignored. `set_max_sources` keeps
production well below this bound.

`storage::compute_median` selects in place and has **no** such cap. Above the
envelope the two therefore disagree — a documented, characterised asymmetry
rather than an accident:

* inputs at or below 128 → both aggregate the whole set and agree;
* inputs above 128 → `median_core` aggregates the first 128, `compute_median`
  aggregates all of them.

Every branch inside `median_core` is derived from `len = min(n, MEDIAN_WINDOW)`,
the length of the window actually copied. Deriving the parity branch from the
full input length `n` instead is a real defect: for `n = 129` it would select
the odd-count rule (return a single element) while indexing an even-count
window, and silently return the wrong value. That bug was found by this
differential suite; it is now fixed and pinned by
`median_core_window_parity_is_consistent_above_the_envelope`.

## 3. Median

Sort ascending.

* **Odd** count → the middle element.
* **Even** count → `lower + (upper - lower) / 2`, i.e. the floor of the
  midpoint. Because `upper >= lower` after sorting, the division floors, so an
  even-count median **rounds toward the lower middle element**. `[1, 2]` → 1,
  `[1, 3]` → 2, `[1, 2, 2, 3]` → 2.

Both implementations use quickselect rather than a full sort, but select the
same elements, so the result is identical.

## 4. Mean

Truncated integer division of the exact sum by `n`: `sum / n`, which truncates
toward zero for both signs. `[3, 3, 3, 2]` (sum 11, n 4) → 2;
`[-3, -3, -3, -2]` (sum −11, n 4) → −2.

The sum accumulates with `checked_add`; a sum that would exceed `i128` makes the
plain mean fall back to `mean_no_overflow` (#464), which accumulates quotient
and remainder instead of a single sum. The differential reference returns `None`
outside the representable-sum domain so those inputs are skipped explicitly
rather than silently agreeing on a saturated value.

## 5. Trimmed mean

1. Sort ascending.
2. Drop `trim_count` from each end, where
   `trim_count = min(floor(n * trim_percent / 100) / 2, n - 1)`.
   The percentage arithmetic is integer throughout and each side is floored, so
   the two ends always drop the same number.
3. Average the remainder (truncated, per §4).
4. If the remainder would be empty, fall back to `sorted[n / 2]`.

`trim_percent = 0` is the plain mean. `trim_percent = 100` on two elements drops
one from each end, leaving nothing, so the fallback applies: `[10, 90]` → 90.

## 6. Weighted median

Weights are clamped to `>= 1` first, so a zero or negative weight still counts
as one unit rather than being discarded.

Sort `(price, weight)` pairs by price, then walk the cumulative weight. The
winner is the **first price at which the cumulative weight is strictly greater
than half the total**.

Two consequences worth stating explicitly:

* An **exact-half tie resolves to the next higher price**, because the strict
  `>` means the loop runs past the boundary element.
* Equal prices are interchangeable, so the unstable sort order is unobservable.

If `weights.len() != prices.len()` the function falls back to the unweighted
median rather than guessing at a pairing.

## 7. VWAP

* Volumes `<= 0` are ignored — they contribute to neither numerator nor
  denominator.
* Products and the running sums **saturate** rather than wrap, so an extreme
  price cannot produce a wrapped (and therefore wrong-signed) result.
* If no positive volume remains, the fallback is the **mean of all prices** —
  not the mean of only the positive-volume ones. `[100, 300]` with volumes
  `[0, -7]` → 200.
* The final division is truncated toward zero, like the plain mean.

## 8. Bounds

A non-override aggregate always lies within `[min, max]` of the submissions that
were allowed to influence it. This is invariant **S4** in
[`security/invariants.md`](security/invariants.md), checked over hostile
sequences rather than asserted here.
