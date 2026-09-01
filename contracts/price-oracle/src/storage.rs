use crate::types::{DataKey, ErrorCode, OracleSources, SubscriptionPlans};
use soroban_sdk::{panic_with_error, Address, Env, Map, Vec};

// Keep frequently accessed contract entries alive longer to reduce TTL bump traffic
// on hot paths such as admin/config/registry lookups.
pub const LEDGER_THRESHOLD: u32 = 10_000;
pub const LEDGER_BUMP: u32 = 40_000;
pub const DEFAULT_QUERY_RATE_LIMIT: u32 = 100;

/// Alias used by public getter wrappers in `lib.rs`.
pub use crate::reentrancy::{enter as enter_reentrancy_guard, exit as exit_reentrancy_guard};

/// Default number of ledgers after creation before a pending operation expires (~24 h at 5 s/ledger).
pub const DEFAULT_EXPIRY_LEDGERS: u32 = 17_280;

pub fn get_admin(env: &Env) -> Address {
    env.storage().persistent().get(&DataKey::Admin).unwrap()
}

pub fn check_source(env: &Env, addr: &Address) {
    let key = DataKey::SrcActive(addr.clone());
    let is_source: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if !is_source {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let required_bond = crate::sources::get_source_bond(env);
    if required_bond > 0 {
        let deposited = crate::sources::get_source_deposited_bond(env, addr.clone());
        if deposited < required_bond {
            panic_with_error!(env, ErrorCode::InsufficientBond);
        }
    }

    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn check_registered_asset(env: &Env, asset: &Address) {
    // Prefer the O(1) membership index.
    let index_key = DataKey::AssetRegistryIndex(asset.clone());
    let indexed: bool = env.storage().persistent().get(&index_key).unwrap_or(false);
    if indexed {
        env.storage()
            .persistent()
            .extend_ttl(&index_key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return;
    }

    // Backward compatibility: older deployments only have the legacy
    // `AssetRegistered(asset)` flag. If it exists, lazily (re)build
    // the index entry.
    let legacy_key = DataKey::AssetRegistered(asset.clone());
    let exists: bool = env.storage().persistent().get(&legacy_key).unwrap_or(false);
    if !exists {
        panic_with_error!(env, ErrorCode::AssetNotRegistered);
    }

    env.storage()
        .persistent()
        .extend_ttl(&legacy_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    env.storage().persistent().set(&index_key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&index_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn check_source_asset(env: &Env, source: &Address, asset: &Address) {
    let key = DataKey::SourceAssets(source.clone());
    let assets: Option<Vec<Address>> = env.storage().persistent().get(&key);
    if let Some(assets) = assets {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        for i in 0..assets.len() {
            if assets.get_unchecked(i) == *asset {
                return;
            }
        }
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
}

/// Sort prices using heapsort — guaranteed O(n log n) worst-case, O(1) extra space.
/// Used by `compute_trimmed_mean` which needs a fully sorted array.
pub fn sort_prices(prices: &mut soroban_sdk::Vec<i128>) {
    let n = prices.len();
    if n <= 1 {
        return;
    }
    // Build max-heap
    let mut i = n / 2;
    loop {
        heapify(prices, n, i);
        if i == 0 {
            break;
        }
        i -= 1;
    }
    // Extract elements from heap one by one
    let mut end = n - 1;
    loop {
        let tmp = prices.get_unchecked(0);
        prices.set(0, prices.get_unchecked(end));
        prices.set(end, tmp);
        heapify(prices, end, 0);
        if end == 0 {
            break;
        }
        end -= 1;
    }
}

/// Sift down the element at `root` within a heap of size `n` (iterative, no stack growth).
fn heapify(prices: &mut soroban_sdk::Vec<i128>, n: u32, root: u32) {
    let mut current = root;
    loop {
        let mut largest = current;
        let left = 2 * current + 1;
        let right = 2 * current + 2;
        if left < n && prices.get_unchecked(left) > prices.get_unchecked(largest) {
            largest = left;
        }
        if right < n && prices.get_unchecked(right) > prices.get_unchecked(largest) {
            largest = right;
        }
        if largest == current {
            break;
        }
        let tmp = prices.get_unchecked(current);
        prices.set(current, prices.get_unchecked(largest));
        prices.set(largest, tmp);
        current = largest;
    }
}

fn vec_swap(prices: &mut soroban_sdk::Vec<i128>, i: u32, j: u32) {
    if i == j {
        return;
    }
    let tmp = prices.get_unchecked(i);
    prices.set(i, prices.get_unchecked(j));
    prices.set(j, tmp);
}

fn median_of_five(prices: &mut soroban_sdk::Vec<i128>, left: u32, right: u32) -> u32 {
    let mut i = left + 1;
    while i <= right {
        let mut j = i;
        while j > left && prices.get_unchecked(j) < prices.get_unchecked(j - 1) {
            vec_swap(prices, j, j - 1);
            j -= 1;
        }
        i += 1;
    }
    left + (right - left) / 2
}

fn partition(prices: &mut soroban_sdk::Vec<i128>, left: u32, right: u32, pivot_index: u32) -> u32 {
    let pivot_value = prices.get_unchecked(pivot_index);
    vec_swap(prices, pivot_index, right);
    let mut store_index = left;
    let mut i = left;
    while i < right {
        if prices.get_unchecked(i) < pivot_value {
            vec_swap(prices, store_index, i);
            store_index += 1;
        }
        i += 1;
    }
    vec_swap(prices, store_index, right);
    store_index
}

fn select_pivot(prices: &mut soroban_sdk::Vec<i128>, left: u32, right: u32) -> u32 {
    let n = right - left + 1;
    if n <= 5 {
        return median_of_five(prices, left, right);
    }
    let mut store = left;
    let mut i = left;
    while i <= right {
        let group_end = if i + 4 <= right { i + 4 } else { right };
        let median = median_of_five(prices, i, group_end);
        vec_swap(prices, median, store);
        store += 1;
        i += 5;
    }
    let mid = left + ((store - left - 1) / 2);
    select_kth(prices, left, store - 1, mid)
}

fn select_kth(prices: &mut soroban_sdk::Vec<i128>, mut left: u32, mut right: u32, k: u32) -> i128 {
    loop {
        if left == right {
            return prices.get_unchecked(left);
        }
        let pivot_index = select_pivot(prices, left, right);
        let pivot_index = partition(prices, left, right, pivot_index);
        if k == pivot_index {
            return prices.get_unchecked(k);
        } else if k < pivot_index {
            right = pivot_index - 1;
        } else {
            left = pivot_index + 1;
        }
    }
}

pub fn compute_median(prices: &soroban_sdk::Vec<i128>) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    let mut selected = prices.clone();
    let mid = n / 2;
    if n.is_multiple_of(2) {
        let lower = select_kth(&mut selected, 0, n - 1, mid - 1);
        let upper = select_kth(&mut selected, 0, n - 1, mid);
        lower + (upper - lower) / 2
    } else {
        select_kth(&mut selected, 0, n - 1, mid)
    }
}

pub fn compute_mean(prices: &soroban_sdk::Vec<i128>) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    let mut sum: i128 = 0;
    for i in 0..n {
        sum = sum.saturating_add(prices.get_unchecked(i));
    }
    sum / (n as i128)
}

fn integer_sqrt(value: i128) -> i128 {
    if value <= 1 {
        return value;
    }

    let mut lo = 0i128;
    let mut hi = value;
    while lo + 1 < hi {
        let mid = lo + (hi - lo) / 2;
        if mid.saturating_mul(mid) <= value {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo
}

pub fn compute_stddev(prices: &soroban_sdk::Vec<i128>) -> u32 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }

    let mean = compute_mean(prices);
    let mut sum_sq: i128 = 0;
    for i in 0..n {
        let diff = prices.get_unchecked(i) - mean;
        sum_sq = sum_sq.saturating_add(diff.saturating_mul(diff));
    }

    let variance = sum_sq / (n as i128);
    integer_sqrt(variance).max(0) as u32
}

pub fn compute_confidence_bps(prices: &soroban_sdk::Vec<i128>) -> u32 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }

    let mean = compute_mean(prices);
    if mean <= 0 {
        return 0;
    }

    let stddev = compute_stddev(prices) as u128;
    ((stddev.saturating_mul(10000u128)) / (mean as u128)) as u32
}

pub fn compute_trimmed_mean(prices: &soroban_sdk::Vec<i128>, trim_percent: u32) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    if trim_percent == 0 {
        return compute_mean(prices);
    }

    let mut sorted = prices.clone();
    sort_prices(&mut sorted);

    let trim_count = ((n.saturating_mul(trim_percent) / 100) / 2).min(n - 1);
    if trim_count == 0 {
        return compute_mean(&sorted);
    }

    let mut trimmed: soroban_sdk::Vec<i128> = soroban_sdk::Vec::new(prices.env());
    for i in trim_count..(n - trim_count) {
        trimmed.push_back(sorted.get_unchecked(i));
    }

    if trimmed.is_empty() {
        return sorted.get_unchecked(n / 2);
    }

    compute_mean(&trimmed)
}

pub fn compute_vwap(prices: &soroban_sdk::Vec<i128>, volumes: &soroban_sdk::Vec<i128>) -> i128 {
    let n = prices.len().min(volumes.len());
    if n == 0 {
        return 0;
    }

    let mut weighted_sum: i128 = 0;
    let mut total_volume: i128 = 0;
    for i in 0..n {
        let volume = volumes.get_unchecked(i);
        if volume <= 0 {
            continue;
        }
        weighted_sum = weighted_sum.saturating_add(prices.get_unchecked(i).saturating_mul(volume));
        total_volume = total_volume.saturating_add(volume);
    }

    if total_volume == 0 {
        compute_mean(prices)
    } else {
        weighted_sum / total_volume
    }
}

pub fn read_registered_assets(env: &Env) -> Vec<Address> {
    let key = DataKey::AssetRegistry;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn write_registered_assets(env: &Env, assets: &Vec<Address>) {
    env.storage()
        .persistent()
        .set(&DataKey::AssetRegistry, assets);
}

pub fn read_oracle_sources(env: &Env) -> OracleSources {
    let key = DataKey::SrcRegistry;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(OracleSources {
            sources: soroban_sdk::Vec::new(env),
            metadata: soroban_sdk::Map::new(env),
            verification: soroban_sdk::Map::new(env),
        })
}

pub fn is_source_inactive(env: &Env, source: &Address) -> bool {
    let key = DataKey::SrcInactive(source.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn mark_source_inactive(env: &Env, source: &Address) {
    let key = DataKey::SrcInactive(source.clone());
    env.storage().persistent().set(&key, &true);
}

pub fn mark_source_active(env: &Env, source: &Address) {
    let key = DataKey::SrcInactive(source.clone());
    env.storage().persistent().remove(&key);
}

/// Not currently wired into `get_price` — kept for a future rate-limiting pass.
#[allow(dead_code)]
pub fn check_rate_limit(env: &Env, consumer: &Address) -> bool {
    let ledger = env.ledger().sequence();
    let key = DataKey::QueryCount(consumer.clone(), ledger);
    let count: u32 = env.storage().temporary().get(&key).unwrap_or(0);
    let rate_limit_key = DataKey::QueryRateLimit;
    let max_queries: u32 = env
        .storage()
        .persistent()
        .get(&rate_limit_key)
        .unwrap_or(DEFAULT_QUERY_RATE_LIMIT);
    count < max_queries
}

/// Not currently wired into `get_price` — kept for a future rate-limiting pass.
#[allow(dead_code)]
pub fn increment_query_count(env: &Env, consumer: &Address) -> u32 {
    let ledger = env.ledger().sequence();
    let key = DataKey::QueryCount(consumer.clone(), ledger);
    let count: u32 = env.storage().temporary().get(&key).unwrap_or(0);
    let new_count = count + 1;
    env.storage().temporary().set(&key, &new_count);
    env.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    new_count
}

pub fn read_subscription_expiry(env: &Env, consumer: &Address) -> Option<u64> {
    let key = DataKey::SubscriptionExpiry(consumer.clone());
    env.storage().persistent().get(&key)
}

pub fn write_subscription_expiry(env: &Env, consumer: &Address, expiry: u64) {
    let key = DataKey::SubscriptionExpiry(consumer.clone());
    env.storage().persistent().set(&key, &expiry);
}

pub fn read_subscription_plans(env: &Env) -> SubscriptionPlans {
    let key = DataKey::SubscriptionPlans;
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Map::new(env))
}

pub fn write_subscription_plans(env: &Env, plans: &SubscriptionPlans) {
    let key = DataKey::SubscriptionPlans;
    env.storage().persistent().set(&key, plans);
}

pub fn get_plan_amount(env: &Env, duration: u32) -> Option<i128> {
    let plans = read_subscription_plans(env);
    plans.get(duration)
}

/// Only reachable today via `check_rate_limit_and_increment`, which is itself unwired.
#[allow(dead_code)]
pub fn is_subscribed(env: &Env, consumer: &Address) -> bool {
    let key = DataKey::SubscriptionExpiry(consumer.clone());
    let expiry: u64 = env.storage().persistent().get(&key).unwrap_or(0);
    if expiry > 0 {
        let ledger_timestamp = env.ledger().timestamp();
        expiry > ledger_timestamp
    } else {
        false
    }
}

/// Gather TTL status for known storage entries. Exact remaining TTL values
/// are not exposed by the Soroban storage API; return `0` when unavailable.
pub fn get_storage_ttl_status(env: &Env) -> soroban_sdk::Vec<crate::types::StorageTtlEntry> {
    let mut out: soroban_sdk::Vec<crate::types::StorageTtlEntry> = soroban_sdk::Vec::new(env);

    // Assets registry
    let asset_key = DataKey::AssetRegistry;
    let asset_exists = env.storage().persistent().has(&asset_key);
    out.push_back(crate::types::StorageTtlEntry {
        key: soroban_sdk::String::from_str(env, "AssetRegistry"),
        exists: asset_exists,
        remaining_ttl: 0,
    });

    // Oracle sources registry
    let src_key = DataKey::SrcRegistry;
    let src_exists = env.storage().persistent().has(&src_key);
    out.push_back(crate::types::StorageTtlEntry {
        key: soroban_sdk::String::from_str(env, "SrcRegistry"),
        exists: src_exists,
        remaining_ttl: 0,
    });

    // For each registered asset, report aggregate existence and history entries.
    let assets = read_registered_assets(env);
    for i in 0..assets.len() {
        let a = assets.get_unchecked(i);
        let agg_key = DataKey::Aggregate(a.clone());
        let exists = env.storage().persistent().has(&agg_key);
        out.push_back(crate::types::StorageTtlEntry {
            key: soroban_sdk::String::from_str(env, &format!("Aggregate({})", i)),
            exists,
            remaining_ttl: 0,
        });

        // Price history ledgers list (if present)
        let ledgers_key = DataKey::PriceHistoryLedgers(a.clone());
        if env.storage().persistent().has(&ledgers_key) {
            let ledger_list: Option<soroban_sdk::Vec<u32>> =
                env.storage().persistent().get(&ledgers_key);
            if let Some(list) = ledger_list {
                for j in 0..list.len() {
                    let ledger = list.get_unchecked(j);
                    let hist_key = DataKey::PriceHistory(a.clone(), ledger);
                    let exists_hist = env.storage().temporary().has(&hist_key);
                    out.push_back(crate::types::StorageTtlEntry {
                        key: soroban_sdk::String::from_str(
                            env,
                            &format!("PriceHistory({}, {})", i, ledger),
                        ),
                        exists: exists_hist,
                        remaining_ttl: 0,
                    });
                }
            }
        }
    }

    out
}
