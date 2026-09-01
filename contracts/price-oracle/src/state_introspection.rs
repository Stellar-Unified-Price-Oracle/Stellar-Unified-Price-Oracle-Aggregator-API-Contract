//! # Contract State Introspection Tools (#278)
//!
//! Provides read-only views over contract configuration and aggregated statistics.
//! These helpers are exposed through the CLI and can be called off-chain for
//! debugging and monitoring.

use soroban_sdk::{Address, Env, String, Symbol, Vec};

use crate::storage::get_admin;
use crate::types::{DataKey, StateAnalysis, StateDump};

/// Build a [`StateDump`] from live contract storage.
pub fn build_state_dump(env: &Env) -> StateDump {
    let admin = get_admin(env);
    StateDump {
        admin: admin.clone(),
        description: read_string(env, &DataKey::CfgDescription, String::from_str(env, "")),
        min_sources_required: read_u32(env, &DataKey::CfgMinSources, 1),
        max_history_length: read_u32(env, &DataKey::CfgMaxHistory, 100),
        decimals: read_u32(env, &DataKey::CfgDecimals, 18),
        resolution: read_u32(env, &DataKey::CfgResolution, 0),
        timestamp_threshold: read_u64(env, &DataKey::CfgTimestampThreshold, 300),
        max_deviation_bps: read_u32(env, &DataKey::CfgMaxDeviation, 500),
        heartbeat_interval: read_u64(env, &DataKey::CfgHeartbeatInterval, 0),
    }
}

/// Build a [`StateAnalysis`] from live contract storage.
pub fn build_state_analysis(env: &Env) -> StateAnalysis {
    let admin = get_admin(env);
    let registered_assets: u32 = env
        .storage()
        .persistent()
        .get::<_, Vec<Address>>(&DataKey::RegisteredAssets)
        .map(|v| v.len())
        .unwrap_or(0);

    let sources: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::OracleSources)
        .unwrap_or_else(|| Vec::new(env));

    StateAnalysis {
        admin,
        decimals: read_u32(env, &DataKey::CfgDecimals, 18),
        min_sources_required: read_u32(env, &DataKey::CfgMinSources, 1),
        max_history_length: read_u32(env, &DataKey::CfgMaxHistory, 100),
        registered_assets,
        registered_sources: sources.len() as u32,
        aggregate_count: 0,
        history_depth_avg: 0,
    }
}

fn read_u32(env: &Env, key: &DataKey, default: u32) -> u32 {
    env.storage().persistent().get(key).unwrap_or(default)
}

fn read_u64(env: &Env, key: &DataKey, default: u64) -> u64 {
    env.storage().persistent().get(key).unwrap_or(default)
}

fn read_string(env: &Env, key: &DataKey, default: String) -> String {
    env.storage().persistent().get(key).unwrap_or(default)
}
