use soroban_sdk::{panic_with_error, symbol_short, Bytes, Env};

use crate::events::{emit_admin_action, ContractPausedEvent, ContractUnpausedEvent};
use crate::storage::get_admin;
use crate::types::{DataKey, ErrorCode};

pub fn pause(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage().persistent().set(&DataKey::CfgPauseFlag, &true);

    ContractPausedEvent {
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("pause"), admin, Bytes::new(env));
}

pub fn unpause(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage().persistent().set(&DataKey::CfgPauseFlag, &false);

    ContractUnpausedEvent {
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("unpause"), admin, Bytes::new(env));
}

pub fn is_paused(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::CfgPauseFlag)
        .unwrap_or(false)
}

pub fn check_not_paused(env: &Env) {
    if is_paused(env) {
        panic_with_error!(env, ErrorCode::ContractPaused);
    }
}
