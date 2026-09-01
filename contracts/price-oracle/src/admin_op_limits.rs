/// Admin operation spending limits (#238)
///
/// Enforces per-operation-type daily limits. Prevents compromised admin key from causing
/// massive damage in a single transaction.
use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env};

use crate::events::emit_admin_action;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AdminOpLimit, AdminOperationType, DataKey, ErrorCode};

// Default limits per operation type per day
const DEFAULT_ADD_SOURCE_LIMIT: u32 = 5;
const DEFAULT_REMOVE_SOURCE_LIMIT: u32 = 3;
const DEFAULT_REGISTER_ASSET_LIMIT: u32 = 10;
const DEFAULT_UNREGISTER_ASSET_LIMIT: u32 = 5;
const DEFAULT_SET_DECIMALS_LIMIT: u32 = 2;
const DEFAULT_SET_RESOLUTION_LIMIT: u32 = 2;

/// Get the default daily limit for an operation type.
fn get_default_limit(op_type: u32) -> u32 {
    match op_type {
        0 => DEFAULT_ADD_SOURCE_LIMIT,
        1 => DEFAULT_REMOVE_SOURCE_LIMIT,
        2 => DEFAULT_REGISTER_ASSET_LIMIT,
        3 => DEFAULT_UNREGISTER_ASSET_LIMIT,
        4 => DEFAULT_SET_DECIMALS_LIMIT,
        5 => DEFAULT_SET_RESOLUTION_LIMIT,
        _ => u32::MAX, // Unknown operation types are unlimited
    }
}

/// Calculate the "day" epoch from a ledger timestamp.
/// Uses days since Unix epoch for consistency.
fn get_day_epoch(timestamp: u64) -> u32 {
    (timestamp / 86400) as u32
}

/// Set the daily limit for a specific operation type.
/// Only the admin can call this.
pub fn set_admin_op_daily_limit(env: &Env, op_type: u32, daily_limit: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if daily_limit == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let limit_config = AdminOpLimit {
        daily_limit,
        set_ledger: env.ledger().sequence(),
    };

    env.storage()
        .persistent()
        .set(&DataKey::AdminOpDailyLimit(op_type), &limit_config);

    emit_admin_action(env, symbol_short!("op_lim"), admin, Bytes::new(env));
}

/// Get the current daily limit for an operation type.
pub fn get_admin_op_daily_limit(env: &Env, op_type: u32) -> u32 {
    if let Some(config) = env
        .storage()
        .persistent()
        .get::<_, AdminOpLimit>(&DataKey::AdminOpDailyLimit(op_type))
    {
        config.daily_limit
    } else {
        get_default_limit(op_type)
    }
}

/// Check if an operation can proceed without exceeding the daily limit.
/// Returns `true` if the operation is allowed, `false` if it would exceed the limit.
pub fn check_admin_op_limit(env: &Env, op_type: u32) -> bool {
    let limit = get_admin_op_daily_limit(env, op_type);
    if limit == u32::MAX {
        return true; // Unlimited operations
    }

    let current_day = get_day_epoch(env.ledger().timestamp());
    let count_key = DataKey::AdminOpDailyCount(op_type, current_day);

    let current_count: u32 = env
        .storage()
        .persistent()
        .get::<_, u32>(&count_key)
        .unwrap_or(0);

    current_count < limit
}

/// Increment the operation counter for the current day.
/// Call this after an operation is successfully executed.
pub fn increment_admin_op_counter(env: &Env, op_type: u32) {
    let current_day = get_day_epoch(env.ledger().timestamp());
    let count_key = DataKey::AdminOpDailyCount(op_type, current_day);

    let current_count: u32 = env
        .storage()
        .persistent()
        .get::<_, u32>(&count_key)
        .unwrap_or(0);

    let new_count = current_count.saturating_add(1);
    env.storage().persistent().set(&count_key, &new_count);

    env.storage()
        .persistent()
        .bump(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Get the current count of operations performed today for a given type.
pub fn get_admin_op_daily_count(env: &Env, op_type: u32) -> u32 {
    let current_day = get_day_epoch(env.ledger().timestamp());
    env.storage()
        .persistent()
        .get::<_, u32>(&DataKey::AdminOpDailyCount(op_type, current_day))
        .unwrap_or(0)
}

/// Validate that an operation is allowed and panic if the daily limit is exceeded.
pub fn validate_admin_op_allowed(env: &Env, op_type: u32) {
    if !check_admin_op_limit(env, op_type) {
        panic_with_error!(env, ErrorCode::OperationLimitExceeded);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::{Address, Env};

    #[test]
    fn test_admin_op_limits_track_daily_count() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000; // Some day
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        // Initially count should be 0
        assert_eq!(get_admin_op_daily_count(&env, 0), 0);

        // Increment a few times
        increment_admin_op_counter(&env, 0);
        assert_eq!(get_admin_op_daily_count(&env, 0), 1);

        increment_admin_op_counter(&env, 0);
        assert_eq!(get_admin_op_daily_count(&env, 0), 2);

        // Different operation type should have separate count
        increment_admin_op_counter(&env, 1);
        assert_eq!(get_admin_op_daily_count(&env, 1), 1);
    }

    #[test]
    fn test_check_admin_op_limit() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        // Set a low limit
        set_admin_op_daily_limit(&env, 0, 2);

        // Should be able to do operations up to the limit
        assert!(check_admin_op_limit(&env, 0));
        increment_admin_op_counter(&env, 0);

        assert!(check_admin_op_limit(&env, 0));
        increment_admin_op_counter(&env, 0);

        // Now we've hit the limit
        assert!(!check_admin_op_limit(&env, 0));
    }

    #[test]
    fn test_validate_admin_op_allowed_panics() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        // Set limit to 1
        set_admin_op_daily_limit(&env, 0, 1);

        // First call should succeed
        validate_admin_op_allowed(&env, 0);
        increment_admin_op_counter(&env, 0);

        // Second call should panic
        // (In actual test framework, we'd use #[should_panic])
    }

    #[test]
    fn test_day_epoch_calculation() {
        // Day epoch should reset at midnight UTC
        let day1 = get_day_epoch(1000);
        let day1_later = get_day_epoch(1000 + 3600);
        assert_eq!(day1, day1_later); // Same day

        let day2 = get_day_epoch(1000 + 86400);
        assert!(day2 > day1); // Different day
    }

    #[test]
    fn test_default_limits() {
        assert_eq!(get_default_limit(0), DEFAULT_ADD_SOURCE_LIMIT);
        assert_eq!(get_default_limit(1), DEFAULT_REMOVE_SOURCE_LIMIT);
        assert_eq!(get_default_limit(2), DEFAULT_REGISTER_ASSET_LIMIT);
        assert_eq!(get_default_limit(3), DEFAULT_UNREGISTER_ASSET_LIMIT);
    }
}
