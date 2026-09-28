//! # Submission-to-aggregate latency analytics (#492)
//!
//! Consumers previously saw only the timestamp of the last aggregate, so a
//! source whose submissions *consistently* arrive too late to be counted
//! looked identical to a healthy one — a silent participation loss. This
//! module makes that loss visible as a distribution.
//!
//! ## Units
//!
//! Every duration is in **ledgers**, not seconds: ledger timestamps have
//! coarse resolution and sources choose their own `timestamp`, so only the
//! ledger sequence is unambiguous. [`SECONDS_PER_LEDGER`] is reported
//! alongside the numbers so a consumer can convert to a wall-clock
//! approximation (Stellar's target close time is 5 s).
//!
//! ## Deferral is not latency
//!
//! Aggregation may be deferred past the submission — a policy min-interval,
//! an aggregation trigger, a full event budget. That is time the *oracle*
//! added, not time the *source* was late, and conflating the two hides a
//! source-side participation problem behind an oracle-side one. So each
//! sample separates:
//!
//! * `latency_ledgers = inclusion_ledger - submission_ledger` — how long
//!   the source's value waited to be counted.
//! * `deferral_ledgers = inclusion_ledger - previous_aggregate_ledger` —
//!   how long the publication as a whole was deferred past the previous
//!   aggregate. A source that is always late but never deferred shows a
//!   high latency with a low deferral.
//!
//! ## Never-counted submissions
//!
//! A submission replaced by a newer one from the same source before any
//! aggregate counts it is recorded with `counted: false` and
//! `inclusion_ledger: 0`, and emits [`SubmissionNeverCountedEvent`]. It is
//! excluded from the latency percentiles (a pending sample has no latency
//! yet) but counted in `never_counted`, so a source that never gets in is
//! distinguishable from a source that is merely slow.
//!
//! ## Bounded storage
//!
//! At most [`MAX_SAMPLES`] samples are kept per (source, asset) pair in a
//! ring; the oldest is dropped on overflow. Storage per pair is therefore
//! constant, not a function of the source's history, and the retained
//! window is returned verbatim in [`LatencyReport::window`] so every
//! percentile can be recomputed off-chain.

use soroban_sdk::{Address, Env, Vec};

use crate::events::{SubmissionLatencyEvent, SubmissionNeverCountedEvent};
use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, LatencyReport, LatencySample};

/// Rolling window length per (source, asset) pair. Fixed, so per-pair
/// storage is constant.
pub const MAX_SAMPLES: u32 = 16;
/// Nominal Stellar ledger close time, used only for the seconds
/// conversion hint on the report.
pub const SECONDS_PER_LEDGER: u32 = 5;

/// Ledger in which `asset` last published an aggregate, or 0 if never.
///
/// Stored inside the asset's disagreement record (#494) rather than in a key
/// of its own: a round that publishes already writes that entry, and a second
/// key for the same fact is enough on its own to push a wide batch over the
/// network footprint cap.
pub fn last_aggregate_ledger(env: &Env, asset: &Address) -> u32 {
    crate::disagreement::last_aggregate_ledger(env, asset)
}

/// The (source, asset) storage key of a latency window.
fn key(source: &Address, asset: &Address) -> DataKey {
    DataKey::LatencySamples(source.clone(), asset.clone())
}

/// Pushes `sample` into the (source, asset) ring, dropping the oldest entry
/// when the window is full.
pub fn push_sample(env: &Env, source: &Address, asset: &Address, sample: LatencySample) {
    let k = key(source, asset);
    let mut window: Vec<LatencySample> = env
        .storage()
        .persistent()
        .get(&k)
        .unwrap_or_else(|| Vec::new(env));
    // A source that is counted in the very ledger it submits is the healthy
    // case, and repeating that identical sample adds nothing a percentile can
    // see. Skipping the rewrite keeps one ledger entry per active
    // (source, asset) pair rather than one per round, which is what keeps a
    // wide batch inside the network footprint cap.
    if let Some(last) = window.last() {
        if last == sample {
            return;
        }
    }
    if window.len() >= MAX_SAMPLES {
        window.remove(0);
    }
    window.push_back(sample);
    env.storage().persistent().set(&k, &window);
    env.storage()
        .persistent()
        .extend_ttl(&k, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// The raw rolling window for a (source, asset) pair, oldest first.
pub fn get_samples(env: &Env, source: &Address, asset: &Address) -> Vec<LatencySample> {
    let k = key(source, asset);
    let v: Vec<LatencySample> = env
        .storage()
        .persistent()
        .get(&k)
        .unwrap_or_else(|| Vec::new(env));
    if !v.is_empty() {
        env.storage()
            .persistent()
            .extend_ttl(&k, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Records that `source`'s submission made in `submission_ledger` was
/// counted into the aggregate published at `inclusion_ledger`.
///
/// `deferral_ledgers` is the publication deferral measured from the
/// previous aggregate of the asset; the caller passes it in so the value
/// is consistent with the provenance record of the same round.
pub fn record_counted(
    env: &Env,
    source: &Address,
    asset: &Address,
    submission_ledger: u32,
    inclusion_ledger: u32,
    deferral_ledgers: u32,
) {
    let sample = LatencySample {
        submission_ledger,
        inclusion_ledger,
        latency_ledgers: inclusion_ledger.saturating_sub(submission_ledger),
        deferral_ledgers,
        counted: true,
    };
    push_sample(env, source, asset, sample.clone());
    SubmissionLatencyEvent {
        asset: asset.clone(),
        source: source.clone(),
        submission_ledger,
        inclusion_ledger,
        latency_ledgers: sample.latency_ledgers,
        deferral_ledgers,
    }
    .publish(env);
}

/// Records that `source`'s submission in `submission_ledger` was replaced
/// before any aggregate counted it.
///
/// The sample carries `counted: false` and `inclusion_ledger: 0`, which is
/// what makes "never counted" distinguishable from "counted, but slowly".
pub fn record_never_counted(env: &Env, source: &Address, asset: &Address, submission_ledger: u32) {
    push_sample(
        env,
        source,
        asset,
        LatencySample {
            submission_ledger,
            inclusion_ledger: 0,
            latency_ledgers: 0,
            deferral_ledgers: 0,
            counted: false,
        },
    );
    SubmissionNeverCountedEvent {
        asset: asset.clone(),
        source: source.clone(),
        submission_ledger,
    }
    .publish(env);
}

/// `p`-th percentile (0..=100) of `sorted` by nearest rank: the smallest
/// value at or above which `p` % of the samples fall.
pub fn percentile(sorted: &[u32], p: u32) -> u32 {
    let n = sorted.len();
    if n == 0 {
        return 0;
    }
    let rank = ((n as u64 * p as u64) + 99) / 100;
    let idx = (rank.max(1) as usize - 1).min(n - 1);
    sorted[idx]
}

/// Builds the latency report for a (source, asset) pair.
///
/// Percentiles cover the **counted** samples only — a never-counted
/// submission has no latency to rank — and `window` carries every stored
/// sample (counted and not) so the numbers can be verified off-chain.
pub fn get_report(env: &Env, source: &Address, asset: &Address) -> LatencyReport {
    let window = get_samples(env, source, asset);
    let cap = MAX_SAMPLES as usize;
    let mut latencies = [0u32; MAX_SAMPLES as usize];
    let mut deferrals = [0u32; MAX_SAMPLES as usize];
    let mut n = 0usize;
    let mut never = 0u32;
    for s in window.iter() {
        if s.counted {
            if n < cap {
                latencies[n] = s.latency_ledgers;
                deferrals[n] = s.deferral_ledgers;
                n += 1;
            }
        } else {
            never += 1;
        }
    }
    let mut sorted = latencies;
    sorted[..n].sort_unstable();
    let deferral_sum: u64 = deferrals[..n].iter().map(|d| *d as u64).sum();
    LatencyReport {
        asset: asset.clone(),
        source: source.clone(),
        samples: window.len(),
        never_counted: never,
        max_samples: MAX_SAMPLES,
        p50_ledgers: percentile(&sorted[..n], 50),
        p90_ledgers: percentile(&sorted[..n], 90),
        max_ledgers: if n == 0 { 0 } else { sorted[n - 1] },
        avg_deferral_ledgers: if n == 0 {
            0
        } else {
            (deferral_sum / n as u64) as u32
        },
        seconds_per_ledger: SECONDS_PER_LEDGER,
        window,
    }
}
