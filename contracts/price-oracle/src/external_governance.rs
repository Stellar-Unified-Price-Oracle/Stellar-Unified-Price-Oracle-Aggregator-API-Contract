//! # Cross-contract governance delegation
//!
//! Lets the on-chain admin delegate a *narrow, explicit* set of governance
//! operations to an external governor (a governance contract or an operations
//! account) without handing over full admin rights.
//!
//! Nothing is delegated implicitly: each operation name has to be allow-listed
//! individually with [`allow_governor_op`], and the whole delegation can be
//! revoked at any time with [`clear_external_governor`].
//!
//! ## Storage layout
//!
//! | Key | Type | Description |
//! |-----|------|-------------|
//! | `ExternalGovernor` | `Address` | Currently delegated governor |
//! | `GovernorEpoch` | `u32` | Authorization epoch; bumping it revokes every grant |
//! | `GovernorOpGrant(name)` | `u32` | Epoch at which `name` was allow-listed |
//!
//! ## Trust model
//!
//! * The admin always outranks the governor: every function in this module
//!   requires admin auth, so the governor can never change its own authority.
//! * A grant is only valid in the epoch it was made. Installing, replacing or
//!   clearing a governor, or calling [`reauthorize_governor`] (e.g. after the
//!   external contract was upgraded), starts a new epoch, so trust is never
//!   inherited by a new governor or a new governor implementation.
//! * The oracle makes no calls into the governor, so it can neither block nor
//!   re-enter oracle operations. See `docs/governance-attack-surface-audit.md`.

use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String};

use crate::events::emit_admin_action;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode};

/// Delegates governance to `governor`.
///
/// Admin only. Registering a governor grants no power on its own — every
/// operation must additionally be allow-listed with [`allow_governor_op`].
pub fn set_external_governor(env: &Env, governor: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if governor == env.current_contract_address() {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    bump_epoch(env);

    let key = DataKey::ExternalGovernor;
    env.storage().persistent().set(&key, &governor);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns the currently delegated governor.
///
/// When no delegation is active the on-chain admin is returned, so callers
/// always get a usable address and governance falls back to the admin.
pub fn get_external_governor(env: &Env) -> Address {
    let key = DataKey::ExternalGovernor;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return env.storage().persistent().get(&key).unwrap();
    }
    get_admin(env)
}

/// Revokes the current delegation, returning governance to the on-chain admin.
///
/// Admin only.
pub fn clear_external_governor(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .remove(&DataKey::ExternalGovernor);
    bump_epoch(env);
}

/// Current governor authorization epoch.
pub fn get_governor_epoch(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::GovernorEpoch)
        .unwrap_or(0)
}

fn bump_epoch(env: &Env) {
    let key = DataKey::GovernorEpoch;
    env.storage()
        .persistent()
        .set(&key, &get_governor_epoch(env).saturating_add(1));
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Revokes every operation grant, requiring each to be allow-listed again.
///
/// Admin only. Call after the external governor contract is upgraded.
pub fn reauthorize_governor(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();
    bump_epoch(env);
    emit_admin_action(env, symbol_short!("gov_reset"), admin, Bytes::new(env));
}

/// Allow-lists `operation` for the external governor.
///
/// Admin only.
pub fn allow_governor_op(env: &Env, operation: String) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::GovernorOpGrant(operation);
    env.storage()
        .persistent()
        .set(&key, &get_governor_epoch(env));
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Removes `operation` from the governor's allow-list.
///
/// Admin only. Removing an operation that was never allow-listed is a no-op.
pub fn disallow_governor_op(env: &Env, operation: String) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .remove(&DataKey::GovernorOpGrant(operation));
}

/// Returns whether the external governor may perform `operation`.
pub fn is_governor_op_allowed(env: &Env, operation: String) -> bool {
    let granted: Option<u32> = env
        .storage()
        .persistent()
        .get(&DataKey::GovernorOpGrant(operation));
    granted == Some(get_governor_epoch(env))
}
