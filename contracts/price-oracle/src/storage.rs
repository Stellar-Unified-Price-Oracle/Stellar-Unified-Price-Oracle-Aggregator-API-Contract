use crate::types::{DataKey, ErrorCode, OracleSources};
use soroban_sdk::{panic_with_error, Address, Env, Vec};

pub const LEDGER_THRESHOLD: u32 = 1000;
pub const LEDGER_BUMP: u32 = 4000;

pub fn get_admin(env: &Env) -> Address {
    env.storage().persistent().get(&DataKey::Admin).unwrap()
}

pub fn check_source(env: &Env, addr: &Address) {
    let key = DataKey::Source(addr.clone());
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

pub fn sort_prices(prices: &mut soroban_sdk::Vec<i128>) {
    let n = prices.len();
    if n <= 1 {
        return;
    }
    quicksort(prices, 0, n - 1);
}

fn quicksort(prices: &mut soroban_sdk::Vec<i128>, low: u32, high: u32) {
    if low < high {
        let pi = partition(prices, low, high);
        if pi > 0 {
            quicksort(prices, low, pi - 1);
        }
        quicksort(prices, pi + 1, high);
    }
}

fn partition(prices: &mut soroban_sdk::Vec<i128>, low: u32, high: u32) -> u32 {
    let pivot = prices.get_unchecked(high);
    let mut i = low;
    let mut j = low;
    while j < high {
        if prices.get_unchecked(j) <= pivot {
            let tmp = prices.get_unchecked(i);
            prices.set(i, prices.get_unchecked(j));
            prices.set(j, tmp);
            i += 1;
        }
        j += 1;
    }
    let tmp = prices.get_unchecked(i);
    prices.set(i, prices.get_unchecked(high));
    prices.set(high, tmp);
    i
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

#[allow(dead_code)]
pub fn compute_trimmed_median(prices: &soroban_sdk::Vec<i128>, trim_percent: u32) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    if trim_percent == 0 {
        return compute_median(prices);
    }

    let mut sorted = prices.clone();
    sort_prices(&mut sorted);

    let trim_count = ((n.saturating_mul(trim_percent) / 100) / 2).min(n - 1);
    if trim_count == 0 {
        return compute_median(&sorted);
    }

    let mut trimmed: soroban_sdk::Vec<i128> = soroban_sdk::Vec::new(prices.env());
    for i in trim_count..(n - trim_count) {
        trimmed.push_back(sorted.get_unchecked(i));
    }

    if trimmed.is_empty() {
        return sorted.get_unchecked(n / 2);
    }

    compute_median(&trimmed)
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
    let key = DataKey::RegisteredAssets;
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
        .set(&DataKey::RegisteredAssets, assets);
}

pub fn read_oracle_sources(env: &Env) -> OracleSources {
    let key = DataKey::OracleSources;
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
    let key = DataKey::InactiveSource(source.clone());
    env.storage().persistent().get(&key).unwrap_or(false)
}

pub fn mark_source_inactive(env: &Env, source: &Address) {
    let key = DataKey::InactiveSource(source.clone());
    env.storage().persistent().set(&key, &true);
}

pub fn mark_source_active(env: &Env, source: &Address) {
    let key = DataKey::InactiveSource(source.clone());
    env.storage().persistent().remove(&key);
}
