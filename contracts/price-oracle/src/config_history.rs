//! Configuration snapshot history and rollback.
//!
//! Captures core global oracle parameters before each successful mutation so
//! an admin can restore a known-good parameter set via `rollback_config`.

use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, Vec};

use crate::admin;
use crate::events::{emit_admin_action, ConfigRolledBackEvent, ConfigSnapshotTakenEvent};
use crate::pause;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::timelock;
use crate::types::{ConfigSnapshot, DataKey, ErrorCode};

/// Maximum number of retained configuration snapshots.
pub const MAX_CONFIG_HISTORY: u32 = 100;

/// Capture the current core configuration via canonical getters.
fn capture_current(env: &Env, version: u32) -> ConfigSnapshot {
    ConfigSnapshot {
        version,
        ledger: env.ledger().sequence(),
        timestamp: env.ledger().timestamp(),
        min_sources_required: admin::get_min_sources_required(env),
        max_history_length: admin::get_max_history_length(env),
        resolution: admin::get_resolution(env),
        decimals: admin::get_decimals(env),
        description: admin::get_description(env),
        aggregation_method: admin::get_aggregation_method(env),
        timestamp_threshold: admin::get_timestamp_threshold(env),
        max_price_deviation: admin::get_max_price_deviation(env),
        circuit_breaker_threshold: admin::get_circuit_breaker_threshold(env),
        heartbeat_interval: admin::get_heartbeat_interval(env),
        max_history_per_asset: admin::get_max_history_per_asset(env),
        max_events_per_call: admin::get_max_events_per_call(env),
        max_aggregation_sources: admin::get_max_aggregation_sources(env),
        aggregation_cooldown: admin::get_aggregation_cooldown(env),
        min_submission_interval: admin::get_min_submission_interval(env),
        interpolation_enabled: admin::get_interpolation_enabled(env),
        max_sources: admin::get_max_sources(env),
        query_rate_limit: admin::get_query_rate_limit(env),
        max_assets: admin::get_max_assets(env),
        paused: pause::is_paused(env),
        timelock_duration: timelock::get_timelock_duration(env),
    }
}

fn read_version_index(env: &Env) -> Vec<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::ConfigVersionIndex)
        .unwrap_or_else(|| Vec::new(env))
}

fn write_version_index(env: &Env, index: &Vec<u32>) {
    let key = DataKey::ConfigVersionIndex;
    env.storage().persistent().set(&key, index);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn store_snapshot(env: &Env, snapshot: &ConfigSnapshot, admin: &Address) {
    let version = snapshot.version;
    let snap_key = DataKey::ConfigSnapshot(version);
    env.storage().persistent().set(&snap_key, snapshot);
    env.storage()
        .persistent()
        .extend_ttl(&snap_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    env.storage()
        .persistent()
        .set(&DataKey::ConfigVersionCount, &version);
    env.storage().persistent().extend_ttl(
        &DataKey::ConfigVersionCount,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    let mut index = read_version_index(env);
    index.push_back(version);
    while index.len() > MAX_CONFIG_HISTORY {
        let old = index.get_unchecked(0);
        index.remove_unchecked(0);
        env.storage()
            .persistent()
            .remove(&DataKey::ConfigSnapshot(old));
    }
    write_version_index(env, &index);

    ConfigSnapshotTakenEvent {
        admin: admin.clone(),
        version,
        ledger: snapshot.ledger,
    }
    .publish(env);
}

/// Snapshot the live core configuration before a parameter mutation.
///
/// Call after auth/validation and before the storage write.
///
/// # Returns
///
/// The newly assigned snapshot version.
pub fn snapshot_before_change(env: &Env, admin: &Address) -> u32 {
    let count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::ConfigVersionCount)
        .unwrap_or(0);
    let version = count + 1;
    let snapshot = capture_current(env, version);
    store_snapshot(env, &snapshot, admin);
    version
}

/// Returns the newest retained configuration snapshots.
///
/// Ordering is newest-first. `count == 0` returns an empty vector. Results are
/// capped at [`MAX_CONFIG_HISTORY`].
pub fn get_config_history(env: &Env, count: u32) -> Vec<ConfigSnapshot> {
    if count == 0 {
        return Vec::new(env);
    }

    let index = read_version_index(env);
    let retained = index.len();
    if retained == 0 {
        return Vec::new(env);
    }

    let take = count.min(retained).min(MAX_CONFIG_HISTORY);

    let mut results = Vec::new(env);
    let mut i = retained;
    let mut returned = 0u32;
    while i > 0 && returned < take {
        i -= 1;
        let version = index.get_unchecked(i);
        if let Some(snapshot) = env
            .storage()
            .persistent()
            .get::<_, ConfigSnapshot>(&DataKey::ConfigSnapshot(version))
        {
            results.push_back(snapshot);
            returned += 1;
        }
    }
    results
}

fn apply_snapshot(env: &Env, snapshot: &ConfigSnapshot) {
    env.storage()
        .persistent()
        .set(&DataKey::CfgMinSources, &snapshot.min_sources_required);
    env.storage()
        .persistent()
        .set(&DataKey::CfgMaxHistory, &snapshot.max_history_length);
    env.storage()
        .persistent()
        .set(&DataKey::CfgResolution, &snapshot.resolution);
    env.storage()
        .persistent()
        .set(&DataKey::CfgDecimals, &snapshot.decimals);
    env.storage()
        .persistent()
        .set(&DataKey::CfgDescription, &snapshot.description);
    env.storage()
        .persistent()
        .set(&DataKey::CfgAggregationMethod, &snapshot.aggregation_method);
    env.storage().persistent().set(
        &DataKey::CfgTimestampThreshold,
        &snapshot.timestamp_threshold,
    );
    env.storage()
        .persistent()
        .set(&DataKey::CfgMaxDeviation, &snapshot.max_price_deviation);
    env.storage().persistent().set(
        &DataKey::CircuitBreakerThreshold,
        &snapshot.circuit_breaker_threshold,
    );
    env.storage()
        .persistent()
        .set(&DataKey::CfgHeartbeatInterval, &snapshot.heartbeat_interval);
    env.storage().persistent().set(
        &DataKey::MaxHistoryPerAsset,
        &snapshot.max_history_per_asset,
    );
    env.storage()
        .persistent()
        .set(&DataKey::MaxEventsPerCall, &snapshot.max_events_per_call);
    env.storage().persistent().set(
        &DataKey::MaxAggregationSources,
        &snapshot.max_aggregation_sources,
    );
    env.storage().persistent().set(
        &DataKey::AggregationCooldown,
        &snapshot.aggregation_cooldown,
    );
    env.storage().persistent().set(
        &DataKey::MinSubmissionInterval,
        &snapshot.min_submission_interval,
    );
    env.storage().persistent().set(
        &DataKey::InterpolationEnabled,
        &snapshot.interpolation_enabled,
    );
    env.storage()
        .persistent()
        .set(&DataKey::MaxSources, &snapshot.max_sources);
    env.storage()
        .persistent()
        .set(&DataKey::QueryRateLimit, &snapshot.query_rate_limit);
    env.storage()
        .persistent()
        .set(&DataKey::MaxAssets, &snapshot.max_assets);
    env.storage()
        .persistent()
        .set(&DataKey::CfgPauseFlag, &snapshot.paused);
    env.storage()
        .persistent()
        .set(&DataKey::CfgTimelockDuration, &snapshot.timelock_duration);
}

/// Restore a previously captured configuration snapshot.
///
/// Snapshots the current live config first (append-only), then applies the
/// selected version. Rejects missing or pruned versions.
pub fn rollback_config(env: &Env, version: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let target_key = DataKey::ConfigSnapshot(version);
    let target: ConfigSnapshot = env
        .storage()
        .persistent()
        .get(&target_key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ConfigVersionNotFound));

    let saved_version = snapshot_before_change(env, &admin);
    apply_snapshot(env, &target);

    ConfigRolledBackEvent {
        admin: admin.clone(),
        restored_version: version,
        saved_version,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("rollback"), admin, Bytes::new(env));
}
