//! # Source influence caps (#475)
//!
//! Bounds the share of aggregate weight any single source can hold.
//!
//! * **Cap** — `cap_bps` in `1_000..=10_000` (10 %–100 %), default `5_000`
//!   (the 50 % share previously hard-coded in `freshness_weight`). Owner: the
//!   contract admin, via `set_influence_cap`. `10_000` disables the cap.
//! * **Weighted path (method 4)** — after freshness weighting, every weight is
//!   lowered until `w_i / Σw <= cap_bps / 10_000`. Capping only lowers
//!   weights; the weight sum is recomputed from the capped weights, so the
//!   invariant "effective influence sums to 100 %" is kept.
//! * **Unweighted paths (median etc.)** — each contributing source has one
//!   vote, i.e. an influence of `1/n`. The median additionally bounds a lone
//!   source to moving the result no further than its neighbouring order
//!   statistic, so no extra enforcement is needed there.
//! * **Quorum fallback** — the cap never removes a source, so it cannot make a
//!   satisfiable quorum unsatisfiable. With fewer than 3 sources the cap is not
//!   applied (a two-source weighted median interpolates anyway), and when
//!   `n * cap_bps < 10_000` no weight assignment can satisfy the cap, so all
//!   sources fall back to equal weight (`1/n`, the smallest achievable max share).
//! * **Collusion** — `k` colluding sources together hold at most
//!   `min(10_000, k * cap_bps)` bps of weight; with `k * cap_bps < 5_000` they
//!   cannot hold a weighted majority and so cannot choose the weighted median.
//!
//! The effective per-source influence (bps) is emitted in
//! `InfluenceCapAppliedEvent` in contributing-submission order.

use soroban_sdk::{panic_with_error, symbol_short, Env, Symbol, Vec};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::ErrorCode;

pub const BPS: u32 = 10_000;
pub const DEFAULT_CAP_BPS: u32 = 5_000;
pub const MIN_CAP_BPS: u32 = 1_000;
const KEY: Symbol = symbol_short!("INFL_CAP");

/// Current cap in basis points.
pub fn get_cap_bps(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&KEY)
        .unwrap_or(DEFAULT_CAP_BPS)
}

/// Sets the cap. Admin only; `cap_bps` must be in `1_000..=10_000`.
pub fn set_cap_bps(env: &Env, cap_bps: u32) {
    get_admin(env).require_auth();
    if !(MIN_CAP_BPS..=BPS).contains(&cap_bps) {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage().persistent().set(&KEY, &cap_bps);
    env.storage()
        .persistent()
        .extend_ttl(&KEY, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Lowers `weights` in place so no weight exceeds `cap_bps` of the total.
pub fn apply_cap(weights: &mut [u32], cap_bps: u32) {
    let n = weights.len();
    if n < 3 || cap_bps >= BPS {
        return;
    }
    if (n as u64) * (cap_bps as u64) < BPS as u64 {
        weights.iter_mut().for_each(|w| *w = 1);
        return;
    }
    // w / (w + others) <= c  <=>  w <= others * c / (1 - c)
    for _ in 0..n * 4 {
        let total: u64 = weights.iter().map(|w| *w as u64).sum();
        let mut changed = false;
        for w in weights.iter_mut() {
            let others = total - *w as u64;
            let max = (others * cap_bps as u64 / (BPS - cap_bps) as u64) as u32;
            if *w > max {
                *w = max.max(1);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    // Integer rounding can stall convergence near the boundary; equal weights
    // always satisfy the cap here because `n * cap_bps >= 10_000`.
    if !within_cap(weights, cap_bps) {
        weights.iter_mut().for_each(|w| *w = 1);
    }
}

/// Whether every weight is at most `cap_bps` of the total.
pub fn within_cap(weights: &[u32], cap_bps: u32) -> bool {
    let total: u64 = weights.iter().map(|w| *w as u64).sum();
    weights
        .iter()
        .all(|w| (*w as u64) * (BPS as u64) <= (cap_bps as u64) * total)
}

/// Effective influence of each weight in basis points of the total.
pub fn influence_bps(env: &Env, weights: &Vec<u32>) -> Vec<u32> {
    let total: u64 = weights.iter().map(|w| w as u64).sum();
    let mut out = Vec::new(env);
    for w in weights.iter() {
        out.push_back(if total == 0 {
            0
        } else {
            (w as u64 * BPS as u64 / total) as u32
        });
    }
    out
}
