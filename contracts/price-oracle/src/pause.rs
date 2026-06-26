use soroban_sdk::{panic_with_error, Env};

use crate::events::{ContractPausedEvent, ContractUnpausedEvent};
use crate::storage::get_admin;
use crate::types::{DataKey, ErrorCode};

pub fn pause(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();
    
    env.storage().persistent().set(&DataKey::PauseFlag, &true);
    
    ContractPausedEvent {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn unpause(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();
    
    env.storage().persistent().set(&DataKey::PauseFlag, &false);
    
    ContractUnpausedEvent {
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::PauseFlag)
        .unwrap_or(false)
}

pub fn check_not_paused(env: &Env) {
    if is_paused(env) {
        panic_with_error!(env, ErrorCode::ContractPaused);
    }
}
