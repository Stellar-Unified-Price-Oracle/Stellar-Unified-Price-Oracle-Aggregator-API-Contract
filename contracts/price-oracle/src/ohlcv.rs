//! # OHLCV Aggregation
//!
//! Aggregates the asset's indexed price-history snapshots ([`crate::history`],
//! backed by `DataKey::PriceHistory` / `DataKey::PriceHistoryLedgers`) into
//! configurable open/high/low/close (OHLCV) time buckets for charting and
//! downstream analytics, instead of forcing consumers to replay raw per-ledger
//! entries themselves.
//!
//! ## Bucketing
//!
//! Buckets are aligned to `bucket_seconds` using integer division on each
//! snapshot's Unix timestamp: `bucket_start = (timestamp / bucket_seconds) *
//! bucket_seconds`. Common values: `3600` (hourly), `86400` (daily).
//!
//! ## Caching
//!
//! A bar is only immutable once its bucket has fully elapsed — the bucket
//! containing the current ledger time may still receive new snapshots. `get_ohlcv`
//! therefore write-through caches every **closed** bar it computes under
//! `DataKey::OhlcvBar(asset, bucket_seconds, bucket_start)` (indexed by
//! `DataKey::OhlcvBucketIndex`) and always recomputes the in-progress bucket.
//! Once cached, a closed bar can be read in O(1) via [`get_cached_ohlcv_bar`]
//! without rescanning the asset's full history.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::history::read_history_entry;
use crate::storage::{check_registered_asset, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, OhlcvBar};

/// Upper bound on the number of bars returned by a single `get_ohlcv` call, to
/// keep worst-case compute bounded regardless of the requested range.
pub const MAX_OHLCV_BARS: u32 = 500;

fn bucket_start_for(timestamp: u64, bucket_seconds: u64) -> u64 {
    (timestamp / bucket_seconds) * bucket_seconds
}

fn write_cache(env: &Env, asset: &Address, bucket_seconds: u64, bar: &OhlcvBar) {
    let key = DataKey::OhlcvBar(asset.clone(), bucket_seconds, bar.bucket_start);
    env.storage().persistent().set(&key, bar);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let index_key = DataKey::OhlcvBucketIndex(asset.clone(), bucket_seconds);
    let mut index: Vec<u64> = env
        .storage()
        .persistent()
        .get(&index_key)
        .unwrap_or_else(|| Vec::new(env));
    for i in 0..index.len() {
        if index.get_unchecked(i) == bar.bucket_start {
            env.storage()
                .persistent()
                .extend_ttl(&index_key, LEDGER_THRESHOLD, LEDGER_BUMP);
            return;
        }
    }
    index.push_back(bar.bucket_start);
    env.storage().persistent().set(&index_key, &index);
    env.storage()
        .persistent()
        .extend_ttl(&index_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns a cached, previously closed OHLCV bar for
/// `(asset, bucket_seconds, bucket_start)`, or `None` if it was never computed
/// (e.g. no history fell in that bucket, or `get_ohlcv` has not been called for
/// a range covering it yet).
pub fn get_cached_ohlcv_bar(
    env: &Env,
    asset: Address,
    bucket_seconds: u64,
    bucket_start: u64,
) -> Option<OhlcvBar> {
    let key = DataKey::OhlcvBar(asset, bucket_seconds, bucket_start);
    let result = env.storage().persistent().get(&key);
    if result.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    result
}

/// Returns OHLCV bars for `asset` covering `[from_ts, to_ts]`, bucketed into
/// `bucket_seconds`-wide windows.
///
/// Scans the asset's recorded price-history snapshots once, grouping them into
/// bars in chronological order. Every bar whose bucket has fully elapsed is
/// written through to the cache (see module docs); the caller can subsequently
/// fetch it directly via [`get_cached_ohlcv_bar`] without rescanning history.
///
/// # Panics
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::InvalidOhlcvRange`] — `bucket_seconds == 0` or `to_ts < from_ts`.
pub fn get_ohlcv(
    env: &Env,
    asset: Address,
    bucket_seconds: u64,
    from_ts: u64,
    to_ts: u64,
) -> Vec<OhlcvBar> {
    check_registered_asset(env, &asset);
    if bucket_seconds == 0 || to_ts < from_ts {
        panic_with_error!(env, ErrorCode::InvalidOhlcvRange);
    }

    let current_bucket = bucket_start_for(env.ledger().timestamp(), bucket_seconds);

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or_else(|| Vec::new(env));

    let mut bars: Vec<OhlcvBar> = Vec::new(env);
    let mut current: Option<OhlcvBar> = None;

    for i in 0..ledger_list.len() {
        if bars.len() >= MAX_OHLCV_BARS {
            break;
        }
        let ledger = ledger_list.get_unchecked(i);
        let entry = match read_history_entry(env, &asset, ledger) {
            Some(e) => e,
            None => continue,
        };
        if entry.timestamp < from_ts || entry.timestamp > to_ts {
            continue;
        }

        let bucket_start = bucket_start_for(entry.timestamp, bucket_seconds);
        let same_bucket = current
            .as_ref()
            .map(|bar| bar.bucket_start == bucket_start)
            .unwrap_or(false);

        if same_bucket {
            let bar = current.as_mut().unwrap();
            if entry.price > bar.high {
                bar.high = entry.price;
            }
            if entry.price < bar.low {
                bar.low = entry.price;
            }
            bar.close = entry.price;
            bar.sample_count += 1;
        } else {
            if let Some(prev) = current.take() {
                if prev.bucket_start < current_bucket {
                    write_cache(env, &asset, bucket_seconds, &prev);
                }
                bars.push_back(prev);
            }
            current = Some(OhlcvBar {
                bucket_start,
                open: entry.price,
                high: entry.price,
                low: entry.price,
                close: entry.price,
                sample_count: 1,
            });
        }
    }

    if let Some(prev) = current {
        if prev.bucket_start < current_bucket {
            write_cache(env, &asset, bucket_seconds, &prev);
        }
        bars.push_back(prev);
    }

    bars
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_helpers::{
        ledger_default, register_test_asset, register_test_source, setup_contract,
    };

    #[test]
    fn test_ohlcv_buckets_hourly() {
        let env = Env::default();
        ledger_default(&env, 1, 0);
        let (client, _admin) = setup_contract(&env);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&env, &client, "Source");
        let asset = register_test_asset(&env, &client);

        // Three submissions in the first hour, two in the second.
        client.submit_price(&source, &asset, &100i128, &0u64, &1u64);
        client.submit_price(&source, &asset, &110i128, &1_000u64, &2u64);
        client.submit_price(&source, &asset, &90i128, &3_000u64, &3u64);
        client.submit_price(&source, &asset, &200i128, &3_700u64, &4u64);
        client.submit_price(&source, &asset, &210i128, &7_100u64, &5u64);

        ledger_default(&env, 10, 10_000);
        let bars = get_ohlcv(&env, asset, 3_600, 0, 7_200);
        assert_eq!(bars.len(), 2);

        let first = bars.get_unchecked(0);
        assert_eq!(first.bucket_start, 0);
        assert_eq!(first.open, 100);
        assert_eq!(first.high, 110);
        assert_eq!(first.low, 90);
        assert_eq!(first.close, 90);
        assert_eq!(first.sample_count, 3);

        let second = bars.get_unchecked(1);
        assert_eq!(second.bucket_start, 3_600);
        assert_eq!(second.open, 200);
        assert_eq!(second.close, 210);
        assert_eq!(second.sample_count, 2);
    }

    #[test]
    fn test_closed_bucket_is_cached() {
        let env = Env::default();
        ledger_default(&env, 1, 0);
        let (client, _admin) = setup_contract(&env);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&env, &client, "Source");
        let asset = register_test_asset(&env, &client);

        client.submit_price(&source, &asset, &100i128, &0u64, &1u64);
        client.submit_price(&source, &asset, &105i128, &10u64, &2u64);

        // Advance well past the bucket so it is considered closed.
        ledger_default(&env, 10, 100_000);
        let bars = get_ohlcv(&env, asset.clone(), 3_600, 0, 3_600);
        assert_eq!(bars.len(), 1);

        let cached = get_cached_ohlcv_bar(&env, asset, 3_600, 0);
        assert!(cached.is_some());
        assert_eq!(cached.unwrap().close, 105);
    }

    #[test]
    #[should_panic]
    fn test_zero_bucket_seconds_panics() {
        let env = Env::default();
        ledger_default(&env, 1, 0);
        let (client, _admin) = setup_contract(&env);
        let asset = register_test_asset(&env, &client);
        get_ohlcv(&env, asset, 0, 0, 100);
    }
}
