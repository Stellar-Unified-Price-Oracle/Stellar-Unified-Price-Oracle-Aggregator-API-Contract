//! Emergency pause module (#240)
//!
//! Implements emergency pause functionality that bypasses normal timelock delays.
//! Useful for responding to critical incidents. Auto-unpauses after a configured
//! timeout unless extended by the admin.

use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String};

use crate::events::{
    emit_admin_action, EmergencyPauseExtendedEvent, EmergencyPausedEvent, EmergencyUnpausedEvent,
};
use crate::storage::get_admin;
use crate::types::{DataKey, EmergencyPause, ErrorCode};

/// Trigger an emergency pause that bypasses normal timelock delays.
///
/// The contract is immediately paused, blocking all price submissions and reads.
/// The pause automatically expires after `auto_unpause_ledgers` unless extended.
/// Admin can extend or cancel at any time.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `reason` - Human-readable reason for the emergency pause (max 256 chars).
/// * `auto_unpause_ledgers` - Number of ledgers until automatic unpause.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
/// * [`ErrorCode::ReasonTooLong`] — if reason exceeds 256 characters.
pub fn emergency_pause(env: &Env, reason: String, auto_unpause_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if reason.len() > 256 {
        panic_with_error!(env, ErrorCode::ReasonTooLong);
    }

    let current_ledger = env.ledger().sequence();
    let auto_unpause_ledger = current_ledger + auto_unpause_ledgers;

    let emergency_pause = EmergencyPause {
        reason: reason.clone(),
        initiated_ledger: current_ledger,
        auto_unpause_ledger,
        initiated_by: admin.clone(),
    };

    // Set pause flag
    env.storage()
        .persistent()
        .set(&DataKey::CfgPauseFlag, &true);

    // Store emergency pause entry
    env.storage()
        .persistent()
        .set(&DataKey::EmergencyPauseActive, &true);
    env.storage()
        .persistent()
        .set(&DataKey::EmergencyPauseEntry, &emergency_pause);
    env.storage()
        .persistent()
        .set(&DataKey::EmergencyPauseReason, &reason);

    // Emit event
    EmergencyPausedEvent {
        reason: reason.clone(),
        auto_unpause_ledger,
        initiated_by: admin.clone(),
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("emgp"), admin, Bytes::new(env));
}

/// Extend an active emergency pause.
///
/// Adds additional ledgers to the auto-unpause timer. Can be called multiple
/// times to keep the contract paused.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `additional_ledgers` - Additional ledgers to extend the pause.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] — if no emergency pause is active.
pub fn extend_emergency_pause(env: &Env, additional_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let mut emergency_pause: EmergencyPause = env
        .storage()
        .persistent()
        .get(&DataKey::EmergencyPauseEntry)
        .ok_or_else(|| panic_with_error!(env, ErrorCode::InvalidConfiguration))
        .unwrap();

    let current_ledger = env.ledger().sequence();

    // Check if auto-unpause has already occurred
    if current_ledger >= emergency_pause.auto_unpause_ledger {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    emergency_pause.auto_unpause_ledger += additional_ledgers;

    env.storage()
        .persistent()
        .set(&DataKey::EmergencyPauseEntry, &emergency_pause);

    // Emit event
    EmergencyPauseExtendedEvent {
        reason: emergency_pause.reason.clone(),
        new_unpause_ledger: emergency_pause.auto_unpause_ledger,
        extended_by: admin.clone(),
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("extep"), admin, Bytes::new(env));
}

/// Cancel an active emergency pause.
///
/// Immediately unpauses the contract, allowing price submissions and reads.
/// Admin can do this at any time to end the emergency pause early.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] — if no emergency pause is active.
pub fn cancel_emergency_pause(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();

    let emergency_pause: EmergencyPause = env
        .storage()
        .persistent()
        .get(&DataKey::EmergencyPauseEntry)
        .ok_or_else(|| panic_with_error!(env, ErrorCode::InvalidConfiguration))
        .unwrap();

    // Clear emergency pause state
    env.storage()
        .persistent()
        .remove(&DataKey::EmergencyPauseActive);
    env.storage()
        .persistent()
        .remove(&DataKey::EmergencyPauseEntry);
    env.storage()
        .persistent()
        .remove(&DataKey::EmergencyPauseReason);

    // Unpause the contract
    env.storage()
        .persistent()
        .set(&DataKey::CfgPauseFlag, &false);

    // Emit event
    EmergencyUnpausedEvent {
        reason: emergency_pause.reason,
        cancelled_by: admin.clone(),
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("cnlep"), admin, Bytes::new(env));
}

/// Check if emergency auto-unpause timeout has been reached, and unpause if so.
///
/// This is called internally by price submission and query endpoints to check
/// if the auto-unpause timer has expired.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
pub fn auto_unpause_if_due(env: &Env) {
    let is_emergency_active: bool = env
        .storage()
        .persistent()
        .get(&DataKey::EmergencyPauseActive)
        .unwrap_or(false);

    if !is_emergency_active {
        return;
    }

    let emergency_pause: Option<EmergencyPause> = env
        .storage()
        .persistent()
        .get(&DataKey::EmergencyPauseEntry);

    if let Some(pause) = emergency_pause {
        let current_ledger = env.ledger().sequence();

        if current_ledger >= pause.auto_unpause_ledger {
            // Auto-unpause has triggered
            env.storage()
                .persistent()
                .set(&DataKey::CfgPauseFlag, &false);
            env.storage()
                .persistent()
                .remove(&DataKey::EmergencyPauseActive);
            env.storage()
                .persistent()
                .remove(&DataKey::EmergencyPauseEntry);
            env.storage()
                .persistent()
                .remove(&DataKey::EmergencyPauseReason);
        }
    }
}

/// Check if an emergency pause is currently active.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
///
/// # Returns
///
/// `true` if emergency pause is active, `false` otherwise.
pub fn is_emergency_pause_active(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::EmergencyPauseActive)
        .unwrap_or(false)
}

/// Get details of the current emergency pause, if active.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
///
/// # Returns
///
/// The `EmergencyPause` entry if active, or `None` if not.
pub fn get_emergency_pause(env: &Env) -> Option<EmergencyPause> {
    env.storage()
        .persistent()
        .get(&DataKey::EmergencyPauseEntry)
}
