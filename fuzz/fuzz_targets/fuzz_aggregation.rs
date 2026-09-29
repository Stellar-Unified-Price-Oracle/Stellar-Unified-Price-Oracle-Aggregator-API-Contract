//! # Fuzz target: `fuzz_aggregation`  (#189)
//!
//! Coverage-guided fuzzer that differentially tests every aggregation
//! function in `core_pricing.rs` (pure slice) against the corresponding
//! `storage.rs` function (Soroban-SDK `Vec<i128>`).
//!
//! ## What is fuzzed
//!
//! The fuzzer feeds raw bytes from libFuzzer into:
//!
//! | Core fn | Storage fn | Assertion |
//! |---|---|---|
//! | `median_core` | `compute_median` | identical i128 result |
//! | `mean_core` | `compute_mean` | identical i128 result |
//! | `trimmed_mean_core(_, 10)` | `compute_trimmed_mean(_, 10)` | identical i128 result |
//! | `weighted_median_core` | `compute_weighted_median` | identical i128 result |
//!
//! Any divergence causes an immediate panic, which libFuzzer records as a
//! crash corpus entry.
//!
//! ## Running
//!
//! ```sh
//! # One-shot: run for 1 million iterations then exit.
//! cargo fuzz run fuzz_aggregation -- -runs=1000000
//!
//! # Continuous: run until a bug is found or Ctrl-C.
//! cargo fuzz run fuzz_aggregation
//! ```
//!
//! ## Input encoding
//!
//! The raw byte buffer is interpreted as a sequence of `i64` values (8 bytes
//! each, little-endian).  The first half of the values are used as prices,
//! the second half as reputation weights (clamped 1–100).  This keeps the
//! fuzzer input space structured while still exercising boundary conditions.

#![no_main]

use libfuzzer_sys::fuzz_target;
use price_oracle::core_pricing::{
    mean_core, median_core, trimmed_mean_core, weighted_median_core,
};
use soroban_sdk::Env;

// We need a minimal Soroban Env for the SDK-side.  In the fuzz context the
// testutils feature is available because price-oracle is compiled with it.
fn to_sdk_vec(env: &Env, s: &[i128]) -> soroban_sdk::Vec<i128> {
    let mut v = soroban_sdk::Vec::new(env);
    for &x in s {
        v.push_back(x);
    }
    v
}

fuzz_target!(|data: &[u8]| {
    // Need at least two i64 (16 bytes) to form a non-trivial input.
    if data.len() < 16 {
        return;
    }

    // Decode bytes as i64 little-endian words → cast to i128.
    let mut raw_vals: Vec<i128> = data
        .chunks_exact(8)
        .map(|b| {
            let arr: [u8; 8] = b.try_into().unwrap();
            i64::from_le_bytes(arr) as i128
        })
        .collect();

    // Cap at 100 entries (matches the maximum allowed oracle sources).
    raw_vals.truncate(100);
    let n = raw_vals.len();
    if n == 0 {
        return;
    }

    // Split into prices (first half) and weights (second half).
    // If odd, prices get the extra element.
    let price_len = (n + 1) / 2;
    let prices: Vec<i128> = raw_vals[..price_len].to_vec();
    let raw_weights = &raw_vals[price_len..];
    // Clamp weights to [1, 100] so they represent valid reputation scores.
    let weights: Vec<i128> = raw_weights
        .iter()
        .map(|&w| w.rem_euclid(100) + 1)
        .collect();

    let env = Env::default();
    let sdk_prices = to_sdk_vec(&env, &prices);

    // ── median ───────────────────────────────────────────────────────────────
    {
        use price_oracle::storage::compute_median;
        let core_v = median_core(&prices);
        let sdk_v  = compute_median(&sdk_prices);
        assert_eq!(
            core_v, sdk_v,
            "MEDIAN DIVERGENCE: core={}, sdk={}, prices={:?}",
            core_v, sdk_v, &prices[..prices.len().min(8)]
        );
    }

    // ── mean ─────────────────────────────────────────────────────────────────
    {
        use price_oracle::storage::compute_mean;
        let core_v = mean_core(&prices);
        let sdk_v  = compute_mean(&sdk_prices);
        assert_eq!(
            core_v, sdk_v,
            "MEAN DIVERGENCE: core={}, sdk={}", core_v, sdk_v
        );
    }

    // ── trimmed mean ─────────────────────────────────────────────────────────
    {
        use price_oracle::storage::compute_trimmed_mean;
        let core_v = trimmed_mean_core(&prices, 10);
        let sdk_v  = compute_trimmed_mean(&sdk_prices, 10);
        assert_eq!(
            core_v, sdk_v,
            "TRIMMED_MEAN DIVERGENCE: core={}, sdk={}", core_v, sdk_v
        );
    }

    // ── weighted median ──────────────────────────────────────────────────────
    // Only run when we have a matching weight vector.
    if weights.len() == prices.len() {
        use price_oracle::storage::compute_weighted_median;
        let sdk_weights = to_sdk_vec(&env, &weights);
        let core_v = weighted_median_core(&prices, &weights);
        let sdk_v  = compute_weighted_median(&sdk_prices, &sdk_weights);
        assert_eq!(
            core_v, sdk_v,
            "WEIGHTED_MEDIAN DIVERGENCE: core={}, sdk={}", core_v, sdk_v
        );
    }
});
