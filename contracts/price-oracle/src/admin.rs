use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String};

use crate::events::{
    emit_initialized, emit_max_price_deviation_changed, emit_timestamp_threshold_changed,
    AdminChangedEvent, ContractUpgradedEvent, DecimalsChangedEvent, DescriptionChangedEvent,
    HeartbeatIntervalChangedEvent, MaxHistoryChangedEvent, MinSourcesChangedEvent,
    QueryRateLimitChangedEvent, ResolutionChangedEvent,
};
use crate::storage::{
    get_admin, read_oracle_sources, read_subscription_plans, write_subscription_plans,
    DEFAULT_QUERY_RATE_LIMIT, LEDGER_BUMP, LEDGER_THRESHOLD,
    HeartbeatIntervalChangedEvent, MaxAggregationSourcesChangedEvent, MaxEventsPerCallChangedEvent,
    MaxHistoryChangedEvent, MaxHistoryPerAssetChangedEvent, MinSourcesChangedEvent,
    ResolutionChangedEvent,
};
use crate::types::{AggregationMethod, DataKey, ErrorCode, OracleSources};

const DEFAULT_MAX_HISTORY: u32 = 100;
const DEFAULT_MIN_SOURCES: u32 = 1;
const DEFAULT_MAX_ASSETS: u32 = 100;

const DEFAULT_DECIMALS: u32 = 18;
pub const DEFAULT_RESOLUTION: u32 = 0;
pub const DEFAULT_TIMESTAMP_THRESHOLD: u64 = 300; // 5 minutes
const MAX_DESCRIPTION_LENGTH: u32 = 256;
pub const DEFAULT_MAX_PRICE_DEVIATION: u32 = 500; // 5% in basis points
pub const DEFAULT_HEARTBEAT_INTERVAL: u64 = 3600; // 1 hour
pub const DEFAULT_MAX_INVALID_SUBMISSIONS: u32 = 5;
/// Default per-asset history cap (issue #94).
pub const DEFAULT_MAX_HISTORY_PER_ASSET: u32 = 1000;
/// Default maximum events per call (issue #92).
pub const DEFAULT_MAX_EVENTS_PER_CALL: u32 = 20;
/// Default maximum aggregation sources; 0 means no limit (issue #93).
pub const DEFAULT_MAX_AGGREGATION_SOURCES: u32 = 0;

pub fn initialize(
    env: &Env,
    admin: Address,

    min_sources_required: u32,
    max_history_length: u32,
    decimals: u32,
    description: String,
) {
    if env.storage().persistent().has(&DataKey::Admin) {
        panic_with_error!(env, ErrorCode::AlreadyInitialized);
    }
    if description.len() > MAX_DESCRIPTION_LENGTH {
        panic_with_error!(env, ErrorCode::DescriptionTooLong);
    }
    if decimals > 18 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    admin.require_auth();
    env.storage().persistent().set(&DataKey::Admin, &admin);
    env.storage().persistent().set(
        &DataKey::CfgMinSources,
        &if min_sources_required > 0 {
            min_sources_required
        } else {
            DEFAULT_MIN_SOURCES
        },
    );
    env.storage().persistent().set(
        &DataKey::CfgMaxHistory,
        &if max_history_length > 0 {
            max_history_length
        } else {
            DEFAULT_MAX_HISTORY
        },
    );
    env.storage()
        .persistent()
        .set(&DataKey::CfgResolution, &DEFAULT_RESOLUTION);
    env.storage()
        .persistent()
        .set(&DataKey::CfgDecimals, &decimals);
    env.storage()
        .persistent()
        .set(&DataKey::CfgDescription, &description);
    env.storage().persistent().set(
        &DataKey::SrcRegistry,
        &OracleSources {
            sources: soroban_sdk::Vec::new(env),
            metadata: soroban_sdk::Map::new(env),
        },
    );
    env.storage().persistent().set(
        &DataKey::AssetRegistry,
        &soroban_sdk::Vec::<Address>::new(env),
    );
    env.storage().persistent().set(
        &DataKey::CfgMaxInvalidSubs,
        &DEFAULT_MAX_INVALID_SUBMISSIONS,
    );
    env.storage()
        .persistent()
        .set(&DataKey::MaxAssets, &DEFAULT_MAX_ASSETS);
    env.storage().persistent().set(
        &DataKey::CfgAggregationMethod,
        &(AggregationMethod::Median as u32),
    );
    env.storage().persistent().set(
        &DataKey::QueryRateLimit,
        &DEFAULT_QUERY_RATE_LIMIT,
    );
    let init_admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
    emit_initialized(
        env,
        init_admin.clone(),
        if min_sources_required > 0 {
            min_sources_required
        } else {
            DEFAULT_MIN_SOURCES
        },
        if max_history_length > 0 {
            max_history_length
        } else {
            DEFAULT_MAX_HISTORY
        },
        decimals,
        description,
    );
    emit_admin_action(env, symbol_short!("init"), init_admin, Bytes::new(env));
}

pub fn upgrade(env: &Env, new_wasm_hash: soroban_sdk::BytesN<32>) {
    let admin = get_admin(env);
    admin.require_auth();
    ContractUpgradedEvent {
        new_wasm_hash: new_wasm_hash.clone(),
    }
    .publish(env);
    emit_admin_action(
        env,
        symbol_short!("upgrade"),
        admin.clone(),
        Bytes::new(env),
    );
    env.deployer().update_current_contract_wasm(new_wasm_hash);
}

pub fn set_admin(env: &Env, new_admin: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage().persistent().set(&DataKey::Admin, &new_admin);
    AdminChangedEvent {
        old_admin: admin.clone(),
        new_admin: new_admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_admin"), admin, Bytes::new(env));
}

pub fn get_admin_address(env: &Env) -> Address {
    if env.storage().persistent().has(&DataKey::Admin) {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    get_admin(env)
}

pub fn set_min_sources_required(env: &Env, new_min: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if new_min == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let oracle_sources = read_oracle_sources(env);
    let source_count = oracle_sources.sources.len();
    if source_count > 0 && new_min > source_count {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgMinSources, &new_min);
    MinSourcesChangedEvent { value: new_min }.publish(env);
    emit_admin_action(env, symbol_short!("set_min"), admin, Bytes::new(env));
}

pub fn get_min_sources_required(env: &Env) -> u32 {
    let key = DataKey::CfgMinSources;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_MIN_SOURCES)
}

pub fn set_max_history_length(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if new_max == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgMaxHistory, &new_max);
    MaxHistoryChangedEvent { value: new_max }.publish(env);
    emit_admin_action(env, symbol_short!("set_max"), admin, Bytes::new(env));
}

pub fn get_max_history_length(env: &Env) -> u32 {
    let key = DataKey::CfgMaxHistory;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_MAX_HISTORY)
}

pub fn set_resolution(env: &Env, new_resolution: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::CfgResolution, &new_resolution);
    ResolutionChangedEvent {
        value: new_resolution,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_res"), admin, Bytes::new(env));
}

pub fn get_resolution(env: &Env) -> u32 {
    let key = DataKey::CfgResolution;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_RESOLUTION)
}

pub fn set_decimals(env: &Env, new_decimals: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if new_decimals > 18 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgDecimals, &new_decimals);
    DecimalsChangedEvent {
        value: new_decimals,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_dec"), admin, Bytes::new(env));
}

pub fn get_decimals(env: &Env) -> u32 {
    let key = DataKey::CfgDecimals;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_DECIMALS)
}

pub fn set_description(env: &Env, new_description: String) {
    let admin = get_admin(env);
    admin.require_auth();
    if new_description.len() > MAX_DESCRIPTION_LENGTH {
        panic_with_error!(env, ErrorCode::DescriptionTooLong);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgDescription, &new_description);
    DescriptionChangedEvent {
        description: new_description.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_desc"), admin, Bytes::new(env));
}

pub fn get_description(env: &Env) -> String {
    let key = DataKey::CfgDescription;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(String::from_str(env, "Stellar Price Oracle"))
}

pub fn get_aggregation_method(env: &Env) -> u32 {
    let key = DataKey::CfgAggregationMethod;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(AggregationMethod::Median as u32)
}

pub fn set_timestamp_threshold(env: &Env, threshold: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::TimestampThreshold, &threshold);
    emit_timestamp_threshold_changed(env, admin.clone(), threshold);
    emit_admin_action(env, symbol_short!("set_ts"), admin, Bytes::new(env));
}

pub fn get_timestamp_threshold(env: &Env) -> u64 {
    let key = DataKey::CfgTimestampThreshold;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_TIMESTAMP_THRESHOLD)
}

pub fn set_max_price_deviation(env: &Env, deviation_basis_points: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if deviation_basis_points > 100000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::MaxPriceDeviation, &deviation_basis_points);
    emit_max_price_deviation_changed(env, admin.clone(), deviation_basis_points);
    emit_admin_action(env, symbol_short!("set_dev"), admin, Bytes::new(env));
}

pub fn get_max_price_deviation(env: &Env) -> u32 {
    let key = DataKey::CfgMaxDeviation;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_MAX_PRICE_DEVIATION)
}

pub fn set_heartbeat_interval(env: &Env, interval: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    if interval == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgHeartbeatInterval, &interval);
    HeartbeatIntervalChangedEvent { value: interval }.publish(env);
    emit_admin_action(env, symbol_short!("set_hb"), admin, Bytes::new(env));
}

pub fn get_heartbeat_interval(env: &Env) -> u64 {
    let key = DataKey::CfgHeartbeatInterval;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL)
}

pub fn set_query_rate_limit(env: &Env, max_per_ledger: u32) {
pub fn set_max_assets(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::QueryRateLimit, &max_per_ledger);
    QueryRateLimitChangedEvent { value: max_per_ledger }.publish(env);
}

pub fn get_query_rate_limit(env: &Env) -> u32 {
    let key = DataKey::QueryRateLimit;
        .set(&DataKey::MaxAssets, &new_max);
}

pub fn get_max_assets(env: &Env) -> u32 {
    let key = DataKey::MaxAssets;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_QUERY_RATE_LIMIT)
}

pub fn set_subscription_price(env: &Env, duration: u32, amount: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    let mut plans = read_subscription_plans(env);
    plans.set(duration, amount);
    write_subscription_plans(env, &plans);
        .unwrap_or(DEFAULT_MAX_ASSETS)
}
