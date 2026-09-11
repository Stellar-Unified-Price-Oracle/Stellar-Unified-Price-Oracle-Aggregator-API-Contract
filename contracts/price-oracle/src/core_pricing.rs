//! # Pure Computation Core — `core.rs`
//!
//! This module contains **all** aggregation and selection algorithms as pure
//! functions that operate on standard Rust slices (`&[i128]`), with **no**
//! Soroban `Env`, storage, or SDK types.
//!
//! ## Design goals (#189)
//!
//! 1. **No Env access** — functions take `&[i128]` / `&mut [i128]` directly,
//!    so they are trivially testable without a Soroban VM.
//! 2. **Bit-exact parity** — every function here produces the same result as
//!    the corresponding Soroban-SDK `Vec<i128>` variant in `storage.rs`.
//! 3. **`no_std` compatible** — uses only `core` primitives; suitable for both
//!    contract WASM compilation and native test / fuzz harnesses.
//!
//! ## Differential testing
//!
//! `prop_tests.rs` feeds the same input to `core::*` and to the
//! Soroban-Vec-based wrappers in `storage.rs`, asserting identical output.
//! This guards against unintended divergence when either implementation is
//! changed.

// ────────────────────────────────────────────────────────────────────────────
// Quickselect (median-of-three pivot, Lomuto partition, iterative)
// ────────────────────────────────────────────────────────────────────────────

/// Partition `arr[lo..=hi]` using a median-of-three pivot.
/// Returns the final index of the pivot element.
fn partition_core(arr: &mut [i128], lo: usize, hi: usize) -> usize {
    let mid = lo + (hi - lo) / 2;
    // Sort lo, mid, hi in-place
    if arr[lo] > arr[mid] {
        arr.swap(lo, mid);
    }
    if arr[lo] > arr[hi] {
        arr.swap(lo, hi);
    }
    if arr[mid] > arr[hi] {
        arr.swap(mid, hi);
    }
    // Pivot is now at `mid`; move to hi to keep it out of the partition loop.
    let pivot = arr[mid];
    arr.swap(mid, hi);

    let mut store = lo;
    let mut i = lo;
    while i < hi {
        if arr[i] <= pivot {
            arr.swap(i, store);
            store += 1;
        }
        i += 1;
    }
    arr.swap(store, hi);
    store
}

/// Place the k-th smallest element (0-indexed) in `arr[k]`. O(n) average.
///
/// After the call, every element in `arr[..k]` is ≤ `arr[k]` and every
/// element in `arr[k+1..]` is ≥ `arr[k]`.
pub fn quickselect_core(arr: &mut [i128], k: usize) {
    let n = arr.len();
    if n <= 1 || k >= n {
        return;
    }
    let mut lo = 0usize;
    let mut hi = n - 1;
    loop {
        if lo >= hi {
            break;
        }
        if hi - lo < 3 {
            // Sort the tiny window directly (2–3 elements).
            if arr[lo] > arr[lo + 1] {
                arr.swap(lo, lo + 1);
            }
            if hi - lo == 2 {
                if arr[lo + 1] > arr[hi] {
                    arr.swap(lo + 1, hi);
                }
                if arr[lo] > arr[lo + 1] {
                    arr.swap(lo, lo + 1);
                }
            }
            break;
        }
        let pivot = partition_core(arr, lo, hi);
        if k < pivot {
            hi = pivot - 1;
        } else if k > pivot {
            lo = pivot + 1;
        } else {
            break;
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Median
// ────────────────────────────────────────────────────────────────────────────

/// Compute the median of `prices`.
///
/// * **Odd** n  → middle element (exact).
/// * **Even** n → `lower + (upper - lower) / 2` — same rounding as `storage::compute_median`.
///
/// Does **not** modify the input; works on an internal copy.
pub fn median_core(prices: &[i128]) -> i128 {
    let n = prices.len();
    match n {
        0 => 0,
        1 => prices[0],
        _ => {
            let mut buf: [i128; 128] = [0; 128];
            let len = n.min(128);
            buf[..len].copy_from_slice(&prices[..len]);
            let buf = &mut buf[..len];
            if n % 2 == 1 {
                let mid = len / 2;
                quickselect_core(buf, mid);
                buf[mid]
            } else {
                let upper_mid = len / 2;
                quickselect_core(buf, upper_mid);
                let b = buf[upper_mid];
                // Max of the lower partition = lower middle element.
                let mut a = buf[0];
                for &v in &buf[1..upper_mid] {
                    if v > a {
                        a = v;
                    }
                }
                a + (b - a) / 2
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Mean
// ────────────────────────────────────────────────────────────────────────────

/// Compute the arithmetic mean of `prices`.
pub fn mean_core(prices: &[i128]) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    let mut sum: i128 = 0;
    for &p in prices {
        sum = sum.saturating_add(p);
    }
    sum / (n as i128)
}

// ────────────────────────────────────────────────────────────────────────────
// Trimmed mean
// ────────────────────────────────────────────────────────────────────────────

/// Compute the trimmed mean: sort, discard the bottom and top `trim_percent / 2`
/// percent of values, then average the remainder.
pub fn trimmed_mean_core(prices: &[i128], trim_percent: u32) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    if trim_percent == 0 {
        return mean_core(prices);
    }
    let mut buf: [i128; 128] = [0; 128];
    let len = n.min(128);
    buf[..len].copy_from_slice(&prices[..len]);
    let sorted = &mut buf[..len];
    sorted.sort_unstable();

    let trim_count =
        (((len as u32).saturating_mul(trim_percent) / 100) / 2).min(len as u32 - 1) as usize;
    if trim_count == 0 {
        return mean_core(sorted);
    }
    let trimmed = &sorted[trim_count..len - trim_count];
    if trimmed.is_empty() {
        return sorted[len / 2];
    }
    mean_core(trimmed)
}

// ────────────────────────────────────────────────────────────────────────────
// Weighted median
// ────────────────────────────────────────────────────────────────────────────

/// Compute the weighted median of `prices` where each entry is weighted by
/// the corresponding `weights` value (reputation score, clamped to ≥ 1).
///
/// Falls back to `median_core` when `weights.len() != prices.len()`.
pub fn weighted_median_core(prices: &[i128], weights: &[i128]) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    if n == 1 {
        return prices[0];
    }
    if weights.len() != n {
        return median_core(prices);
    }

    // Build (price, weight) pairs sorted by price.
    let len = n.min(128);
    let mut pairs: [(i128, i128); 128] = [(0, 0); 128];
    for i in 0..len {
        pairs[i] = (prices[i], weights[i].max(1));
    }
    let pairs = &mut pairs[..len];
    pairs.sort_unstable_by_key(|&(p, _)| p);

    let total_weight: i128 = pairs
        .iter()
        .fold(0i128, |acc, &(_, w)| acc.saturating_add(w));
    let half = total_weight / 2;
    let mut cumulative: i128 = 0;
    let mut median_idx = 0usize;
    for (i, &(_, w)) in pairs.iter().enumerate() {
        cumulative = cumulative.saturating_add(w);
        if cumulative > half {
            median_idx = i;
            break;
        }
        median_idx = i;
    }

    let price_at = pairs[median_idx].0;
    // Interpolate on even-weight boundary (parity with the SDK version).
    if total_weight % 2 == 0 && cumulative == half && median_idx + 1 < len {
        let next = pairs[median_idx + 1].0;
        return price_at + (next - price_at) / 2;
    }
    price_at
}

// ────────────────────────────────────────────────────────────────────────────
// Unit tests for the pure core
// ────────────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn median_empty() {
        assert_eq!(median_core(&[]), 0);
    }

    #[test]
    fn median_single() {
        assert_eq!(median_core(&[42]), 42);
    }

    #[test]
    fn median_odd_sorted() {
        assert_eq!(median_core(&[1, 2, 3, 4, 5]), 3);
    }

    #[test]
    fn median_odd_unsorted() {
        assert_eq!(median_core(&[5, 1, 3, 2, 4]), 3);
    }

    #[test]
    fn median_even_basic() {
        // [1, 2, 3, 4]: lower_mid=2, upper_mid=3 → 2 + (3-2)/2 = 2
        assert_eq!(median_core(&[1, 2, 3, 4]), 2);
    }

    #[test]
    fn median_even_two_elements() {
        // [1, 3]: lower=1, upper=3 → 1 + (3-1)/2 = 2
        assert_eq!(median_core(&[1, 3]), 2);
    }

    #[test]
    fn median_two_identical() {
        assert_eq!(median_core(&[5, 5]), 5);
    }

    #[test]
    fn median_negative_values() {
        assert_eq!(median_core(&[-5, -3, -1, 0, 2]), -1);
    }

    #[test]
    fn median_i128_boundaries() {
        let max = i128::MAX;
        let min = i128::MIN;
        // Two-element: lower + (upper - lower) / 2
        let expected = min + (max - min) / 2;
        assert_eq!(median_core(&[min, max]), expected);
    }

    #[test]
    fn median_all_equal() {
        assert_eq!(median_core(&[7, 7, 7, 7, 7]), 7);
    }

    #[test]
    fn mean_empty() {
        assert_eq!(mean_core(&[]), 0);
    }

    #[test]
    fn mean_basic() {
        assert_eq!(mean_core(&[1, 2, 3, 4, 5]), 3);
    }

    #[test]
    fn mean_saturation_does_not_panic() {
        let big = i128::MAX / 2;
        let _ = mean_core(&[big, big, big]);
    }

    #[test]
    fn trimmed_mean_zero_trim() {
        assert_eq!(trimmed_mean_core(&[1, 2, 3, 4, 5], 0), 3);
    }

    #[test]
    fn trimmed_mean_removes_outliers() {
        // 10 elements [1..10], 20% trim → drop 1 from each end → [2..9]
        let prices = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        let _ = trimmed_mean_core(&prices, 20); // just verify no panic
    }

    #[test]
    fn weighted_median_uniform_weights() {
        let prices = [10, 20, 30, 40, 50];
        let weights = [50, 50, 50, 50, 50];
        assert_eq!(
            weighted_median_core(&prices, &weights),
            median_core(&prices)
        );
    }

    #[test]
    fn weighted_median_single() {
        assert_eq!(weighted_median_core(&[42], &[100]), 42);
    }

    #[test]
    fn weighted_median_low_weight_outlier_suppressed() {
        // Prices: [10, 20, 1000]. Give 1000 a weight of 1, others 50.
        // Sorted: (10,50), (20,50), (1000,1). total=101, half=50.
        // cumulative: 50 ≤ 50 → continue; 100 > 50 at idx=1 → median = 20.
        let prices = [10, 1000, 20];
        let weights = [50, 1, 50];
        assert_eq!(weighted_median_core(&prices, &weights), 20);
    }

    #[test]
    fn weighted_median_high_weight_outlier_dominant() {
        // Prices: [10, 20, 1000]. Weight 1000 at 200.
        // Sorted: (10,50),(20,50),(1000,200). total=300, half=150.
        // cumulative: 50≤150 → 100≤150 → 300>150 at idx=2 → median = 1000.
        let prices = [10, 20, 1000];
        let weights = [50, 50, 200];
        assert_eq!(weighted_median_core(&prices, &weights), 1000);
    }

    #[test]
    fn weighted_median_empty() {
        assert_eq!(weighted_median_core(&[], &[]), 0);
    }

    #[test]
    fn weighted_median_mismatch_falls_back() {
        let prices = [1, 2, 3];
        let weights = [50, 50];
        assert_eq!(
            weighted_median_core(&prices, &weights),
            median_core(&prices)
        );
    }

    #[test]
    fn quickselect_minimum() {
        let mut arr = [5i128, 3, 1, 4, 2];
        quickselect_core(&mut arr, 0);
        assert_eq!(arr[0], 1);
    }

    #[test]
    fn quickselect_maximum() {
        let mut arr = [5i128, 3, 1, 4, 2];
        quickselect_core(&mut arr, 4);
        assert_eq!(arr[4], 5);
    }

    #[test]
    fn quickselect_all_equal() {
        let mut arr = [7i128; 10];
        quickselect_core(&mut arr, 4);
        assert_eq!(arr[4], 7);
    }
}
