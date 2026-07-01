use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String, Vec};

use crate::admin::get_heartbeat_interval;
use crate::events::{
    emit_admin_action, SourceActiveAgainEvent, SourceAddedEvent, SourceHeartbeatEvent,
    SourceInactiveEvent, SourceRemovedEvent,
};
use crate::storage::{
    get_admin, is_source_inactive as check_source_inactive, mark_source_active,
    mark_source_inactive, read_oracle_sources, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{DataKey, ErrorCode, OracleSources};

const MAX_SOURCE_NAME_LENGTH: u32 = 64;

pub fn add_source(env: &Env, source: Address, name: String) {
    let admin = get_admin(env);
    admin.require_auth();
    if name.is_empty() {
        panic_with_error!(env, ErrorCode::SourceNameEmpty);
    }
    if name.len() > MAX_SOURCE_NAME_LENGTH {
        panic_with_error!(env, ErrorCode::SourceNameTooLong);
    }
    if env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceAlreadyExists);
    }

    let oracle_sources: OracleSources = read_oracle_sources(env);
    let max_sources = crate::admin::get_max_sources(env);
    if oracle_sources.sources.len() >= max_sources {
        panic_with_error!(env, ErrorCode::MaxSourcesReached);
    }

    env.storage()
        .persistent()
        .set(&DataKey::SrcActive(source.clone()), &true);

    let mut oracle_sources: OracleSources = oracle_sources;
    oracle_sources.sources.push_back(source.clone());
    let source_name = name.clone();
    oracle_sources.metadata.set(source.clone(), name);
    env.storage()
        .persistent()
        .set(&DataKey::SrcRegistry, &oracle_sources);
    SourceAddedEvent {
        source: source.clone(),
        admin: admin.clone(),
        name: source_name,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("add_src"), admin, Bytes::new(env));
}

pub fn remove_source(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::SrcActive(source.clone()));

    let mut oracle_sources: OracleSources = read_oracle_sources(env);
    let mut new_sources: Vec<Address> = Vec::new(env);
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
        .set(&DataKey::SrcRegistry, &oracle_sources);
    SourceRemovedEvent {
        source: removed_source,
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("rem_src"), admin, Bytes::new(env));
}

pub fn is_source(env: &Env, source: Address) -> bool {
    let key = DataKey::SrcActive(source.clone());
    let exists: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if exists {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    exists
}

pub fn get_oracle_sources(env: &Env) -> OracleSources {
    read_oracle_sources(env)
}

pub fn submit_heartbeat(env: &Env, source: Address) {
    source.require_auth();
    if !is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    let timestamp = env.ledger().timestamp();
    env.storage()
        .persistent()
        .set(&DataKey::SrcHeartbeat(source.clone()), &timestamp);

    // If source was inactive, mark as active again
    let was_inactive = check_source_inactive(env, &source);
    if was_inactive {
        mark_source_active(env, &source);
        SourceActiveAgainEvent {
            source: source.clone(),
            timestamp,
        }
        .publish(env);
    }

    SourceHeartbeatEvent {
        source: source.clone(),
        timestamp,
    }
    .publish(env);
}

pub fn is_source_inactive(env: &Env, source: Address) -> bool {
    // Check if source was marked as inactive
    let is_marked_inactive = check_source_inactive(env, &source);
    if is_marked_inactive {
        return true;
    }

    // Check heartbeat timeout
    let key = DataKey::SrcHeartbeat(source.clone());
    let last_heartbeat: Option<u64> = env.storage().persistent().get(&key);

    if let Some(hb_time) = last_heartbeat {
        let interval = get_heartbeat_interval(env);
        let current_time = env.ledger().timestamp();
        if current_time > hb_time.saturating_add(interval) {
            // Mark as inactive
            mark_source_inactive(env, &source);
            SourceInactiveEvent {
                source: source.clone(),
                last_heartbeat: hb_time,
            }
            .publish(env);
            return true;
        }
    } else {
        // Never submitted heartbeat, but check if recently added
        // If no heartbeat ever, consider inactive after interval
        let current_time = env.ledger().timestamp();
        let interval = get_heartbeat_interval(env);
        // Allow grace period equal to interval from now
        if current_time > interval {
            mark_source_inactive(env, &source);
            return true;
        }
    }

    false
}

pub fn get_inactive_sources(env: &Env) -> u32 {
    let oracle_sources = read_oracle_sources(env);
    let mut count: u32 = 0;

    for i in 0..oracle_sources.sources.len() {
        let source = oracle_sources.sources.get_unchecked(i);
        if is_source_inactive(env, source) {
            count += 1;
        }
    }

    count
}

pub fn is_source_suspended(_env: &Env, _source: Address) -> bool {
    false
}

pub fn record_invalid_submission(_env: &Env, _source: Address) {}

pub fn get_source_last_heartbeat(env: &Env, source: Address) -> u64 {
    let key = DataKey::SrcHeartbeat(source);
    env.storage().persistent().get(&key).unwrap_or(0u64)
}

// --- #66: Phased source removal ---

const DEFAULT_REMOVAL_COOLDOWN: u32 = 100; // ledgers

pub fn set_removal_cooldown(env: &Env, ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::RemovalCooldown, &ledgers);
    RemovalCooldownChangedEvent { value: ledgers }.publish(env);
}

pub fn get_removal_cooldown(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::RemovalCooldown)
        .unwrap_or(DEFAULT_REMOVAL_COOLDOWN)
}

pub fn mark_source_for_removal(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !env
        .storage()
        .persistent()
        .has(&DataKey::Source(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    let cooldown = get_removal_cooldown(env);
    let current_ledger = env.ledger().sequence();
    let eligible_at = current_ledger + cooldown;
    env.storage()
        .persistent()
        .set(&DataKey::SourcePendingRemoval(source.clone()), &eligible_at);
    SourceMarkedForRemovalEvent {
        source: source.clone(),
        admin: admin.clone(),
        eligible_at_ledger: eligible_at,
    }
    .publish(env);
}

pub fn cancel_source_removal(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !env
        .storage()
        .persistent()
        .has(&DataKey::SourcePendingRemoval(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotPendingRemoval);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::SourcePendingRemoval(source.clone()));
    SourceRemovalCancelledEvent {
        source: source.clone(),
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn finalize_source_removal(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    let eligible_at: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::SourcePendingRemoval(source.clone()))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::SourceNotPendingRemoval));
    if env.ledger().sequence() < eligible_at {
        panic_with_error!(env, ErrorCode::CooldownNotElapsed);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::SourcePendingRemoval(source.clone()));
    // Perform the actual removal (same logic as remove_source)
    env.storage()
        .persistent()
        .remove(&DataKey::Source(source.clone()));
    let mut oracle_sources: OracleSources = read_oracle_sources(env);
    let mut new_sources: Vec<Address> = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let s = oracle_sources.sources.get_unchecked(i);
        if s != source {
            new_sources.push_back(s);
        }
    }
    oracle_sources.sources = new_sources;
    oracle_sources.metadata.remove(source.clone());
    env.storage()
        .persistent()
        .set(&DataKey::OracleSources, &oracle_sources);
    SourceRemovedEvent {
        source: source.clone(),
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn is_source_pending_removal(env: &Env, source: Address) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::SourcePendingRemoval(source))
}

// --- #65: Source reputation ---

const DEFAULT_DECAY_FACTOR: u32 = 10; // out of 100, higher = faster decay towards 50
const INITIAL_REPUTATION: i128 = 50;

pub fn set_reputation_decay_factor(env: &Env, factor: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::ReputationDecayFactor, &factor);
    crate::events::ReputationDecayChangedEvent { value: factor }.publish(env);
}

pub fn get_reputation_decay_factor(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::ReputationDecayFactor)
        .unwrap_or(DEFAULT_DECAY_FACTOR)
}

pub fn get_source_reputation(env: &Env, source: Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::SourceReputation(source))
        .unwrap_or(INITIAL_REPUTATION)
}

/// Called after aggregation to update a source's reputation based on deviation from median.
/// `source_price`: the price submitted by this source
/// `median_price`: the aggregated median for the asset
pub fn update_source_reputation(env: &Env, source: &Address, source_price: i128, median_price: i128) {
    if median_price == 0 {
        return;
    }
    let old_score = get_source_reputation(env, source.clone());
    let decay = get_reputation_decay_factor(env) as i128;

    // Deviation in basis points (0 = perfect, 10000 = 100% off)
    let deviation_bps = ((source_price - median_price).abs() * 10_000) / median_price;

    // Accuracy score: 100 if exact, decreasing linearly, floored at 0
    // 100 bps (~1%) deviation → still near perfect; 5000 bps (50%) → score 0
    let accuracy: i128 = if deviation_bps >= 5000 {
        0
    } else {
        100 - (deviation_bps * 100 / 5000)
    };

    // Weighted moving average: new = old * (100 - decay)/100 + accuracy * decay/100
    let new_score = (old_score * (100 - decay) + accuracy * decay) / 100;
    let new_score = new_score.clamp(0, 100);

    env.storage()
        .persistent()
        .set(&DataKey::SourceReputation(source.clone()), &new_score);

    crate::events::SourceReputationUpdatedEvent {
        source: source.clone(),
        old_score,
        new_score,
    }
    .publish(env);
}
