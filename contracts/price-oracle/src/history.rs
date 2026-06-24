use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::admin::get_max_history_length;
use crate::storage::{check_registered_asset, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, PriceHistoryEntry};

pub fn get_historical_price(env: &Env, asset: Address, ledger: u32) -> PriceHistoryEntry {
    check_registered_asset(env, &asset);
    let key = DataKey::PriceHistory(asset, ledger);
    env.storage()
        .temporary()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    env.storage().temporary().get(&key).unwrap()
}

pub fn has_historical_price(env: &Env, asset: Address, ledger: u32) -> bool {
    if !env
        .storage()
        .persistent()
        .has(&DataKey::AssetRegistered(asset.clone()))
    {
        return false;
    }
    let key = DataKey::PriceHistory(asset, ledger);
    let exists = env.storage().temporary().has(&key);
    if exists {
        env.storage()
            .temporary()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    exists
}

pub fn get_historical_prices(
    env: &Env,
    asset: Address,
    start_ledger: u32,
    end_ledger: u32,
) -> Vec<PriceHistoryEntry> {
    check_registered_asset(env, &asset);
    let max_range = get_max_history_length(env);
    if end_ledger - start_ledger > max_range {
        panic_with_error!(env, ErrorCode::NoData);
    }
    let mut entries: Vec<PriceHistoryEntry> = Vec::new(env);
    let mut ledger = start_ledger;
    while ledger <= end_ledger {
        let key = DataKey::PriceHistory(asset.clone(), ledger);
        if env.storage().temporary().has(&key) {
            let entry: PriceHistoryEntry = env.storage().temporary().get(&key).unwrap();
            env.storage()
                .temporary()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            entries.push_back(entry);
        }
        ledger += 1;
    }
    entries
}
