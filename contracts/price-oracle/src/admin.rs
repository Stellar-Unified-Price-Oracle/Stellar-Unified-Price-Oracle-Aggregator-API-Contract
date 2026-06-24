use soroban_sdk::{panic_with_error, Address, Env, String};

use crate::events::{
    AdminChangedEvent, ContractUpgradedEvent, DecimalsChangedEvent, DescriptionChangedEvent,
    MaxHistoryChangedEvent, MinSourcesChangedEvent, ResolutionChangedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, OracleSources};

const DEFAULT_MAX_HISTORY: u32 = 100;
const DEFAULT_MIN_SOURCES: u32 = 1;
const DEFAULT_DECIMALS: u32 = 18;
pub const DEFAULT_RESOLUTION: u32 = 0;

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
    env.storage()
        .persistent()
        .set(&DataKey::Resolution, &DEFAULT_RESOLUTION);
    env.storage()
        .persistent()
        .set(&DataKey::Decimals, &decimals);
    env.storage()
        .persistent()
        .set(&DataKey::Description, &description);
    env.storage().persistent().set(
        &DataKey::OracleSources,
        &OracleSources {
            sources: soroban_sdk::Vec::new(env),
            metadata: soroban_sdk::Map::new(env),
        },
    );
    env.storage()
        .persistent()
        .set(&DataKey::RegisteredAssets, &soroban_sdk::Vec::<Address>::new(env));
}

pub fn upgrade(env: &Env, new_wasm_hash: soroban_sdk::BytesN<32>) {
    let admin = get_admin(env);
    admin.require_auth();
    ContractUpgradedEvent {
        new_wasm_hash: new_wasm_hash.clone(),
    }
    .publish(env);
    env.deployer().update_current_contract_wasm(new_wasm_hash);
}

pub fn set_admin(env: &Env, new_admin: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage().persistent().set(&DataKey::Admin, &new_admin);
    AdminChangedEvent {
        new_admin: new_admin.clone(),
    }
    .publish(env);
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
    env.storage()
        .persistent()
        .set(&DataKey::MinSourcesRequired, &new_min);
    MinSourcesChangedEvent { value: new_min }.publish(env);
}

pub fn get_min_sources_required(env: &Env) -> u32 {
    let key = DataKey::MinSourcesRequired;
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
    env.storage()
        .persistent()
        .set(&DataKey::MaxHistoryLength, &new_max);
    MaxHistoryChangedEvent { value: new_max }.publish(env);
}

pub fn get_max_history_length(env: &Env) -> u32 {
    let key = DataKey::MaxHistoryLength;
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
        .set(&DataKey::Resolution, &new_resolution);
    ResolutionChangedEvent {
        value: new_resolution,
    }
    .publish(env);
}

pub fn get_resolution(env: &Env) -> u32 {
    let key = DataKey::Resolution;
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
    env.storage()
        .persistent()
        .set(&DataKey::Decimals, &new_decimals);
    DecimalsChangedEvent {
        value: new_decimals,
    }
    .publish(env);
}

pub fn get_decimals(env: &Env) -> u32 {
    let key = DataKey::Decimals;
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
    env.storage()
        .persistent()
        .set(&DataKey::Description, &new_description);
    DescriptionChangedEvent {
        description: new_description.clone(),
    }
    .publish(env);
}

pub fn get_description(env: &Env) -> String {
    let key = DataKey::Description;
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
