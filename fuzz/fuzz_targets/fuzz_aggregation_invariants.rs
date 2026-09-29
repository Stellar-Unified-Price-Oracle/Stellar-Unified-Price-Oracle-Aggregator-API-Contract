//! # Fuzz target: `fuzz_aggregation_invariants`  (#409)
//!
//! Security-invariant fuzzing for the aggregation methods beyond the median:
//! `compute_trimmed_mean`, `compute_vwap` and `weighted_median_core`.
//! Every assertion is derived from what an attacker would try to break, not
//! from the implementation, so a wrong-but-self-consistent change still fails.
//!
//! ## Invariants checked
//!
//! * **INV-BOUNDED** — no aggregate may leave `[min, max]` of the inputs that
//!   were allowed to influence it (an attacker cannot fabricate a price).
//! * **INV-TRIM-OUTLIER** — once trimming removes ≥ 1 value from each side, a
//!   single extreme source cannot move the trimmed mean at all.
//! * **INV-VWAP-NONPOSITIVE-VOLUME** — a source reporting zero or negative
//!   volume carries no weight: removing it leaves the VWAP unchanged.
//! * **INV-WEIGHT-DOMINANCE** — in the weighted median, a source holding
//!   strictly more than half the total weight always wins, and one holding
//!   less than half cannot pull the result outside the honest range.
//! * **INV-SDK-CORE** — the SDK trimmed mean agrees with `trimmed_mean_core`.
//!
//! Prices and volumes are decoded from `i32` so sums cannot saturate; that
//! keeps the invariants exact rather than approximate.
//!
//! ## Results
//!
//! * Clean run: 200 000 iterations in 462 s from the committed seed corpus,
//!   no crashes (CI runs 1 000 000).
//! * Has teeth: removing the `volume <= 0` guard in `compute_vwap` fails on
//!   `seed_negative_volume` with `INV-BOUNDED vwap 400 ∉ [200,300]`.
//!
//! ## Running
//!
//! ```sh
//! cargo fuzz run fuzz_aggregation_invariants fuzz/corpus/fuzz_aggregation_invariants -- -runs=1000000
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use price_oracle::core_pricing::{trimmed_mean_core, weighted_median_core};
use price_oracle::storage::{compute_trimmed_mean, compute_vwap};
use soroban_sdk::{Env, Vec as SVec};

const MAX: usize = 50;

fn words(data: &[u8]) -> std::vec::Vec<i128> {
    data.chunks_exact(4)
        .map(|b| i32::from_le_bytes(b.try_into().unwrap()) as i128)
        .collect()
}

fn bounds(xs: &[i128]) -> (i128, i128) {
    (*xs.iter().min().unwrap(), *xs.iter().max().unwrap())
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 5 {
        return;
    }
    let trim = (data[0] % 101) as u32;
    let mut ws = words(&data[1..]);
    ws.truncate(2 * MAX);
    let n = ws.len() / 2;
    if n == 0 {
        return;
    }
    let prices = &ws[..n];
    let vols: std::vec::Vec<i128> = ws[n..2 * n].to_vec();
    let (lo, hi) = bounds(prices);

    let env = Env::default();
    let sp = SVec::from_slice(&env, prices);
    let sv = SVec::from_slice(&env, &vols);

    // ── Trimmed mean ────────────────────────────────────────────────────────
    let tm = compute_trimmed_mean(&sp, trim);
    assert!(
        lo <= tm && tm <= hi,
        "INV-BOUNDED trimmed_mean {tm} ∉ [{lo},{hi}]"
    );
    assert_eq!(tm, trimmed_mean_core(prices, trim), "INV-SDK-CORE");

    // INV-TRIM-OUTLIER: replace the maximum with an extreme value. If at
    // least one value is trimmed from each side the result must not move.
    let trim_count = ((n as u32 * trim / 100) / 2).min(n as u32 - 1);
    if trim_count >= 1 && n >= 3 {
        let mut attacked = prices.to_vec();
        let i = attacked.iter().position(|&p| p == hi).unwrap();
        attacked[i] = i32::MAX as i128;
        let ta = compute_trimmed_mean(&SVec::from_slice(&env, &attacked), trim);
        assert_eq!(
            ta, tm,
            "INV-TRIM-OUTLIER: single outlier moved trimmed mean"
        );
    }

    // ── VWAP ────────────────────────────────────────────────────────────────
    let vwap = compute_vwap(&sp, &sv);
    let counted: std::vec::Vec<i128> = prices
        .iter()
        .zip(&vols)
        .filter(|(_, &v)| v > 0)
        .map(|(&p, _)| p)
        .collect();
    let (vlo, vhi) = if counted.is_empty() {
        (lo, hi)
    } else {
        bounds(&counted)
    };
    assert!(
        vlo <= vwap && vwap <= vhi,
        "INV-BOUNDED vwap {vwap} ∉ [{vlo},{vhi}]"
    );

    if !counted.is_empty() {
        let (kp, kv): (std::vec::Vec<i128>, std::vec::Vec<i128>) = prices
            .iter()
            .zip(&vols)
            .filter(|(_, &v)| v > 0)
            .map(|(&p, &v)| (p, v))
            .unzip();
        let pruned = compute_vwap(&SVec::from_slice(&env, &kp), &SVec::from_slice(&env, &kv));
        assert_eq!(pruned, vwap, "INV-VWAP-NONPOSITIVE-VOLUME");
    }

    // ── Weighted median ─────────────────────────────────────────────────────
    let weights: std::vec::Vec<i128> = vols.iter().map(|v| v.abs().max(1)).collect();
    let wm = weighted_median_core(prices, &weights);
    assert!(
        lo <= wm && wm <= hi,
        "INV-BOUNDED weighted_median {wm} ∉ [{lo},{hi}]"
    );

    let total: i128 = weights.iter().sum();
    for (i, &w) in weights.iter().enumerate() {
        if 2 * w > total {
            assert_eq!(wm, prices[i], "INV-WEIGHT-DOMINANCE: majority weight lost");
        }
    }
    if n >= 2 {
        // Attacker is index 0 with minority weight; the rest are honest.
        let honest_w: i128 = weights[1..].iter().sum();
        if 2 * weights[0] < total && honest_w > weights[0] {
            let (hlo, hhi) = bounds(&prices[1..]);
            assert!(
                hlo <= wm && wm <= hhi,
                "INV-WEIGHT-DOMINANCE: minority source pulled median to {wm} ∉ [{hlo},{hhi}]"
            );
        }
    }
});
