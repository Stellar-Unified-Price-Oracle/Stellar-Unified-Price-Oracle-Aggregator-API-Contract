#![no_std]

mod types;
mod events;
mod storage;

pub use types::{
    AggregatePrice, Asset, DataKey, ErrorCode, OracleSources, PriceData, PriceEntry,
    PriceHistoryEntry,
};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Symbol, Vec, panic_with_error};

use crate::events::*;
use crate::storage::*;

const DEFAULT_MAX_HISTORY: u32 = 100;
const DEFAULT_MIN_SOURCES: u32 = 1;
const DEFAULT_DECIMALS: u32 = 18;
const DEFAULT_RESOLUTION: u32 = 0;

#[contract]
pub struct PriceOracleContract;

#[contractimpl]
impl PriceOracleContract {
    pub fn __constructor(_env: Env) {}

    pub fn initialize(
        env: Env,
        admin: Address,
        min_sources_required: u32,
        max_history_length: u32,
        decimals: u32,
        description: String,
    ) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic_with_error!(env, ErrorCode::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(
            &DataKey::MinSourcesRequired,
            &if min_sources_required > 0 {
                min_sources_required
            } else {
                DEFAULT_MIN_SOURCES
            },
        );
        env.storage().persistent().set(
            &DataKey::MaxHistoryLength,
            &if max_history_length > 0 {
                max_history_length
            } else {
                DEFAULT_MAX_HISTORY
            },
        );
        env.storage().persistent().set(&DataKey::Resolution, &DEFAULT_RESOLUTION);
        env.storage().persistent().set(&DataKey::Decimals, &decimals);
        env.storage().persistent().set(&DataKey::Description, &description);
        env.storage().persistent().set(
            &DataKey::OracleSources,
            &OracleSources {
                sources: Vec::new(&env),
                metadata: soroban_sdk::Map::new(&env),
            },
        );
        env.storage()
            .persistent()
            .set(&DataKey::RegisteredAssets, &Vec::<Address>::new(&env));
    }

    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) {
        let admin = get_admin(&env);
        admin.require_auth();
        env.deployer().update_current_contract_wasm(new_wasm_hash);
    }

    pub fn set_admin(env: Env, new_admin: Address) {
        let admin = get_admin(&env);
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        AdminChangedEvent {
            new_admin: new_admin.clone(),
        }
        .publish(&env);
    }

    pub fn get_admin_address(env: Env) -> Address {
        if env.storage().persistent().has(&DataKey::Admin) {
            env.storage()
                .persistent()
                .extend_ttl(&DataKey::Admin, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        get_admin(&env)
    }

    pub fn set_min_sources_required(env: Env, new_min: u32) {
        let admin = get_admin(&env);
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::MinSourcesRequired, &new_min);
        MinSourcesChangedEvent { value: new_min }.publish(&env);
    }

    pub fn get_min_sources_required(env: Env) -> u32 {
        let key = DataKey::MinSourcesRequired;
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        env.storage().persistent().get(&key).unwrap_or(DEFAULT_MIN_SOURCES)
    }

    pub fn set_max_history_length(env: Env, new_max: u32) {
        let admin = get_admin(&env);
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::MaxHistoryLength, &new_max);
        MaxHistoryChangedEvent { value: new_max }.publish(&env);
    }

    pub fn get_max_history_length(env: Env) -> u32 {
        let key = DataKey::MaxHistoryLength;
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        env.storage().persistent().get(&key).unwrap_or(DEFAULT_MAX_HISTORY)
    }

    pub fn set_resolution(env: Env, new_resolution: u32) {
        let admin = get_admin(&env);
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::Resolution, &new_resolution);
        ResolutionChangedEvent { value: new_resolution }.publish(&env);
    }

    pub fn get_resolution(env: Env) -> u32 {
        let key = DataKey::Resolution;
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        env.storage().persistent().get(&key).unwrap_or(DEFAULT_RESOLUTION)
    }

    pub fn set_decimals(env: Env, new_decimals: u32) {
        let admin = get_admin(&env);
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Decimals, &new_decimals);
        DecimalsChangedEvent { value: new_decimals }.publish(&env);
    }

    pub fn get_decimals(env: Env) -> u32 {
        let key = DataKey::Decimals;
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        env.storage().persistent().get(&key).unwrap_or(DEFAULT_DECIMALS)
    }

    pub fn set_description(env: Env, new_description: String) {
        let admin = get_admin(&env);
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::Description, &new_description);
        DescriptionChangedEvent {
            description: new_description.clone(),
        }
        .publish(&env);
    }

    pub fn get_description(env: Env) -> String {
        let key = DataKey::Description;
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(String::from_str(&env, "Stellar Price Oracle"))
    }

    pub fn register_asset(env: Env, asset: Address) {
        let admin = get_admin(&env);
        admin.require_auth();
        if env
            .storage()
            .persistent()
            .has(&DataKey::AssetRegistered(asset.clone()))
        {
            panic_with_error!(env, ErrorCode::AssetAlreadyRegistered);
        }
        env.storage()
            .persistent()
            .set(&DataKey::AssetRegistered(asset.clone()), &true);
        env.storage()
            .persistent()
            .set(&DataKey::Aggregate(asset.clone()), &AggregatePrice {
                price: 0,
                timestamp: 0,
                num_sources: 0,
                decimals: Self::get_decimals(env.clone()),
            });
        let mut assets = read_registered_assets(&env);
        assets.push_back(asset.clone());
        write_registered_assets(&env, &assets);
        AssetRegisteredEvent {
            asset: asset.clone(),
        }
        .publish(&env);
    }

    pub fn unregister_asset(env: Env, asset: Address) {
        let admin = get_admin(&env);
        admin.require_auth();
        check_registered_asset(&env, &asset);
        env.storage()
            .persistent()
            .remove(&DataKey::AssetRegistered(asset.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::Aggregate(asset.clone()));
        let assets = read_registered_assets(&env);
        let mut new_assets: Vec<Address> = Vec::new(&env);
        for i in 0..assets.len() {
            let a = assets.get_unchecked(i);
            if a != asset {
                new_assets.push_back(a);
            }
        }
        write_registered_assets(&env, &new_assets);
        AssetUnregisteredEvent {
            asset: asset.clone(),
        }
        .publish(&env);
    }

    pub fn is_asset_registered(env: Env, asset: Address) -> bool {
        let key = DataKey::AssetRegistered(asset);
        let exists: bool = env.storage().persistent().get(&key).unwrap_or(false);
        if exists {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        exists
    }

    pub fn add_source(env: Env, source: Address, name: String) {
        let admin = get_admin(&env);
        admin.require_auth();
        if env
            .storage()
            .persistent()
            .has(&DataKey::Source(source.clone()))
        {
            panic_with_error!(env, ErrorCode::SourceAlreadyExists);
        }
        env.storage()
            .persistent()
            .set(&DataKey::Source(source.clone()), &true);

        let mut oracle_sources: OracleSources = read_oracle_sources(&env);
        oracle_sources.sources.push_back(source.clone());
        let source_name = name.clone();
        oracle_sources.metadata.set(source.clone(), name);
        env.storage()
            .persistent()
            .set(&DataKey::OracleSources, &oracle_sources);
        SourceAddedEvent {
            source: source.clone(),
            name: source_name,
        }
        .publish(&env);
    }

    pub fn remove_source(env: Env, source: Address) {
        let admin = get_admin(&env);
        admin.require_auth();
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Source(source.clone()))
        {
            panic_with_error!(env, ErrorCode::SourceNotFound);
        }
        env.storage()
            .persistent()
            .remove(&DataKey::Source(source.clone()));

        let mut oracle_sources: OracleSources = read_oracle_sources(&env);
        let mut new_sources: Vec<Address> = Vec::new(&env);
        for i in 0..oracle_sources.sources.len() {
            let s = oracle_sources.sources.get_unchecked(i);
            if s != source {
                new_sources.push_back(s);
            }
        }
        oracle_sources.sources = new_sources;
        let removed_source = source.clone();
        oracle_sources.metadata.remove(source);
        env.storage()
            .persistent()
            .set(&DataKey::OracleSources, &oracle_sources);
        SourceRemovedEvent {
            source: removed_source,
        }
        .publish(&env);
    }

    pub fn is_source(env: Env, source: Address) -> bool {
        let key = DataKey::Source(source.clone());
        let exists: bool = env.storage().persistent().get(&key).unwrap_or(false);
        if exists {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
        exists
    }

    pub fn get_oracle_sources(env: Env) -> OracleSources {
        read_oracle_sources(&env)
    }

    pub fn submit_price(
        env: Env,
        source: Address,
        asset: Address,
        price: i128,
        timestamp: u64,
    ) {
        source.require_auth();
        check_source(&env, &source);
        check_registered_asset(&env, &asset);

        if price <= 0 {
            panic_with_error!(env, ErrorCode::InvalidPrice);
        }

        let decimals = Self::get_decimals(env.clone());

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
        .publish(&env);

        let min_required = Self::get_min_sources_required(env.clone());
        let oracle_sources: OracleSources = read_oracle_sources(&env);
        let total_sources = oracle_sources.sources.len();

        let mut valid_prices: Vec<i128> = Vec::new(&env);
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
            let prev_aggregate: AggregatePrice = env
                .storage()
                .persistent()
                .get(&agg_key)
                .unwrap();

            let aggregate = AggregatePrice {
                price: median_price,
                timestamp: latest_timestamp,
                num_sources: contributing_sources,
                decimals,
            };
            env.storage()
                .persistent()
                .set(&DataKey::Aggregate(asset.clone()), &aggregate);

            if prev_aggregate.price != median_price
                || prev_aggregate.timestamp != latest_timestamp
            {
                let history_entry = PriceHistoryEntry {
                    price: median_price,
                    timestamp: latest_timestamp,
                    ledger: current_ledger,
                    num_sources: contributing_sources,
                };
                env.storage()
                    .temporary()
                    .set(&DataKey::PriceHistory(asset.clone(), current_ledger), &history_entry);
            }

            PriceAggregatedEvent {
                asset: asset.clone(),
                price: median_price,
                num_sources: contributing_sources,
                timestamp: latest_timestamp,
            }
            .publish(&env);
        }
    }

    pub fn get_price(env: Env, asset: Address) -> AggregatePrice {
        check_registered_asset(&env, &asset);
        let key = DataKey::Aggregate(asset);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        let result: AggregatePrice = env.storage().persistent().get(&key).unwrap();
        let resolution = Self::get_resolution(env.clone());
        if resolution > 0 {
            let ledger_time = env.ledger().timestamp();
            if result.timestamp + (resolution as u64) < ledger_time {
                panic_with_error!(env, ErrorCode::NoData);
            }
        }
        result
    }

    pub fn get_source_price(
        env: Env,
        asset: Address,
        source: Address,
    ) -> PriceEntry {
        check_registered_asset(&env, &asset);
        check_source(&env, &source);
        let key = DataKey::Submission(asset, source);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        env.storage().persistent().get(&key).unwrap()
    }

    pub fn get_all_prices(env: Env, asset: Address) -> Vec<PriceEntry> {
        check_registered_asset(&env, &asset);
        let oracle_sources: OracleSources = read_oracle_sources(&env);
        let mut prices: Vec<PriceEntry> = Vec::new(&env);
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

    pub fn get_historical_price(
        env: Env,
        asset: Address,
        ledger: u32,
    ) -> PriceHistoryEntry {
        check_registered_asset(&env, &asset);
        let key = DataKey::PriceHistory(asset, ledger);
        env.storage()
            .temporary()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        env.storage().temporary().get(&key).unwrap()
    }

    pub fn has_historical_price(
        env: Env,
        asset: Address,
        ledger: u32,
    ) -> bool {
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
        env: Env,
        asset: Address,
        start_ledger: u32,
        end_ledger: u32,
    ) -> Vec<PriceHistoryEntry> {
        check_registered_asset(&env, &asset);
        let max_range = Self::get_max_history_length(env.clone());
        if end_ledger - start_ledger > max_range {
            panic_with_error!(env, ErrorCode::NoData);
        }
        let mut entries: Vec<PriceHistoryEntry> = Vec::new(&env);
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

    pub fn get_latest_ledger(env: Env) -> u32 {
        env.ledger().sequence()
    }

    // ---- SEP-40 Oracle Interface ----

    pub fn base(env: Env) -> Asset {
        Asset::Other(Symbol::new(&env, "USD"))
    }

    pub fn assets(env: Env) -> Vec<Asset> {
        let registered = read_registered_assets(&env);
        let mut result: Vec<Asset> = Vec::new(&env);
        for i in 0..registered.len() {
            result.push_back(Asset::Stellar(registered.get_unchecked(i)));
        }
        result
    }

    pub fn resolution(env: Env) -> u32 {
        Self::get_resolution(env)
    }

    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
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
        let resolution = Self::get_resolution(env.clone());
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

    pub fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
        let addr = match asset {
            Asset::Stellar(a) => a,
            Asset::Other(_) => return None,
        };
        let reg_key = DataKey::AssetRegistered(addr.clone());
        if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
            return None;
        }
        let agg_key = DataKey::Aggregate(addr.clone());
        if let Some(agg) = env.storage().persistent().get::<_, AggregatePrice>(&agg_key) {
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
        while ledger >= start {
            let hist_key = DataKey::PriceHistory(addr.clone(), ledger);
            if let Some(entry) = env.storage().temporary().get::<_, PriceHistoryEntry>(&hist_key)
            {
                if entry.timestamp <= timestamp {
                    return Some(PriceData {
                        price: entry.price,
                        timestamp: entry.timestamp,
                    });
                }
            }
            ledger -= 1;
        }
        None
    }

    pub fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
        let addr = match asset {
            Asset::Stellar(a) => a,
            Asset::Other(_) => return None,
        };
        let reg_key = DataKey::AssetRegistered(addr.clone());
        if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
            return None;
        }
        if records == 0 {
            return Some(Vec::new(&env));
        }
        let mut result: Vec<PriceData> = Vec::new(&env);
        let current_ledger = env.ledger().sequence();
        let max_to_check = (records * 10).min(10000);
        let start = current_ledger.saturating_sub(max_to_check);
        let mut ledger = current_ledger;
        while ledger >= start && result.len() < records {
            let hist_key = DataKey::PriceHistory(addr.clone(), ledger);
            if let Some(entry) = env.storage().temporary().get::<_, PriceHistoryEntry>(&hist_key)
            {
                result.push_back(PriceData {
                    price: entry.price,
                    timestamp: entry.timestamp,
                });
            }
            ledger -= 1;
        }
        if result.is_empty() {
            let agg_key = DataKey::Aggregate(addr);
            if let Some(agg) = env.storage().persistent().get::<_, AggregatePrice>(&agg_key) {
                result.push_back(PriceData {
                    price: agg.price,
                    timestamp: agg.timestamp,
                });
            }
        }
        Some(result)
    }
}

mod test;
