use soroban_sdk::{panic_with_error, Address, Env, String, Vec};

use crate::admin::{
    get_aggregation_method, get_decimals, get_max_history_length, get_min_sources_required,
    get_resolution, get_timestamp_threshold,
};
use crate::events::{
    HistoryPrunedEvent, PriceAggregatedEvent, PriceOverrideExpiredEvent, PriceOverrideRemovedEvent,
    PriceOverrideSetEvent, PriceStaleEvent, PriceSubmittedEvent, SourcesInsufficientEvent,
};
use crate::pause::check_not_paused;
use crate::storage::{
    check_registered_asset, check_source, compute_mean, compute_median, compute_trimmed_mean,
    get_admin, read_oracle_sources, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{
    AggregatePrice, Asset, DataKey, ErrorCode, OracleSources, PriceData, PriceEntry,
    PriceHistoryEntry, PriceOverrideEntry,
};

pub fn submit_price(env: &Env, source: Address, asset: Address, price: i128, timestamp: u64) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);

    if crate::sources::is_source_suspended(env, source.clone()) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    if price <= 0 {
        crate::sources::record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let min_price = crate::assets::get_min_price(env, asset.clone());
    if price < min_price {
        panic_with_error!(env, ErrorCode::PriceBelowMinimum);
    }

    let ledger_time = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    if timestamp > ledger_time + threshold {
        crate::sources::record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();

    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
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
        let sub_key = DataKey::Submission(asset.clone(), src.clone());
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
        let method = get_aggregation_method(env);
        let median_price = match method {
            0 => compute_median(&valid_prices),
            1 => compute_mean(&valid_prices),
            2 => compute_trimmed_mean(&valid_prices, 10),
            _ => compute_median(&valid_prices),
        };

        let agg_key = DataKey::Aggregate(asset.clone());
        let prev_aggregate: AggregatePrice =
            env.storage()
                .persistent()
                .get(&agg_key)
                .unwrap_or(AggregatePrice {
                    price: 0,
                    timestamp: 0,
                    num_sources: 0,
                    decimals,
                    is_override: false,
                });

        let aggregate = AggregatePrice {
            price: median_price,
            timestamp: latest_timestamp,
            num_sources: contributing_sources,
            decimals,
            is_override: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Aggregate(asset.clone()), &aggregate);
        env.storage().persistent().extend_ttl(
            &DataKey::Aggregate(asset.clone()),
            LEDGER_THRESHOLD,
            LEDGER_BUMP,
        );

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

            // Track ledger in history index for pruning
            let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
            let mut ledger_list: soroban_sdk::Vec<u32> = env
                .storage()
                .persistent()
                .get(&ledgers_key)
                .unwrap_or(soroban_sdk::Vec::new(env));
            ledger_list.push_back(current_ledger);

            let max_history = get_max_history_length(env);
            while ledger_list.len() > max_history {
                let oldest_ledger = ledger_list.get_unchecked(0);
                ledger_list.remove(0);
                env.storage()
                    .temporary()
                    .remove(&DataKey::PriceHistory(asset.clone(), oldest_ledger));
                HistoryPrunedEvent {
                    asset: asset.clone(),
                    pruned_ledger: oldest_ledger,
                    remaining: ledger_list.len(),
                }
                .publish(env);
            }
            env.storage().persistent().set(&ledgers_key, &ledger_list);
        }

        PriceAggregatedEvent {
            asset: asset.clone(),
            price: median_price,
            num_sources: contributing_sources,
            timestamp: latest_timestamp,
        }
        .publish(env);
    } else {
        SourcesInsufficientEvent {
            asset: asset.clone(),
            current_source_count: contributing_sources,
            min_sources_required: min_required,
        }
        .publish(env);
    }
}

pub fn get_price(env: &Env, asset: Address, max_age: u64) -> Option<AggregatePrice> {
    check_registered_asset(env, &asset);
    let current_ledger = env.ledger().sequence();

    // Check for active price override
    let override_key = DataKey::PriceOverride(asset.clone());
    if let Some(ovr) = env
        .storage()
        .persistent()
        .get::<_, PriceOverrideEntry>(&override_key)
    {
        if current_ledger <= ovr.expiry_ledger {
            env.storage()
                .persistent()
                .extend_ttl(&override_key, LEDGER_THRESHOLD, LEDGER_BUMP);
            let decimals = get_decimals(env);
            return Some(AggregatePrice {
                price: ovr.price,
                timestamp: env.ledger().timestamp(),
                num_sources: 0,
                decimals,
                is_override: true,
            });
        } else {
            // Override has expired
            PriceOverrideExpiredEvent {
                asset: asset.clone(),
                expiry_ledger: ovr.expiry_ledger,
                current_ledger,
            }
            .publish(env);
            env.storage().persistent().remove(&override_key);
        }
    }

    let key = DataKey::Aggregate(asset.clone());
    let result: AggregatePrice = env.storage().persistent().get(&key)?;

    if max_age > 0 {
        let ledger_time = env.ledger().timestamp();
        if result.timestamp + max_age < ledger_time {
            PriceStaleEvent {
                asset: asset.clone(),
                last_update_ledger: 0,
                current_ledger,
            }
            .publish(env);
            return None;
        }
    }
    let resolution = get_resolution(env);
    if resolution > 0 {
        let ledger_time = env.ledger().timestamp();
        if result.timestamp + (resolution as u64) < ledger_time {
            PriceStaleEvent {
                asset: asset.clone(),
                last_update_ledger: 0,
                current_ledger,
            }
            .publish(env);
            return None;
        }
    }
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    Some(result)
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
        last_updated: env.ledger().sequence(),
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
                last_updated: env.ledger().sequence(),
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
                    last_updated: ledger,
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
                last_updated: ledger,
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
                last_updated: current_ledger,
            });
        }
    }
    Some(result)
}

#[allow(dead_code)]
pub fn get_prices(env: &Env, assets: Vec<Address>) -> Vec<Option<AggregatePrice>> {
    let mut results: Vec<Option<AggregatePrice>> = Vec::new(env);
    for i in 0..assets.len() {
        let asset = assets.get_unchecked(i);
        let price = get_price(env, asset, 0);
        results.push_back(price);
    }
    results
}

pub fn override_price(env: &Env, asset: Address, price: i128, reason: String, expiry_ledger: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let current_ledger = env.ledger().sequence();
    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }
    if expiry_ledger <= current_ledger {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let entry = PriceOverrideEntry {
        price,
        reason: reason.clone(),
        expiry_ledger,
        set_ledger: current_ledger,
    };
    env.storage()
        .persistent()
        .set(&DataKey::PriceOverride(asset.clone()), &entry);
    env.storage().persistent().extend_ttl(
        &DataKey::PriceOverride(asset.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    PriceOverrideSetEvent {
        asset: asset.clone(),
        admin: admin.clone(),
        price,
        reason,
        expiry_ledger,
    }
    .publish(env);
}

pub fn remove_price_override(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let override_key = DataKey::PriceOverride(asset.clone());
    if !env.storage().persistent().has(&override_key) {
        panic_with_error!(env, ErrorCode::NoData);
    }
    env.storage().persistent().remove(&override_key);

    PriceOverrideRemovedEvent {
        asset: asset.clone(),
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn get_price_override(env: &Env, asset: Address) -> Option<PriceOverrideEntry> {
    check_registered_asset(env, &asset);
    let override_key = DataKey::PriceOverride(asset);
    if env.storage().persistent().has(&override_key) {
        env.storage()
            .persistent()
            .extend_ttl(&override_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&override_key)
}

#[allow(dead_code)]
pub fn get_price_change(env: &Env, asset: Address, ledgers_back: u32) -> Option<i128> {
    check_registered_asset(env, &asset);

    let current_price = get_price(env, asset.clone(), 0)?;

    if current_price.price == 0 {
        return None;
    }

    let current_ledger = env.ledger().sequence();
    let target_ledger = current_ledger.saturating_sub(ledgers_back);

    let hist_key = DataKey::PriceHistory(asset.clone(), target_ledger);
    let historical_entry: Option<PriceHistoryEntry> = env.storage().temporary().get(&hist_key);

    let old_price = match historical_entry {
        Some(entry) => entry.price,
        None => return None,
    };

    if old_price == 0 {
        return None;
    }

    let change_percent = ((current_price.price - old_price) * 100) / old_price;
    Some(change_percent)
}
