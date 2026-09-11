//! Consumer Contract Authorization System (#304)
//!
//! Implements an allowlist/blocklist for consumer contracts querying price data.
//! The admin configures the global access mode and manages per-consumer records.
//!
//! ## Access Modes
//!
//! | Mode          | Description                                                |
//! |---------------|------------------------------------------------------------|
//! | `Public`      | No restriction — all callers may query prices (default).   |
//! | `AllowedOnly` | Only explicitly authorized consumers may query prices.     |
//! | `BlockedOnly` | All consumers may query prices except those explicitly     |
//! |               | added to the blocklist.                                    |

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{
    ConsumerAccessModeChangedEvent, ConsumerAuthorizedEvent, ConsumerDeauthorizedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{ConsumerAccessMode, DataKey, ErrorCode};

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Read the current access mode from instance storage, defaulting to `Public`.
pub fn read_access_mode(env: &Env) -> ConsumerAccessMode {
    env.storage()
        .instance()
        .get(&DataKey::ConsumerAccessMode)
        .unwrap_or(ConsumerAccessMode::Public)
}

/// Write the access mode to instance storage.
fn write_access_mode(env: &Env, mode: ConsumerAccessMode) {
    env.storage()
        .instance()
        .set(&DataKey::ConsumerAccessMode, &mode);
}

/// Return `true` when the consumer is recorded as explicitly authorized.
fn is_explicitly_authorized(env: &Env, consumer: &Address) -> bool {
    let key = DataKey::ConsumerAuthorized(consumer.clone());
    let flag: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if flag {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    flag
}

/// Return `true` when the consumer is recorded as explicitly blocked.
fn is_explicitly_blocked(env: &Env, consumer: &Address) -> bool {
    let key = DataKey::ConsumerBlocked(consumer.clone());
    let flag: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if flag {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    flag
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Grants explicit authorization to `consumer`.  Admin-only.
///
/// In `AllowedOnly` mode this consumer will be permitted to query prices.
/// In `Public` or `BlockedOnly` mode the flag is stored but has no effect
/// unless the mode is later changed.
pub fn add_authorized_consumer(env: &Env, consumer: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .set(&DataKey::ConsumerAuthorized(consumer.clone()), &true);
    env.storage().persistent().extend_ttl(
        &DataKey::ConsumerAuthorized(consumer.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    ConsumerAuthorizedEvent {
        consumer: consumer.clone(),
        admin,
    }
    .publish(env);
}

/// Removes the explicit authorization record for `consumer`.  Admin-only.
pub fn remove_authorized_consumer(env: &Env, consumer: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::ConsumerAuthorized(consumer.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }

    ConsumerDeauthorizedEvent {
        consumer: consumer.clone(),
        admin,
    }
    .publish(env);
}

/// Sets the global consumer access mode.  Admin-only.
///
/// Accepts a `u32` discriminant matching [`ConsumerAccessMode`]:
/// - `0` → `Public`
/// - `1` → `AllowedOnly`
/// - `2` → `BlockedOnly`
pub fn set_consumer_access_mode(env: &Env, mode: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let mode_enum = match mode {
        0 => ConsumerAccessMode::Public,
        1 => ConsumerAccessMode::AllowedOnly,
        2 => ConsumerAccessMode::BlockedOnly,
        _ => panic_with_error!(env, ErrorCode::InvalidConfiguration),
    };

    write_access_mode(env, mode_enum.clone());

    ConsumerAccessModeChangedEvent {
        admin,
        new_mode: mode_enum.clone(),
    }
    .publish(env);
}

/// Returns the current consumer access mode.
pub fn get_consumer_access_mode(env: &Env) -> ConsumerAccessMode {
    read_access_mode(env)
}

/// Returns whether `consumer` is currently authorized to query prices.
///
/// Resolution:
/// - `Public`      → always `true`
/// - `AllowedOnly` → `true` iff the consumer is in the allowlist
/// - `BlockedOnly` → `true` iff the consumer is **not** in the blocklist
pub fn is_consumer_authorized(env: &Env, consumer: &Address) -> bool {
    match read_access_mode(env) {
        ConsumerAccessMode::Public => true,
        ConsumerAccessMode::AllowedOnly => is_explicitly_authorized(env, consumer),
        ConsumerAccessMode::BlockedOnly => !is_explicitly_blocked(env, consumer),
    }
}

/// Panics with [`ErrorCode::NotAuthorized`] when `consumer` is not allowed to
/// query prices under the current access mode.
///
/// Call this at the start of any price-read endpoint that should be gated.
pub fn check_consumer_authorized(env: &Env, consumer: &Address) {
    if !is_consumer_authorized(env, consumer) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
}

/// Adds `consumer` to the blocklist.  Admin-only.
///
/// Effective in `BlockedOnly` mode: blocked consumers cannot query prices.
pub fn block_consumer(env: &Env, consumer: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .set(&DataKey::ConsumerBlocked(consumer.clone()), &true);
    env.storage().persistent().extend_ttl(
        &DataKey::ConsumerBlocked(consumer.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    // Re-use the deauthorized event to indicate the consumer was blocked.
    ConsumerDeauthorizedEvent { consumer, admin }.publish(env);
}

/// Removes `consumer` from the blocklist.  Admin-only.
pub fn unblock_consumer(env: &Env, consumer: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::ConsumerBlocked(consumer.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }

    ConsumerAuthorizedEvent { consumer, admin }.publish(env);
}
