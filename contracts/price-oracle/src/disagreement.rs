//! # Pairwise source disagreement index (#494)
//!
//! The confidence band of #476 shows how far the contributing prices spread
//! around the median, but not the *shape* of that spread: a single dissenting
//! source and a genuine two-way split produce the same band width, and a
//! consumer cannot tell a healthy-but-noisy round from a fractured one. This
//! module adds a first-class, scale-invariant disagreement metric.
//!
//! ## Definition
//!
//! For the `n` counted prices, let `med` be their median and, for every
//! pair `(i, j)`, the relative deviation
//!
//! ```text
//! d_ij = |p_i - p_j| * 10_000 / med      (basis points)
//! ```
//!
//! Then `index_bps` is the **median** of the `n(n-1)/2` pairwise
//! deviations, and `max_bps` the largest.
//!
//! ## Why the median of pairs, and not a mean
//!
//! * **Scale invariance.** Every deviation is divided by the median price, so
//!   the index is a pure ratio: the same prices expressed with 8 decimals
//!   and with 18 decimals give the same number. A consumer can compare a
//!   $1 pair against a $100 000 pair without conversion.
//! * **Lone dissent vs. broad split.** A lone dissenter inflates only the
//!   `n-1` pairs it takes part in, so the *median* of the pairs stays near
//!   the inlier spread while `max_bps` spikes. A broad split inflates most
//!   pairs, so `index_bps` rises with it. Reporting both is what separates
//!   the two cases; `index_bps` alone would miss the lone dissenter.
//! * **Few sources.** With `n < 2` there are no pairs and the index is `0`
//! by definition — no division by zero is possible, because the median of
//!   one price is that price and no pair is ever formed. `low_sample` is
//!   set for `n < 3`, where the index rests on at most one pair and should
//!   not be alerted on.
//!
//! ## Rolling baseline
//!
//! Genuine volatility spikes the index, and a spike is not an error. So the
//! current value is compared against a rolling median of the last
//! [`BASELINE_WINDOW`] values of the *same* asset, and `above_baseline` is
//! set only when the current index exceeds twice that baseline. The baseline
//! is reported in the event and the record, so a spike during a real move is
//! visible as a spike rather than as a fault. See
//! `docs/disagreement-index.md` for how consumers should read it.
//!
//! Storage is bounded: the history is a ring of [`BASELINE_WINDOW`] `u32`
//! values per asset, independent of how long the asset trades.

use soroban_sdk::{Address, Env, Vec};

use crate::events::DisagreementIndexEvent;
use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, DisagreementIndex, DisagreementRecord};

/// Number of historical index values kept per asset for the baseline.
pub const BASELINE_WINDOW: u32 = 16;
/// Upper bound on the prices scored in one round.
pub const MAX_PRICES: usize = 64;
/// Below this many sources the index is not a meaningful consensus signal.
pub const LOW_SAMPLE_SOURCES: u32 = 3;

/// Computes the pairwise relative deviations of `prices` in bps.
///
/// Returns `(index_bps, max_bps)` where `index_bps` is the median of the
/// `n(n-1)/2` pair deviations. `(0, 0)` for fewer than two prices.
///
/// `n` is bounded by [`MAX_PRICES`]; the pair count is bounded by
/// `MAX_PRICES^2 / 2`.
pub fn compute(prices: &[i128]) -> (u32, u32) {
    let n = prices.len().min(MAX_PRICES);
    if n < 2 {
        return (0, 0);
    }
    let mut sorted = [0i128; MAX_PRICES];
    sorted[..n].copy_from_slice(&prices[..n]);
    sorted[..n].sort_unstable();
    let med = if n % 2 == 1 {
        sorted[n / 2]
    } else {
        let (a, b) = (sorted[n / 2 - 1], sorted[n / 2]);
        a + (b - a) / 2
    };
    if med <= 0 {
        return (0, 0);
    }
    let cap = MAX_PRICES * (MAX_PRICES - 1) / 2;
    let mut devs = [0u32; MAX_PRICES * (MAX_PRICES - 1) / 2];
    let mut m = 0usize;
    let mut max_bps: u32 = 0;
    for i in 0..n {
        for j in (i + 1)..n {
            let dev = ((sorted[j] - sorted[i]).abs() as u128) * 10_000u128 / (med as u128);
            let bps = dev.min(u32::MAX as u128) as u32;
            if m < cap {
                devs[m] = bps;
                m += 1;
            }
            if bps > max_bps {
                max_bps = bps;
            }
        }
    }
    if m == 0 {
        return (0, max_bps);
    }
    let mut sorted_devs = devs;
    sorted_devs[..m].sort_unstable();
    (sorted_devs[m / 2], max_bps)
}

/// Median of a `u32` slice, used for the rolling baseline.
fn median_u32(values: &[u32]) -> u32 {
    if values.is_empty() {
        return 0;
    }
    let mut buf = [0u32; BASELINE_WINDOW as usize];
    let n = values.len().min(BASELINE_WINDOW as usize);
    buf[..n].copy_from_slice(&values[..n]);
    buf[..n].sort_unstable();
    buf[n / 2]
}

/// Ledger in which `asset` last published an aggregate, or 0 if never (#492).
pub fn last_aggregate_ledger(env: &Env, asset: &Address) -> u32 {
    read_record(env, asset)
        .map(|r| r.last_aggregate_ledger)
        .unwrap_or(0)
}

/// The stored index record of `asset`, or `None` when it has none.
fn read_record(env: &Env, asset: &Address) -> Option<DisagreementRecord> {
    let key = DataKey::DisagreementIndex(asset.clone());
    let v: Option<DisagreementRecord> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Baseline the current index is compared against: the rolling median of the
/// stored history of the asset, computed *before* the current value is
/// appended, so a spike never raises its own baseline.
fn baseline_of(hist: &Vec<u32>) -> u32 {
    if hist.is_empty() {
        return 0;
    }
    let mut buf = [0u32; BASELINE_WINDOW as usize];
    let n = (hist.len() as usize).min(BASELINE_WINDOW as usize);
    for (i, slot) in buf.iter_mut().take(n).enumerate() {
        *slot = hist.get_unchecked(i as u32);
    }
    median_u32(&buf[..n])
}

/// Records the index of a published aggregate, updates the rolling
/// baseline ring and publishes the event.
pub fn record(env: &Env, asset: &Address, ledger: u32, prices: &Vec<i128>) -> DisagreementIndex {
    let n = (prices.len() as usize).min(MAX_PRICES);
    let mut buf = [0i128; MAX_PRICES];
    for (i, slot) in buf.iter_mut().take(n).enumerate() {
        *slot = prices.get_unchecked(i as u32);
    }
    let (index_bps, max_bps) = compute(&buf[..n]);

    let prev_record = read_record(env, asset);
    let mut hist = prev_record
        .map(|r| r.history)
        .unwrap_or_else(|| Vec::new(env));
    let baseline_bps = baseline_of(&hist);
    let above = baseline_bps > 0 && index_bps > baseline_bps.saturating_mul(2);
    let low_sample = prices.len() < LOW_SAMPLE_SOURCES;
    let idx = DisagreementIndex {
        asset: asset.clone(),
        ledger,
        index_bps,
        max_bps,
        baseline_bps,
        above_baseline: above,
        num_sources: prices.len(),
        low_sample,
    };

    // The rolling window rides along in the same entry as the reading, so a
    // round costs one write rather than two.
    if hist.len() >= BASELINE_WINDOW {
        hist.remove(0);
    }
    hist.push_back(index_bps);

    let key = DataKey::DisagreementIndex(asset.clone());
    let rec = DisagreementRecord {
        index: idx.clone(),
        history: hist,
        last_aggregate_ledger: ledger,
    };
    env.storage().persistent().set(&key, &rec);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    DisagreementIndexEvent {
        asset: asset.clone(),
        index_bps,
        max_bps,
        baseline_bps,
        above_baseline: above,
        num_sources: prices.len(),
        low_sample,
    }
    .publish(env);

    idx
}

/// The index recorded by the most recent aggregate of `asset`, or `None` if
/// the asset has never published one.
pub fn get_index(env: &Env, asset: &Address) -> Option<DisagreementIndex> {
    read_record(env, asset).map(|r| r.index)
}

/// The rolling window of prior index values backing the baseline.
pub fn get_history(env: &Env, asset: &Address) -> Vec<u32> {
    read_record(env, asset)
        .map(|r| r.history)
        .unwrap_or_else(|| Vec::new(env))
}
