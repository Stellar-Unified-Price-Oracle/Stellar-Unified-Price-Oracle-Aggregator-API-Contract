use soroban_sdk::{panic_with_error, Address, Env, String, Vec};

use crate::admin::get_heartbeat_interval;
use crate::events::{
    SourceActiveAgainEvent, SourceAddedEvent, SourceHeartbeatEvent, SourceInactiveEvent,
    SourceRemovedEvent,
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
        .has(&DataKey::Source(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceAlreadyExists);
    }
    env.storage()
        .persistent()
        .set(&DataKey::Source(source.clone()), &true);

    let mut oracle_sources: OracleSources = read_oracle_sources(env);
    oracle_sources.sources.push_back(source.clone());
    let source_name = name.clone();
    oracle_sources.metadata.set(source.clone(), name);
    env.storage()
        .persistent()
        .set(&DataKey::OracleSources, &oracle_sources);
    SourceAddedEvent {
        source: source.clone(),
        admin: admin.clone(),
        name: source_name,
    }
    .publish(env);
}

pub fn remove_source(env: &Env, source: Address) {
    let admin = get_admin(env);
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
        .set(&DataKey::OracleSources, &oracle_sources);
    SourceRemovedEvent {
        source: removed_source,
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn is_source(env: &Env, source: Address) -> bool {
    let key = DataKey::Source(source.clone());
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
        .set(&DataKey::SourceHeartbeat(source.clone()), &timestamp);

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
    let key = DataKey::SourceHeartbeat(source.clone());
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
    let key = DataKey::SourceHeartbeat(source);
    env.storage().persistent().get(&key).unwrap_or(0u64)
}
