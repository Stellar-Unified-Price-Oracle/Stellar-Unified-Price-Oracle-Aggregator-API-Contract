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
//! | `GovernorAllowedOp(name)` | `bool` | Allow-list flag per operation name |

use soroban_sdk::{Address, Env, String};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::DataKey;

/// Delegates governance to `governor`.
///
/// Admin only. Registering a governor grants no power on its own — every
/// operation must additionally be allow-listed with [`allow_governor_op`].
pub fn set_external_governor(env: &Env, governor: Address) {
    let admin = get_admin(env);
    admin.require_auth();

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
}

/// Allow-lists `operation` for the external governor.
///
/// Admin only.
pub fn allow_governor_op(env: &Env, operation: String) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::GovernorAllowedOp(operation);
    env.storage().persistent().set(&key, &true);
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
        .remove(&DataKey::GovernorAllowedOp(operation));
}

/// Returns whether the external governor may perform `operation`.
pub fn is_governor_op_allowed(env: &Env, operation: String) -> bool {
    env.storage()
        .persistent()
        .get(&DataKey::GovernorAllowedOp(operation))
        .unwrap_or(false)
}
