use crate::types::{DataKey, ErrorCode, OracleSources};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

pub const LEDGER_THRESHOLD: u32 = 1000;
pub const LEDGER_BUMP: u32 = 4000;

pub fn get_admin(env: &Env) -> Address {
    env.storage().persistent().get(&DataKey::Admin).unwrap()
}

pub fn check_source(env: &Env, addr: &Address) {
    let key = DataKey::SrcActive(addr.clone());
    let is_source: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if !is_source {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn check_registered_asset(env: &Env, asset: &Address) {
    let key = DataKey::AssetRegistered(asset.clone());
    let is_registered: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if !is_registered {
        panic_with_error!(env, ErrorCode::AssetNotRegistered);
    }
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Sort prices using heapsort — guaranteed O(n log n) worst-case, O(1) extra space.
/// Preferred over quicksort to avoid O(n²) worst-case gas cost on adversarial inputs.
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

pub fn compute_median(prices: &soroban_sdk::Vec<i128>) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    let mut sorted = prices.clone();
    sort_prices(&mut sorted);
    if n.is_multiple_of(2) {
        let mid = n / 2;
        let a = sorted.get_unchecked(mid - 1);
        let b = sorted.get_unchecked(mid);
        a + (b - a) / 2
    } else {
        sorted.get_unchecked(n / 2)
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
