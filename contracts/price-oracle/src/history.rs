use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::admin::{get_interpolation_enabled, get_max_history_length};
use crate::storage::{
    check_registered_asset, read_registered_assets, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{
    CompactionMetadata, DataKey, ErrorCode, PriceHistoryEntry, StorageBudget, TotalStorageBudget,
};

// ─────────────────────────────────────────────────────────────────────────────
// #251 — History sharding constants
// ─────────────────────────────────────────────────────────────────────────────

/// Number of ledgers per week bucket used by history sharding (#251).
/// At ~5 seconds per ledger: 7 × 24 × 3600 / 5 = 120 960 ≈ 120 000.
pub const LEDGERS_PER_WEEK: u32 = 120_960;

/// Derive the week-bucket index from a ledger sequence number.
#[inline]
fn ledger_to_bucket(ledger: u32) -> u32 {
    ledger / LEDGERS_PER_WEEK
}

pub(crate) fn read_history_entry(
    env: &Env,
    asset: &Address,
    ledger: u32,
) -> Option<PriceHistoryEntry> {
    let key = DataKey::PriceHistory(asset.clone(), ledger);
    if let Some(entry) = env.storage().temporary().get(&key) {
        env.storage()
            .temporary()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return Some(entry);
    }

    let bucket_key = DataKey::HistoryBucket(asset.clone(), ledger_to_bucket(ledger));
    let bucket: Vec<PriceHistoryEntry> = env.storage().persistent().get(&bucket_key)?;
    for i in 0..bucket.len() {
        let entry = bucket.get_unchecked(i);
        if entry.ledger == ledger {
            env.storage()
                .persistent()
                .extend_ttl(&bucket_key, LEDGER_THRESHOLD, LEDGER_BUMP);
            return Some(entry);
        }
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// Existing history query functions (unchanged behaviour)
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_historical_price(env: &Env, asset: Address, ledger: u32) -> PriceHistoryEntry {
    check_registered_asset(env, &asset);

    // Exact match — return as-is.
    let key = DataKey::PriceHistory(asset.clone(), ledger);
    if env.storage().temporary().has(&key) {
        env.storage()
            .temporary()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return env.storage().temporary().get(&key).unwrap();
    }

    // Transparent shard fallback: search the correct week bucket.
    let bucket_idx = ledger_to_bucket(ledger);
    let bucket_key = DataKey::HistoryBucket(asset.clone(), bucket_idx);
    if env.storage().persistent().has(&bucket_key) {
        let bucket: Vec<PriceHistoryEntry> = env
            .storage()
            .persistent()
            .get(&bucket_key)
            .unwrap_or(Vec::new(env));
        for i in 0..bucket.len() {
            let e = bucket.get_unchecked(i);
            if e.ledger == ledger {
                return e;
            }
        }
    }

    // If interpolation is disabled, panic.
    if !get_interpolation_enabled(env) {
        panic_with_error!(env, ErrorCode::NoData);
    }

    // Find the nearest before/after entries via the ledger index.
    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    let mut before: Option<PriceHistoryEntry> = None;
    let mut after: Option<PriceHistoryEntry> = None;

    for i in 0..ledger_list.len() {
        let l = ledger_list.get_unchecked(i);
        if l <= ledger {
            if let Some(entry) = read_history_entry(env, &asset, l) {
                before = Some(entry);
            }
        } else if after.is_none() {
            if let Some(entry) = read_history_entry(env, &asset, l) {
                after = Some(entry);
            }
        }
    }

    match (before, after) {
        (Some(b), Some(a)) => {
            // Linear interpolation: price = b.price + (a.price - b.price) * (ledger - b.ledger) / (a.ledger - b.ledger)
            let range = (a.ledger - b.ledger) as i128;
            let offset = (ledger - b.ledger) as i128;
            let interpolated_price = b.price + (a.price - b.price) * offset / range;
            let interpolated_ts = b.timestamp
                + ((a.timestamp.saturating_sub(b.timestamp) as i128) * offset / range) as u64;
            PriceHistoryEntry {
                price: interpolated_price,
                timestamp: interpolated_ts,
                ledger,
                num_sources: 0,
                is_interpolated: true,
            }
        }
        _ => panic_with_error!(env, ErrorCode::NoData),
    }
}

pub fn has_historical_price(env: &Env, asset: Address, ledger: u32) -> bool {
    if !env
        .storage()
        .persistent()
        .has(&DataKey::AssetRegistered(asset.clone()))
    {
        return false;
    }
    let key = DataKey::PriceHistory(asset.clone(), ledger);
    if env.storage().temporary().has(&key) {
        env.storage()
            .temporary()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return true;
    }
    // Also check sharded bucket.
    let bucket_idx = ledger_to_bucket(ledger);
    let bucket_key = DataKey::HistoryBucket(asset.clone(), bucket_idx);
    if env.storage().persistent().has(&bucket_key) {
        let bucket: Vec<PriceHistoryEntry> = env
            .storage()
            .persistent()
            .get(&bucket_key)
            .unwrap_or(Vec::new(env));
        for i in 0..bucket.len() {
            let e = bucket.get_unchecked(i);
            if e.ledger == ledger {
                return true;
            }
        }
    }
    false
}

pub fn get_historical_prices(
    env: &Env,
    asset: Address,
    start_ledger: u32,
    end_ledger: u32,
) -> Vec<PriceHistoryEntry> {
    check_registered_asset(env, &asset);
    let max_range = get_max_history_length(env);
    if end_ledger < start_ledger || end_ledger - start_ledger > max_range {
        panic_with_error!(env, ErrorCode::NoData);
    }
    let mut entries: Vec<PriceHistoryEntry> = Vec::new(env);
    let mut ledger = start_ledger;
    while ledger <= end_ledger {
        if let Some(entry) = read_history_entry(env, &asset, ledger) {
            entries.push_back(entry);
        }
        ledger += 1;
    }
    entries
}

/// Maximum page size accepted by `get_historical_prices_paginated` (#229).
pub const MAX_PAGE_SIZE: u32 = 50;

/// Returns a cursor-paginated page of historical price entries for an asset.
///
/// `cursor` is the ledger sequence number to start from, inclusive (pass `0` to
/// start at the beginning). `limit` must be in `1..=MAX_PAGE_SIZE`. Entries are
/// returned in ascending ledger order, correctly skipping over gaps where no
/// snapshot was recorded.
///
/// Returns `(entries, next_cursor)`: `next_cursor` is `Some(ledger)` to pass as
/// the next page's `cursor`, or `None` once every recorded entry has been read.
///
/// # Errors
/// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
/// * [`ErrorCode::InvalidPageSize`] — if `limit` is `0` or exceeds `MAX_PAGE_SIZE`.
pub fn get_historical_prices_paginated(
    env: &Env,
    asset: Address,
    cursor: u32,
    limit: u32,
) -> (Vec<PriceHistoryEntry>, Option<u32>) {
    check_registered_asset(env, &asset);
    if limit == 0 || limit > MAX_PAGE_SIZE {
        panic_with_error!(env, ErrorCode::InvalidPageSize);
    }

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    let mut entries: Vec<PriceHistoryEntry> = Vec::new(env);
    let mut next_cursor: Option<u32> = None;

    for i in 0..ledger_list.len() {
        let l = ledger_list.get_unchecked(i);
        if l < cursor {
            continue;
        }
        if entries.len() >= limit {
            next_cursor = Some(l);
            break;
        }
        if let Some(entry) = read_history_entry(env, &asset, l) {
            entries.push_back(entry);
        }
    }

    (entries, next_cursor)
}

// ─────────────────────────────────────────────────────────────────────────────
// #247 — History Compaction
// ─────────────────────────────────────────────────────────────────────────────

/// Default compaction threshold: 0 bps (disabled by default).
pub const DEFAULT_COMPACTION_THRESHOLD_BPS: u32 = 0;

/// Reads the configured compaction threshold in basis points.
/// Returns 0 when not set (compaction disabled).
pub fn get_compaction_threshold_bps(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgCompactionThresholdBps)
        .unwrap_or(DEFAULT_COMPACTION_THRESHOLD_BPS)
}

/// Sets the compaction threshold in basis points. 0 disables compaction.
/// Requires admin authentication (checked by the caller in lib.rs).
pub fn set_compaction_threshold_bps(env: &Env, threshold_bps: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::CfgCompactionThresholdBps, &threshold_bps);
    env.storage().persistent().extend_ttl(
        &DataKey::CfgCompactionThresholdBps,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

/// Returns `true` if `candidate_price` is within `threshold_bps` of `reference_price`.
///
/// A threshold of 0 means every entry is retained (no compaction).
/// Uses saturating arithmetic to avoid overflow on large prices.
fn within_threshold(reference_price: i128, candidate_price: i128, threshold_bps: u32) -> bool {
    if threshold_bps == 0 || reference_price == 0 {
        return false;
    }
    let diff = if candidate_price > reference_price {
        candidate_price - reference_price
    } else {
        reference_price - candidate_price
    };
    // diff * 10_000 / reference <= threshold_bps
    // Use i128 arithmetic: diff * 10_000 <= threshold_bps * reference
    let lhs = diff.saturating_mul(10_000);
    let rhs = (threshold_bps as i128).saturating_mul(reference_price.abs());
    lhs <= rhs
}

/// On-write compaction helper: checks whether the most-recent history entry for
/// `asset` is within the compaction threshold of `candidate_price`. Returns `true`
/// if the new entry should be **skipped** (merged into the existing last entry).
///
/// Called from `prices.rs / aggregate_asset` before appending to the ledger list.
/// This is a non-destructive guard — it never removes existing data.
pub fn should_skip_on_write(env: &Env, asset: &Address, candidate_price: i128) -> bool {
    let threshold_bps = get_compaction_threshold_bps(env);
    if threshold_bps == 0 {
        return false;
    }
    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));
    if ledger_list.is_empty() {
        return false;
    }
    let last_ledger = ledger_list.get_unchecked(ledger_list.len() - 1);
    if last_ledger == env.ledger().sequence() {
        return false;
    }
    if let Some(last_entry) = read_history_entry(env, asset, last_ledger) {
        within_threshold(last_entry.price, candidate_price, threshold_bps)
    } else {
        false
    }
}

/// Admin-triggered on-demand compaction for a specific asset (#247).
///
/// Iterates the full history index for `asset` and removes entries whose price
/// deviates less than `threshold_bps` basis points from the preceding retained
/// entry. The first and last entries are always retained to preserve range bounds.
///
/// Returns [`CompactionMetadata`] describing the before/after counts.
///
/// # Panics
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin (checked by lib.rs).
pub fn compact_history(env: &Env, asset: Address) -> CompactionMetadata {
    check_registered_asset(env, &asset);

    let threshold_bps = get_compaction_threshold_bps(env);
    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    let original_count = ledger_list.len();

    // With fewer than 2 entries there is nothing to compact.
    if original_count <= 1 || threshold_bps == 0 {
        let meta = CompactionMetadata {
            original_count,
            compacted_count: original_count,
            last_compaction_ledger: env.ledger().sequence(),
            threshold_bps,
        };
        env.storage()
            .persistent()
            .set(&DataKey::CompactionMeta(asset.clone()), &meta);
        return meta;
    }

    // Build the compacted ledger list.  Always keep the first entry.
    let mut retained: Vec<u32> = Vec::new(env);
    let first_ledger = ledger_list.get_unchecked(0);
    retained.push_back(first_ledger);

    let mut last_kept_price: i128 = read_history_entry(env, &asset, first_ledger)
        .map(|e| e.price)
        .unwrap_or(0);

    // Walk the middle entries (skip first, handle last separately).
    for i in 1..original_count.saturating_sub(1) {
        let l = ledger_list.get_unchecked(i);
        if let Some(entry) = read_history_entry(env, &asset, l) {
            if within_threshold(last_kept_price, entry.price, threshold_bps) {
                // Merge: discard this entry.
                env.storage()
                    .temporary()
                    .remove(&DataKey::PriceHistory(asset.clone(), l));
                remove_history_shard_entry(env, &asset, l);
            } else {
                retained.push_back(l);
                last_kept_price = entry.price;
            }
        }
        // If the entry no longer exists (expired TTL), drop it from index silently.
    }

    // Always keep the last entry.
    let last_idx = original_count - 1;
    if last_idx > 0 {
        let last_ledger = ledger_list.get_unchecked(last_idx);
        // Avoid duplicating if original_count == 2 and last == first.
        if retained.len() == 0 || retained.get_unchecked(retained.len() - 1) != last_ledger {
            retained.push_back(last_ledger);
        }
    }

    let compacted_count = retained.len();
    env.storage().persistent().set(&ledgers_key, &retained);
    env.storage()
        .persistent()
        .extend_ttl(&ledgers_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let meta = CompactionMetadata {
        original_count,
        compacted_count,
        last_compaction_ledger: env.ledger().sequence(),
        threshold_bps,
    };
    env.storage()
        .persistent()
        .set(&DataKey::CompactionMeta(asset.clone()), &meta);
    env.storage().persistent().extend_ttl(
        &DataKey::CompactionMeta(asset.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
    meta
}

/// Returns the most recent compaction metadata for an asset, if any.
pub fn get_compaction_metadata(env: &Env, asset: Address) -> Option<CompactionMetadata> {
    env.storage()
        .persistent()
        .get(&DataKey::CompactionMeta(asset))
}

// ─────────────────────────────────────────────────────────────────────────────
// #251 — History Sharding (weekly time buckets)
// ─────────────────────────────────────────────────────────────────────────────

/// Writes a `PriceHistoryEntry` into the appropriate weekly shard bucket for an asset.
///
/// The bucket is identified by `ledger / LEDGERS_PER_WEEK`. This mirrors the
/// existing per-ledger temporary storage write so both paths co-exist during
/// the migration window. Consumers transparently benefit through
/// `get_historical_price` which checks both the legacy and shard paths.
///
/// Stored as persistent (not temporary) storage to survive TTL expirations on
/// individual entries — the bucket itself carries one expiry that covers the
/// entire weekly set.
pub fn write_history_shard(env: &Env, asset: &Address, entry: &PriceHistoryEntry) {
    let bucket_idx = ledger_to_bucket(entry.ledger);
    let bucket_key = DataKey::HistoryBucket(asset.clone(), bucket_idx);

    let mut bucket: Vec<PriceHistoryEntry> = env
        .storage()
        .persistent()
        .get(&bucket_key)
        .unwrap_or(Vec::new(env));

    // Avoid duplicate ledger entries within the same bucket.
    for i in 0..bucket.len() {
        if bucket.get_unchecked(i).ledger == entry.ledger {
            // Update in-place by rebuilding the vec (no indexed mutation in SDK).
            let mut updated: Vec<PriceHistoryEntry> = Vec::new(env);
            for j in 0..bucket.len() {
                if j == i {
                    updated.push_back(entry.clone());
                } else {
                    updated.push_back(bucket.get_unchecked(j));
                }
            }
            env.storage().persistent().set(&bucket_key, &updated);
            env.storage()
                .persistent()
                .extend_ttl(&bucket_key, LEDGER_THRESHOLD, LEDGER_BUMP);
            update_bucket_index(env, asset, bucket_idx);
            return;
        }
    }

    bucket.push_back(entry.clone());
    env.storage().persistent().set(&bucket_key, &bucket);
    env.storage()
        .persistent()
        .extend_ttl(&bucket_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    update_bucket_index(env, asset, bucket_idx);
}

pub(crate) fn remove_history_shard_entry(env: &Env, asset: &Address, ledger: u32) {
    let bucket_idx = ledger_to_bucket(ledger);
    let bucket_key = DataKey::HistoryBucket(asset.clone(), bucket_idx);
    let bucket: Vec<PriceHistoryEntry> = match env.storage().persistent().get(&bucket_key) {
        Some(bucket) => bucket,
        None => return,
    };
    let mut retained: Vec<PriceHistoryEntry> = Vec::new(env);
    for i in 0..bucket.len() {
        let entry = bucket.get_unchecked(i);
        if entry.ledger != ledger {
            retained.push_back(entry);
        }
    }
    if retained.is_empty() {
        env.storage().persistent().remove(&bucket_key);
        remove_bucket_index(env, asset, bucket_idx);
    } else {
        env.storage().persistent().set(&bucket_key, &retained);
        env.storage()
            .persistent()
            .extend_ttl(&bucket_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

/// Ensures `bucket_idx` appears in the asset's shard bucket index.
fn update_bucket_index(env: &Env, asset: &Address, bucket_idx: u32) {
    let idx_key = DataKey::HistoryBucketIndex(asset.clone());
    let mut index: Vec<u32> = env
        .storage()
        .persistent()
        .get(&idx_key)
        .unwrap_or(Vec::new(env));
    // Keep the index sorted and duplicate-free.
    for i in 0..index.len() {
        if index.get_unchecked(i) == bucket_idx {
            return; // already present
        }
    }
    index.push_back(bucket_idx);
    env.storage().persistent().set(&idx_key, &index);
    env.storage()
        .persistent()
        .extend_ttl(&idx_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn remove_bucket_index(env: &Env, asset: &Address, bucket_idx: u32) {
    let idx_key = DataKey::HistoryBucketIndex(asset.clone());
    let index: Vec<u32> = env
        .storage()
        .persistent()
        .get(&idx_key)
        .unwrap_or(Vec::new(env));
    let mut retained: Vec<u32> = Vec::new(env);
    for i in 0..index.len() {
        let value = index.get_unchecked(i);
        if value != bucket_idx {
            retained.push_back(value);
        }
    }
    if retained.is_empty() {
        env.storage().persistent().remove(&idx_key);
    } else {
        env.storage().persistent().set(&idx_key, &retained);
    }
}

/// Prunes all shard buckets whose week index falls before `min_bucket_idx`.
///
/// Called from the history pruning loop in `prices.rs` after the existing
/// per-ledger pruning, allowing the two mechanisms to interoperate.
pub fn prune_old_buckets(env: &Env, asset: &Address, min_bucket_idx: u32) {
    let idx_key = DataKey::HistoryBucketIndex(asset.clone());
    let index: Vec<u32> = env
        .storage()
        .persistent()
        .get(&idx_key)
        .unwrap_or(Vec::new(env));
    if index.is_empty() {
        return;
    }
    let mut retained: Vec<u32> = Vec::new(env);
    for i in 0..index.len() {
        let b = index.get_unchecked(i);
        if b < min_bucket_idx {
            env.storage()
                .persistent()
                .remove(&DataKey::HistoryBucket(asset.clone(), b));
        } else {
            retained.push_back(b);
        }
    }
    env.storage().persistent().set(&idx_key, &retained);
    if !retained.is_empty() {
        env.storage()
            .persistent()
            .extend_ttl(&idx_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

/// Migrates all existing per-ledger temporary history entries for `asset` into
/// sharded persistent buckets without removing the originals.
///
/// This is a non-destructive migration: legacy reads continue to work normally
/// while shard reads become available immediately after. Run once per asset to
/// populate the shard layer for existing contracts.
///
/// Returns the number of entries migrated.
pub fn migrate_history_to_shards(env: &Env, asset: Address) -> u32 {
    check_registered_asset(env, &asset);
    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    let mut migrated: u32 = 0;
    for i in 0..ledger_list.len() {
        let l = ledger_list.get_unchecked(i);
        let key = DataKey::PriceHistory(asset.clone(), l);
        if let Some(entry) = env.storage().temporary().get::<_, PriceHistoryEntry>(&key) {
            write_history_shard(env, &asset, &entry);
            migrated += 1;
        }
    }
    migrated
}

/// Returns all entries from the weekly bucket containing `ledger`.
///
/// Transparent to consumers — they do not need to know which bucket to query.
pub fn get_bucket_entries(env: &Env, asset: Address, ledger: u32) -> Vec<PriceHistoryEntry> {
    check_registered_asset(env, &asset);
    let bucket_idx = ledger_to_bucket(ledger);
    let bucket_key = DataKey::HistoryBucket(asset, bucket_idx);
    env.storage()
        .persistent()
        .get(&bucket_key)
        .unwrap_or(Vec::new(env))
}

// ─────────────────────────────────────────────────────────────────────────────
// #253 — Storage Budget Calculator
// ─────────────────────────────────────────────────────────────────────────────

/// Bytes per history entry: rough estimate for a `PriceHistoryEntry` XDR encoding.
/// price(16) + timestamp(8) + ledger(4) + num_sources(4) + is_interpolated(1) + overhead(32) ≈ 65
const BYTES_PER_HISTORY_ENTRY: u32 = 96;

/// Soroban ledger-entry write fee per 1 KB in stroops (approximate network value).
/// This is an illustrative constant; the real cost depends on network configuration.
/// 1 stroop = 10^-7 XLM. At ~4000 stroops/KB/month this gives reasonable estimates.
const STROOP_PER_KB_PER_MONTH: i128 = 4_000;

/// Computes estimated storage budget for a single asset.
///
/// `entry_count` is read from the persistent ledger index. Costs are rough
/// estimates suitable for planning purposes, not exact billing.
///
/// # Panics
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
pub fn get_storage_budget(env: &Env, asset: Address) -> StorageBudget {
    check_registered_asset(env, &asset);

    // Count entries from the persistent ledger index.
    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let entry_count: u32 = env
        .storage()
        .persistent()
        .get::<_, Vec<u32>>(&ledgers_key)
        .map(|v| v.len())
        .unwrap_or(0);

    // The legacy ledger index is the logical source of truth while migration
    // dual-writes entries to both legacy and sharded storage.
    let total_entries = entry_count;

    let estimated_bytes = total_entries.saturating_mul(BYTES_PER_HISTORY_ENTRY);
    let estimated_kb = (estimated_bytes as i128).saturating_add(1023) / 1024; // ceil

    // Monthly TTL cost estimate: KB × stroops/KB/month
    let estimated_ttl_costs = estimated_kb.saturating_mul(STROOP_PER_KB_PER_MONTH);
    // Project total monthly cost including a small write-overhead factor (1.2×)
    let projected_monthly_cost = estimated_ttl_costs.saturating_add(estimated_ttl_costs / 5); // +20% for write/read ops

    StorageBudget {
        asset,
        entry_count: total_entries,
        estimated_ttl_costs,
        projected_monthly_cost,
        estimated_bytes,
    }
}

/// Aggregates storage budgets across all registered assets.
///
/// Iterates every registered asset and sums their individual budgets.
/// For contracts with many assets, this may be expensive; use per-asset
/// `get_storage_budget` for targeted queries.
pub fn get_total_storage_budget(env: &Env) -> TotalStorageBudget {
    let assets = read_registered_assets(env);
    let asset_count = assets.len();

    let mut total_entry_count: u32 = 0;
    let mut total_ttl_costs: i128 = 0;
    let mut total_monthly_cost: i128 = 0;
    let mut total_bytes: u32 = 0;

    for i in 0..asset_count {
        let asset = assets.get_unchecked(i);
        // Skip unregistered / mid-deletion assets gracefully.
        let index_key = DataKey::AssetRegistryIndex(asset.clone());
        let registered: bool = env.storage().persistent().get(&index_key).unwrap_or(false);
        if !registered {
            let legacy: bool = env
                .storage()
                .persistent()
                .get(&DataKey::AssetRegistered(asset.clone()))
                .unwrap_or(false);
            if !legacy {
                continue;
            }
        }
        let budget = get_storage_budget(env, asset);
        total_entry_count = total_entry_count.saturating_add(budget.entry_count);
        total_ttl_costs = total_ttl_costs.saturating_add(budget.estimated_ttl_costs);
        total_monthly_cost = total_monthly_cost.saturating_add(budget.projected_monthly_cost);
        total_bytes = total_bytes.saturating_add(budget.estimated_bytes);
    }

    TotalStorageBudget {
        asset_count,
        total_entry_count,
        total_estimated_ttl_costs: total_ttl_costs,
        total_projected_monthly_cost: total_monthly_cost,
        total_estimated_bytes: total_bytes,
    }
}
