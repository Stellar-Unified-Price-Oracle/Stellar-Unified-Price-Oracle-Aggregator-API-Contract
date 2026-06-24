use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::admin::{get_decimals, get_min_sources_required, get_resolution};
use crate::events::{PriceAggregatedEvent, PriceSubmittedEvent};
use crate::storage::{
    check_registered_asset, check_source, compute_median, read_oracle_sources, LEDGER_BUMP,
    LEDGER_THRESHOLD,
};
use crate::types::{
    AggregatePrice, Asset, DataKey, ErrorCode, OracleSources, PriceData, PriceEntry,
    PriceHistoryEntry,
};

pub fn submit_price(env: &Env, source: Address, asset: Address, price: i128, timestamp: u64) {
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);

    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let decimals = get_decimals(env);

    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
    };

    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

    PriceSubmittedEvent {
        asset: asset.clone(),
        source: source.clone(),
        price,
        timestamp,
    }
    .publish(env);

    let min_required = get_min_sources_required(env);
    let oracle_sources: OracleSources = read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();

    let mut valid_prices: Vec<i128> = Vec::new(env);
    let mut latest_timestamp: u64 = 0;
    let mut contributing_sources: u32 = 0;

    for i in 0..total_sources {
        let src = oracle_sources.sources.get_unchecked(i);
        let sub_key = DataKey::Submission(asset.clone(), src);
        let sub: Option<PriceEntry> = env.storage().persistent().get(&sub_key);
        if let Some(entry_data) = sub {
            env.storage()
                .persistent()
                .extend_ttl(&sub_key, LEDGER_THRESHOLD, LEDGER_BUMP);
            if entry_data.timestamp > latest_timestamp {
                latest_timestamp = entry_data.timestamp;
            }
            valid_prices.push_back(entry_data.price);
            contributing_sources += 1;
        }
    }

    if contributing_sources >= min_required && !valid_prices.is_empty() {
        let median_price = compute_median(&valid_prices);

        let current_ledger = env.ledger().sequence();
        let agg_key = DataKey::Aggregate(asset.clone());
        env.storage()
            .persistent()
            .extend_ttl(&agg_key, LEDGER_THRESHOLD, LEDGER_BUMP);
        let prev_aggregate: AggregatePrice = env.storage().persistent().get(&agg_key).unwrap();

        let aggregate = AggregatePrice {
            price: median_price,
            timestamp: latest_timestamp,
            num_sources: contributing_sources,
            decimals,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Aggregate(asset.clone()), &aggregate);

        if prev_aggregate.price != median_price || prev_aggregate.timestamp != latest_timestamp {
            let history_entry = PriceHistoryEntry {
                price: median_price,
                timestamp: latest_timestamp,
                ledger: current_ledger,
                num_sources: contributing_sources,
            };
            env.storage().temporary().set(
                &DataKey::PriceHistory(asset.clone(), current_ledger),
                &history_entry,
            );
        }

        PriceAggregatedEvent {
            asset: asset.clone(),
            price: median_price,
            num_sources: contributing_sources,
            timestamp: latest_timestamp,
        }
        .publish(env);
    }
}

pub fn get_price(env: &Env, asset: Address) -> AggregatePrice {
    check_registered_asset(env, &asset);
    let key = DataKey::Aggregate(asset);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    let result: AggregatePrice = env.storage().persistent().get(&key).unwrap();
    let resolution = get_resolution(env);
    if resolution > 0 {
        let ledger_time = env.ledger().timestamp();
        if result.timestamp + (resolution as u64) < ledger_time {
            panic_with_error!(env, ErrorCode::NoData);
        }
    }
    result
}

pub fn get_source_price(env: &Env, asset: Address, source: Address) -> PriceEntry {
    check_registered_asset(env, &asset);
    check_source(env, &source);
    let key = DataKey::Submission(asset, source);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    env.storage().persistent().get(&key).unwrap()
}

pub fn get_all_prices(env: &Env, asset: Address) -> Vec<PriceEntry> {
    check_registered_asset(env, &asset);
    let oracle_sources: OracleSources = read_oracle_sources(env);
    let mut prices: Vec<PriceEntry> = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let src = oracle_sources.sources.get_unchecked(i);
        let sub_key = DataKey::Submission(asset.clone(), src);
        let sub: Option<PriceEntry> = env.storage().persistent().get(&sub_key);
        if let Some(entry) = sub {
            env.storage()
                .persistent()
                .extend_ttl(&sub_key, LEDGER_THRESHOLD, LEDGER_BUMP);
            prices.push_back(entry);
        }
    }
    prices
}

pub fn lastprice(env: &Env, asset: Asset) -> Option<PriceData> {
    let addr = match asset {
        Asset::Stellar(a) => a,
        Asset::Other(_) => return None,
    };
    let reg_key = DataKey::AssetRegistered(addr.clone());
    if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
        return None;
    }
    let agg_key = DataKey::Aggregate(addr);
    let result: AggregatePrice = env.storage().persistent().get(&agg_key)?;
    let resolution = get_resolution(env);
    if resolution > 0 {
        let ledger_time = env.ledger().timestamp();
        if result.timestamp + (resolution as u64) < ledger_time {
            return None;
        }
    }
    env.storage()
        .persistent()
        .extend_ttl(&agg_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    Some(PriceData {
        price: result.price,
        timestamp: result.timestamp,
    })
}

pub fn price(env: &Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
    let addr = match asset {
        Asset::Stellar(a) => a,
        Asset::Other(_) => return None,
    };
    let reg_key = DataKey::AssetRegistered(addr.clone());
    if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
        return None;
    }
    let agg_key = DataKey::Aggregate(addr.clone());
    if let Some(agg) = env
        .storage()
        .persistent()
        .get::<_, AggregatePrice>(&agg_key)
    {
        if agg.timestamp == timestamp {
            return Some(PriceData {
                price: agg.price,
                timestamp: agg.timestamp,
            });
        }
    }
    let current_ledger = env.ledger().sequence();
    let start = current_ledger.saturating_sub(1000);
    let mut ledger = current_ledger;
    loop {
        let hist_key = DataKey::PriceHistory(addr.clone(), ledger);
        if let Some(entry) = env
            .storage()
            .temporary()
            .get::<_, PriceHistoryEntry>(&hist_key)
        {
            if entry.timestamp <= timestamp {
                return Some(PriceData {
                    price: entry.price,
                    timestamp: entry.timestamp,
                });
            }
        }
        if ledger == start {
            break;
        }
        ledger -= 1;
    }
    None
}

pub fn prices(env: &Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
    let addr = match asset {
        Asset::Stellar(a) => a,
        Asset::Other(_) => return None,
    };
    let reg_key = DataKey::AssetRegistered(addr.clone());
    if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
        return None;
    }
    if records == 0 {
        return Some(Vec::new(env));
    }
    let mut result: Vec<PriceData> = Vec::new(env);
    let current_ledger = env.ledger().sequence();
    let max_to_check = (records * 10).min(10000);
    let start = current_ledger.saturating_sub(max_to_check);
    let mut ledger = current_ledger;
    loop {
        let hist_key = DataKey::PriceHistory(addr.clone(), ledger);
        if let Some(entry) = env
            .storage()
            .temporary()
            .get::<_, PriceHistoryEntry>(&hist_key)
        {
            result.push_back(PriceData {
                price: entry.price,
                timestamp: entry.timestamp,
            });
            if result.len() >= records {
                break;
            }
        }
        if ledger == start {
            break;
        }
        ledger -= 1;
    }
    if result.is_empty() {
        let agg_key = DataKey::Aggregate(addr);
        if let Some(agg) = env
            .storage()
            .persistent()
            .get::<_, AggregatePrice>(&agg_key)
        {
            result.push_back(PriceData {
                price: agg.price,
                timestamp: agg.timestamp,
            });
        }
    }
    Some(result)
}
