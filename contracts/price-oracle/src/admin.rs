use soroban_sdk::{contracttype, panic_with_error, symbol_short, Address, Bytes, Env, String};

use crate::events::{
    emit_admin_action, emit_initialized, emit_max_price_deviation_changed,
    emit_timestamp_threshold_changed, AdminChangedEvent, AggCooldownChangedEvent,
    AssetResolutionSetEvent, ContractUpgradedEvent, DecimalsChangedEvent, DescriptionChangedEvent,
    DisputeWindowChangedEvent, EventsPerCallChangedEvent, HeartbeatIntervalChangedEvent,
    HistoryPerAssetChangedEvent, InterpolationChangedEvent, MaxAggSourcesChangedEvent,
    MaxHistoryChangedEvent, MaxSourcesChangedEvent, MinSourcesChangedEvent,
    OptimisticBondChangedEvent, QueryRateLimitChangedEvent, ResolutionChangedEvent,
    SubmitIntervalChangedEvent,
};
use crate::storage::{
    get_admin, read_oracle_sources, read_subscription_plans, write_subscription_plans,
    DEFAULT_QUERY_RATE_LIMIT, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{AggregationMethod, DataKey, ErrorCode, OracleSources};

const DEFAULT_MAX_HISTORY: u32 = 100;
const DEFAULT_MIN_SOURCES: u32 = 1;
const DEFAULT_MAX_ASSETS: u32 = 100;

const DEFAULT_DECIMALS: u32 = 18;
pub const DEFAULT_RESOLUTION: u32 = 0;
pub const DEFAULT_TIMESTAMP_THRESHOLD: u64 = 300; // 5 minutes
const DEFAULT_OPTIMISTIC_DISPUTE_WINDOW: u32 = 120;
const DEFAULT_OPTIMISTIC_MIN_BOND: i128 = 100_000_000;
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

/// Upper bounds enforced on configuration setters so a compromised or buggy
/// admin cannot push the contract into an unusable state.
pub const MAX_MIN_SOURCES: u32 = 100_000;
pub const MAX_MAX_HISTORY: u32 = 1_000_000;
pub const MAX_RESOLUTION: u32 = 1_000_000;
/// One day, in seconds — the timestamp window may not exceed the reporting period.
pub const MAX_TIMESTAMP_THRESHOLD: u64 = 86_400;
/// One year, in seconds — heartbeat liveness intervals beyond this are meaningless.
pub const MAX_HEARTBEAT_INTERVAL: u64 = 31_536_000;

// ---------------------------------------------------------------------------
// Emergency council (issue #549)
// ---------------------------------------------------------------------------

/// Default lifetime of an emergency council mandate, in seconds (24 hours).
/// Authority is time-boxed: once `now >= expires_at` the council can no longer
/// act, enforced on-chain at action time.
pub const DEFAULT_EMERGENCY_MANDATE_SECS: u64 = 86_400;
/// Default window after an emergency action within which governance must
/// ratify it, in seconds (48 hours).
pub const DEFAULT_EMERGENCY_RATIFICATION_SECS: u64 = 172_800;

/// The closed, emergency-only action set. Anything not listed here is rejected
/// by `emergency_action`, and governance-mutating actions are explicitly
/// excluded (see `is_governance_action`).
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum EmergencyAction {
    Pause = 1,
    Unpause = 2,
    TripBreaker = 3,
    OracleOverride = 4,
}

impl EmergencyAction {
    pub fn from_u32(value: u32) -> Option<Self> {
        match value {
            1 => Some(EmergencyAction::Pause),
            2 => Some(EmergencyAction::Unpause),
            3 => Some(EmergencyAction::TripBreaker),
            4 => Some(EmergencyAction::OracleOverride),
            _ => None,
        }
    }
}

/// Actions that change governance itself. These can never be performed through
/// the emergency path, even if a council is active.
fn is_governance_action(action: u32) -> bool {
    // Governance-mutating operations are identified by their admin action
    // symbols; the emergency set above never overlaps with these.
    matches!(
        action,
        // set_admin / set_min_sources / set_max_history / set_resolution /
        // set_decimals / set_description / set_aggregation / upgrade etc.
        100..=199
    )
}

/// On-chain record of an emergency action awaiting ratification.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingRatification {
    pub action: u32,
    pub actor: Address,
    pub issued_at: u64,
    pub deadline: u64,
}

fn emergency_council_key() -> DataKey {
    DataKey::EmergencyCouncil
}

fn emergency_expiry_key() -> DataKey {
    DataKey::EmergencyExpiry
}

fn pending_ratification_key() -> DataKey {
    DataKey::EmergencyPending
}

/// Install (or refresh) the emergency council with a time-boxed mandate.
/// Only the admin (governance) may do this. The mandate expires automatically
/// at `now + duration`; a zero duration falls back to the default.
pub fn set_emergency_council(env: &Env, council: Address, duration_secs: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    let duration = if duration_secs == 0 {
        DEFAULT_EMERGENCY_MANDATE_SECS
    } else {
        duration_secs
    };
    let expires_at = env.ledger().timestamp().saturating_add(duration);
    env.storage()
        .persistent()
        .set(&emergency_council_key(), &council);
    env.storage()
        .persistent()
        .set(&emergency_expiry_key(), &expires_at);
    env.events().publish(
        (symbol_short!("emergency"), symbol_short!("council")),
        (council, expires_at),
    );
    emit_admin_action(env, symbol_short!("set_emrg"), admin, Bytes::new(env));
}

/// Returns the active council address if the mandate has not expired.
/// Expiry is enforced on-chain against the ledger timestamp.
pub fn get_active_emergency_council(env: &Env) -> Option<Address> {
    let council: Address = env.storage().persistent().get(&emergency_council_key())?;
    let expires_at: u64 = env
        .storage()
        .persistent()
        .get(&emergency_expiry_key())
        .unwrap_or(0);
    if env.ledger().timestamp() >= expires_at {
        None
    } else {
        Some(council)
    }
}

/// Perform an emergency-only action. Rejects:
/// - actions outside the closed emergency set,
/// - governance-mutating actions,
/// - calls after the mandate has expired.
///
/// Records a pending ratification with a fixed deadline and events the action.
pub fn emergency_action(env: &Env, action: u32, payload: Bytes) {
    let council = match get_active_emergency_council(env) {
        Some(c) => c,
        None => panic_with_error!(env, ErrorCode::EmergencyExpired),
    };
    council.require_auth();

    if is_governance_action(action) {
        panic_with_error!(env, ErrorCode::EmergencyActionForbidden);
    }
    if EmergencyAction::from_u32(action).is_none() {
        panic_with_error!(env, ErrorCode::EmergencyActionForbidden);
    }

    let now = env.ledger().timestamp();
    let deadline = now.saturating_add(DEFAULT_EMERGENCY_RATIFICATION_SECS);
    let pending = PendingRatification {
        action,
        actor: council.clone(),
        issued_at: now,
        deadline,
    };
    env.storage()
        .persistent()
        .set(&pending_ratification_key(), &pending);

    env.events().publish(
        (symbol_short!("emergency"), symbol_short!("action")),
        (action, council, now, deadline, payload),
    );
}

/// Governance ratifies the most recent emergency action before its deadline.
/// Emits the ratification outcome and clears the pending record.
pub fn ratify_emergency_action(env: &Env, approved: bool) {
    let admin = get_admin(env);
    admin.require_auth();
    let pending: PendingRatification = env
        .storage()
        .persistent()
        .get(&pending_ratification_key())
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoPendingEmergency));
    if env.ledger().timestamp() > pending.deadline {
        panic_with_error!(env, ErrorCode::EmergencyRatificationExpired);
    }
    env.storage()
        .persistent()
        .remove(&pending_ratification_key());
    env.events().publish(
        (symbol_short!("emergency"), symbol_short!("ratify")),
        (pending.action, approved, admin),
    );
}

/// Detect a missed ratification: if a pending emergency action exists and its
/// deadline has passed, escalate by emitting an event and clearing the record.
/// Returns `true` when an escalation occurred.
pub fn check_emergency_ratification(env: &Env) -> bool {
    let pending: PendingRatification =
        match env.storage().persistent().get(&pending_ratification_key()) {
            Some(p) => p,
            None => return false,
        };
    if env.ledger().timestamp() <= pending.deadline {
        return false;
    }
    env.storage()
        .persistent()
        .remove(&pending_ratification_key());
    env.events().publish(
        (symbol_short!("emergency"), symbol_short!("escalate")),
        (pending.action, pending.actor, pending.deadline),
    );
    true
}

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
        &DataKey::CfgOptimisticDisputeWindow,
        &DEFAULT_OPTIMISTIC_DISPUTE_WINDOW,
    );
    env.storage()
        .persistent()
        .set(&DataKey::CfgOptimisticMinBond, &DEFAULT_OPTIMISTIC_MIN_BOND);
    env.storage().persistent().set(
        &DataKey::SrcRegistry,
        &OracleSources {
            sources: soroban_sdk::Vec::new(env),
            metadata: soroban_sdk::Map::new(env),
            verification: soroban_sdk::Map::new(env),
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
    env.storage()
        .persistent()
        .set(&DataKey::QueryRateLimit, &DEFAULT_QUERY_RATE_LIMIT);
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
    if new_min == 0 || new_min > MAX_MIN_SOURCES {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let oracle_sources = read_oracle_sources(env);
    let source_count = oracle_sources.sources.len();
    if source_count > 0 && new_min > source_count {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    crate::config_history::snapshot_before_change(env, &admin);
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
        // Fail closed: `initialize` always writes this key, so a missing entry means it was
        // evicted. Falling back to a permissive default would let a single source set prices.
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ConfigMissing))
}

pub fn set_max_history_length(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if new_max == 0 || new_max > MAX_MAX_HISTORY {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::CfgMaxHistory, &new_max);
    MaxHistoryChangedEvent { value: new_max }.publish(env);
    emit_admin_action(env, symbol_short!("set_maxh"), admin, Bytes::new(env));
}

// ─────────────────────────────────────────────────────────────────────────────
// Asset Lifecycle Management: Disable/Enable
// ─────────────────────────────────────────────────────────────────────────────

/// Disables an asset, blocking new price submissions and aggregations.
/// Historical prices remain queryable.
///
/// # Arguments
/// * `asset` — the asset address to disable
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
pub fn disable_asset(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);

    let key = DataKey::AssetDisabled(asset.clone());
    crate::feature_flags::mark(env, crate::feature_flags::ASSET_DISABLED);
    env.storage().persistent().set(&key, &true);

    let disabled_at_key = DataKey::AssetDisabledAt(asset.clone());
    let current_ledger = env.ledger().sequence();
    env.storage()
        .persistent()
        .set(&disabled_at_key, &current_ledger);

    emit_admin_action(env, symbol_short!("dis_ast"), admin, Bytes::new(env));
}

/// Re-enables a disabled asset, allowing price submissions and aggregations to resume.
///
/// # Arguments
/// * `asset` — the asset address to re-enable
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
pub fn enable_asset(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);

    let key = DataKey::AssetDisabled(asset.clone());
    env.storage().persistent().remove(&key);

    emit_admin_action(env, symbol_short!("en_ast"), admin, Bytes::new(env));
}

/// Queries whether an asset is disabled.
///
/// # Arguments
/// * `asset` — the asset address to check
///
/// # Returns
/// `true` if the asset is disabled, `false` if active or not registered.
pub fn is_asset_disabled(env: &Env, asset: &Address) -> bool {
    if !crate::feature_flags::on(env, crate::feature_flags::ASSET_DISABLED) {
        return false;
    }
    let key = DataKey::AssetDisabled(asset.clone());
    let disabled: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if disabled {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    disabled
}

/// Queries the ledger when an asset was disabled.
///
/// # Arguments
/// * `asset` — the asset address to check
///
/// # Returns
/// The ledger sequence number when the asset was disabled, or 0 if not disabled.
pub fn get_asset_disabled_at(env: &Env, asset: &Address) -> u32 {
    let key = DataKey::AssetDisabledAt(asset.clone());
    let disabled_at: u32 = env.storage().persistent().get(&key).unwrap_or(0);
    if disabled_at > 0 {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    disabled_at
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
    if new_resolution > MAX_RESOLUTION {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    crate::config_history::snapshot_before_change(env, &admin);
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
    crate::config_history::snapshot_before_change(env, &admin);
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
    crate::config_history::snapshot_before_change(env, &admin);
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

/// Set the active aggregation method. Admin-only.
///
/// # Valid values
/// * `0` — `Median` (default)
/// * `1` — `Mean`
/// * `2` — `TrimmedMean`
/// * `3` — `WeightedMedian`
/// * `4` — freshness-weighted median
pub fn set_aggregation_method(env: &Env, method: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if method > 4 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let old_method = get_aggregation_method(env);
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::CfgAggregationMethod, &method);
    crate::events::AggregationMethodChangedEvent {
        admin: admin.clone(),
        old_method,
        new_method: method,
    }
    .publish(env);
    emit_admin_action(
        env,
        symbol_short!("set_agg"),
        admin,
        soroban_sdk::Bytes::new(env),
    );
}

pub fn set_timestamp_threshold(env: &Env, threshold: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    if threshold == 0 || threshold > MAX_TIMESTAMP_THRESHOLD {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    crate::config_history::snapshot_before_change(env, &admin);
    crate::feature_flags::mark(env, crate::feature_flags::TIMESTAMP_THRESHOLD);
    env.storage()
        .persistent()
        .set(&DataKey::CfgTimestampThreshold, &threshold);
    emit_timestamp_threshold_changed(env, admin.clone(), threshold);
    emit_admin_action(env, symbol_short!("set_ts"), admin, Bytes::new(env));
}

pub fn get_timestamp_threshold(env: &Env) -> u64 {
    if !crate::feature_flags::on(env, crate::feature_flags::TIMESTAMP_THRESHOLD) {
        return DEFAULT_TIMESTAMP_THRESHOLD;
    }
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
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::CfgMaxDeviation, &deviation_basis_points);
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

pub fn set_circuit_breaker_threshold(env: &Env, threshold_bps: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if threshold_bps > 100000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::CircuitBreakerThreshold, &threshold_bps);
}

pub fn get_circuit_breaker_threshold(env: &Env) -> u32 {
    let key = DataKey::CircuitBreakerThreshold;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_heartbeat_interval(env: &Env, interval: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    if interval == 0 || interval > MAX_HEARTBEAT_INTERVAL {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    crate::config_history::snapshot_before_change(env, &admin);
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

pub fn set_max_history_per_asset(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    crate::feature_flags::mark(env, crate::feature_flags::MAX_HISTORY_PER_ASSET);
    env.storage()
        .persistent()
        .set(&DataKey::MaxHistoryPerAsset, &new_max);
    HistoryPerAssetChangedEvent { value: new_max }.publish(env);
}

pub fn get_max_history_per_asset(env: &Env) -> u32 {
    if !crate::feature_flags::on(env, crate::feature_flags::MAX_HISTORY_PER_ASSET) {
        return DEFAULT_MAX_HISTORY_PER_ASSET;
    }
    let key = DataKey::MaxHistoryPerAsset;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_MAX_HISTORY_PER_ASSET)
}

pub fn set_max_events_per_call(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    crate::feature_flags::mark(env, crate::feature_flags::MAX_EVENTS_PER_CALL);
    env.storage()
        .persistent()
        .set(&DataKey::MaxEventsPerCall, &new_max);
    EventsPerCallChangedEvent { value: new_max }.publish(env);
}

pub fn get_max_events_per_call(env: &Env) -> u32 {
    if !crate::feature_flags::on(env, crate::feature_flags::MAX_EVENTS_PER_CALL) {
        return DEFAULT_MAX_EVENTS_PER_CALL;
    }
    let key = DataKey::MaxEventsPerCall;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_MAX_EVENTS_PER_CALL)
}

pub fn set_max_aggregation_sources(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    crate::feature_flags::mark(env, crate::feature_flags::MAX_AGGREGATION_SOURCES);
    env.storage()
        .persistent()
        .set(&DataKey::MaxAggregationSources, &new_max);
    MaxAggSourcesChangedEvent { value: new_max }.publish(env);
}

pub fn get_max_aggregation_sources(env: &Env) -> u32 {
    if !crate::feature_flags::on(env, crate::feature_flags::MAX_AGGREGATION_SOURCES) {
        return DEFAULT_MAX_AGGREGATION_SOURCES;
    }
    let key = DataKey::MaxAggregationSources;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_MAX_AGGREGATION_SOURCES)
}

pub fn set_asset_resolution(env: &Env, asset: Address, resolution: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    let key = DataKey::AssetResolution(asset.clone());
    if resolution == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &resolution);
    }
    AssetResolutionSetEvent {
        asset,
        admin: admin.clone(),
        resolution,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_ares"), admin, Bytes::new(env));
}

pub fn get_asset_resolution(env: &Env, asset: Address) -> u32 {
    let key = DataKey::AssetResolution(asset);
    if let Some(r) = env.storage().persistent().get::<_, u32>(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        r
    } else {
        get_resolution(env)
    }
}

pub fn set_aggregation_cooldown(env: &Env, cooldown_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::AggregationCooldown, &cooldown_ledgers);
    AggCooldownChangedEvent { cooldown_ledgers }.publish(env);
    emit_admin_action(env, symbol_short!("set_acd"), admin, Bytes::new(env));
}

pub fn get_aggregation_cooldown(env: &Env) -> u32 {
    let key = DataKey::AggregationCooldown;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(10)
}

pub fn set_optimistic_dispute_window(env: &Env, dispute_window_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if dispute_window_ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage().persistent().set(
        &DataKey::CfgOptimisticDisputeWindow,
        &dispute_window_ledgers,
    );
    DisputeWindowChangedEvent {
        admin: admin.clone(),
        dispute_window_ledgers,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_odw"), admin, Bytes::new(env));
}

pub fn get_optimistic_dispute_window(env: &Env) -> u32 {
    let key = DataKey::CfgOptimisticDisputeWindow;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_OPTIMISTIC_DISPUTE_WINDOW)
}

pub fn set_optimistic_min_bond(env: &Env, min_bond: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    if min_bond <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgOptimisticMinBond, &min_bond);
    OptimisticBondChangedEvent {
        admin: admin.clone(),
        min_bond,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_omb"), admin, Bytes::new(env));
}

pub fn get_optimistic_min_bond(env: &Env) -> i128 {
    let key = DataKey::CfgOptimisticMinBond;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_OPTIMISTIC_MIN_BOND)
}

pub fn set_min_submission_interval(env: &Env, interval_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    crate::feature_flags::mark(env, crate::feature_flags::MIN_SUBMISSION_INTERVAL);
    env.storage()
        .persistent()
        .set(&DataKey::MinSubmissionInterval, &interval_ledgers);
    SubmitIntervalChangedEvent { interval_ledgers }.publish(env);
    emit_admin_action(env, symbol_short!("set_msi"), admin, Bytes::new(env));
}

pub fn get_min_submission_interval(env: &Env) -> u32 {
    if !crate::feature_flags::on(env, crate::feature_flags::MIN_SUBMISSION_INTERVAL) {
        return 0;
    }
    let key = DataKey::MinSubmissionInterval;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_interpolation_enabled(env: &Env, enabled: bool) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::InterpolationEnabled, &enabled);
    InterpolationChangedEvent { enabled }.publish(env);
    emit_admin_action(env, symbol_short!("set_intp"), admin, Bytes::new(env));
}

pub fn get_interpolation_enabled(env: &Env) -> bool {
    let key = DataKey::InterpolationEnabled;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(true)
}

pub fn set_max_sources(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::MaxSources, &new_max);
    MaxSourcesChangedEvent { value: new_max }.publish(env);
    emit_admin_action(env, symbol_short!("set_msrc"), admin, Bytes::new(env));
}

pub fn get_max_sources(env: &Env) -> u32 {
    let key = DataKey::MaxSources;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn set_query_rate_limit(env: &Env, max_per_ledger: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
        .set(&DataKey::QueryRateLimit, &max_per_ledger);
    QueryRateLimitChangedEvent {
        value: max_per_ledger,
    }
    .publish(env);
}

pub fn get_query_rate_limit(env: &Env) -> u32 {
    let key = DataKey::QueryRateLimit;
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

pub fn set_max_assets(env: &Env, new_max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
    env.storage()
        .persistent()
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
        .unwrap_or(DEFAULT_MAX_ASSETS)
}

pub fn set_subscription_price(env: &Env, duration: u32, amount: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    let mut plans = read_subscription_plans(env);
    plans.set(duration, amount);
    write_subscription_plans(env, &plans);
}

/// Sets the history compaction threshold in basis points.
///
/// When `threshold_bps > 0`, adjacent history entries whose price difference is
/// within `threshold_bps / 100 %` of each other are eligible for merging.
/// A value of `0` disables compaction entirely (default).
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
pub fn set_compaction_threshold_bps(env: &Env, threshold_bps: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::history::set_compaction_threshold_bps(env, threshold_bps);
    emit_admin_action(env, symbol_short!("set_comp"), admin, Bytes::new(env));
}

/// Returns the current history compaction threshold in basis points (0 = disabled).
pub fn get_compaction_threshold_bps(env: &Env) -> u32 {
    crate::history::get_compaction_threshold_bps(env)
}
