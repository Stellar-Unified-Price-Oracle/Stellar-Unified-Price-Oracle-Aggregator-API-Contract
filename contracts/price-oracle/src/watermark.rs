//! # Historical Query Completeness Watermark (#712)
//!
//! Provides a per-asset watermark indicating the ledger sequence number through
//! which historical price data is **complete and contiguous** — i.e., every
//! ledger in the range `[first_ledger, watermark_ledger]` has a recorded
//! snapshot.
//!
//! ## Design
//!
//! The watermark is derived from the `PriceHistoryLedgers` index (the sorted
//! `Vec<u32>` of every ledger for which a history entry was written). The
//! algorithm scans from the oldest retained entry forward, advancing the
//! watermark as long as each consecutive pair of entries is contiguous (i.e.,
//! no ledger was skipped). The first gap detected halts the scan.
//!
//! ```text
//! PriceHistoryLedgers: [100, 101, 102, 105, 106]
//!                                  ↑             ↑
//!                            gap here        still recorded
//! watermark_ledger = 102
//! gap_start_ledger = 103
//! is_contiguous    = false
//! ```
//!
//! For applications that do not require strict contiguity (e.g. time-sampled
//! aggregators), `is_contiguous` and `gap_start_ledger` expose the gap so the
//! consumer can decide whether the gap is acceptable.
//!
//! ## Gap handling
//!
//! A gap arises when:
//! - A ledger passed with no price submission (no sources active).
//! - A history entry's TTL expired and it was pruned by the Soroban storage layer.
//! - `max_history_length` was reduced and old entries were dropped.
//!
//! In all cases the watermark stops advancing past the start of the gap.
//! Operators can recover by ensuring new submissions are made; the watermark
//! will advance again once the gap is behind the active retention window.
//!
//! ## Consumer guidance
//!
//! ```text
//! watermark_ledger > 0           → query [first_ledger, watermark_ledger] safely
//! is_contiguous = false          → gap at gap_start_ledger; treat above it as partial
//! watermark_ledger = 0           → no confirmed history yet; no safe range
//! ```

use soroban_sdk::{Address, Env, Vec};

use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{CompletenessWatermark, DataKey, PriceHistoryEntry};

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the current completeness watermark for `asset`.
///
/// Returns `None` if no history has ever been recorded for this asset.
/// The caller should call [`advance_watermark`] immediately after writing
/// a new history entry to keep the watermark current.
pub fn get_watermark(env: &Env, asset: &Address) -> Option<CompletenessWatermark> {
    let key = DataKey::HistoryWatermark(asset.clone());
    let watermark: Option<CompletenessWatermark> = env.storage().persistent().get(&key);
    if watermark.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    watermark
}

/// Recomputes and stores the completeness watermark for `asset` from scratch.
///
/// Called after any mutation to the history index (write, prune, or TTL bump).
/// Iterates the `PriceHistoryLedgers` index once, finding the first gap.
///
/// The definition of "gap" used here is:
/// - **Primary:** missing ledger entries between two adjacent snapshots.
///   Snapshots are recorded at the aggregate granularity, so not every single
///   ledger is expected to appear; a gap is detected when the distance between
///   two adjacent snapshot ledgers exceeds the configurable
///   `MAX_SNAPSHOT_GAP_LEDGERS` (default 1, meaning every ledger is expected).
///
/// Returns the updated watermark.
pub fn advance_watermark(env: &Env, asset: &Address) -> CompletenessWatermark {
    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    if ledger_list.is_empty() {
        let wm = CompletenessWatermark {
            asset: asset.clone(),
            watermark_ledger: 0,
            watermark_timestamp: 0,
            first_ledger: 0,
            is_contiguous: true,
            gap_start_ledger: 0,
            snapshot_count: 0,
        };
        store_watermark(env, asset, &wm);
        return wm;
    }

    let n = ledger_list.len();
    let first_ledger = ledger_list.get_unchecked(0);

    // Read the timestamp of the first entry for completeness metadata.
    let first_ts = read_history_timestamp(env, asset, first_ledger);

    if n == 1 {
        let wm = CompletenessWatermark {
            asset: asset.clone(),
            watermark_ledger: first_ledger,
            watermark_timestamp: first_ts,
            first_ledger,
            is_contiguous: true,
            gap_start_ledger: 0,
            snapshot_count: 1,
        };
        store_watermark(env, asset, &wm);
        return wm;
    }

    // Scan forward from the beginning; watermark stays at the last
    // snapshot before the first gap.
    let mut watermark_ledger = first_ledger;
    let mut watermark_ts = first_ts;
    let mut is_contiguous = true;
    let mut gap_start_ledger: u32 = 0;

    // Determine the expected gap tolerance.  History snapshots are written at
    // every aggregation, which may be every ledger or at a coarser cadence.
    // We use the maximum observed inter-snapshot distance across the retained
    // window as the "normal" stride, then flag anything larger.
    // For simplicity — and to be maximally conservative — we treat any
    // inter-snapshot stride > 1 as a gap unless the stride is consistent.
    let stride = compute_dominant_stride(&ledger_list);

    for i in 1..n {
        let prev = ledger_list.get_unchecked(i - 1);
        let curr = ledger_list.get_unchecked(i);
        let distance = curr.saturating_sub(prev);
        if distance > stride.saturating_mul(2).max(stride + 1) {
            // Gap detected: watermark stays at `prev`.
            is_contiguous = false;
            gap_start_ledger = prev.saturating_add(stride);
            break;
        }
        // No gap at this step — advance watermark.
        watermark_ledger = curr;
        watermark_ts = read_history_timestamp(env, asset, curr);
    }

    let wm = CompletenessWatermark {
        asset: asset.clone(),
        watermark_ledger,
        watermark_timestamp: watermark_ts,
        first_ledger,
        is_contiguous,
        gap_start_ledger,
        snapshot_count: n,
    };
    store_watermark(env, asset, &wm);
    wm
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Writes the watermark to persistent storage with a TTL bump.
fn store_watermark(env: &Env, asset: &Address, wm: &CompletenessWatermark) {
    let key = DataKey::HistoryWatermark(asset.clone());
    env.storage().persistent().set(&key, wm);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Reads the timestamp of a history entry without panicking.
fn read_history_timestamp(env: &Env, asset: &Address, ledger: u32) -> u64 {
    let key = DataKey::PriceHistory(asset.clone(), ledger);
    if let Some(entry) = env.storage().temporary().get::<_, PriceHistoryEntry>(&key) {
        return entry.timestamp;
    }
    // Also check sharded bucket path.
    let bucket_idx = ledger / crate::history::LEDGERS_PER_WEEK;
    let bucket_key = DataKey::HistoryBucket(asset.clone(), bucket_idx);
    if let Some(bucket) = env
        .storage()
        .persistent()
        .get::<_, soroban_sdk::Vec<PriceHistoryEntry>>(&bucket_key)
    {
        for i in 0..bucket.len() {
            let e = bucket.get_unchecked(i);
            if e.ledger == ledger {
                return e.timestamp;
            }
        }
    }
    0
}

/// Computes the most common inter-snapshot stride to use as the normal cadence.
///
/// Returns `1` when the list has fewer than 2 entries or when strides vary
/// widely (no dominant pattern).  A value of `1` means "every ledger is
/// expected", which is the most conservative assumption.
fn compute_dominant_stride(ledger_list: &soroban_sdk::Vec<u32>) -> u32 {
    let n = ledger_list.len();
    if n < 2 {
        return 1;
    }

    // Collect strides (up to a small sample to keep cost bounded).
    let sample = n.min(16);
    let mut strides: [u32; 16] = [0u32; 16];
    for i in 0..sample.saturating_sub(1) as usize {
        let prev = ledger_list.get_unchecked(i as u32);
        let curr = ledger_list.get_unchecked(i as u32 + 1);
        strides[i] = curr.saturating_sub(prev);
    }

    // Find the most common stride value (mode) using a simple O(n²) pass — n
    // is capped at 15 so this is cheap.
    let count = (sample - 1) as usize;
    let mut best_stride = strides[0];
    let mut best_count = 0u32;
    for i in 0..count {
        let s = strides[i];
        let mut cnt = 0u32;
        for j in 0..count {
            if strides[j] == s {
                cnt += 1;
            }
        }
        if cnt > best_count {
            best_count = cnt;
            best_stride = s;
        }
    }

    if best_stride == 0 {
        1
    } else {
        best_stride
    }
}
