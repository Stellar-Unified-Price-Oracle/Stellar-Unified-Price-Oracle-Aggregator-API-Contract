//! # Source Rotation Schedule (#206)
//!
//! Implements automated source rotation where subsets of sources are designated active
//! per asset and rotated on a schedule to reduce correlation risk and improve decentralization.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{SourceRotationSetEvent, SourcesRotatedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, SourceRotationSchedule};

/// Sets the rotation schedule for an asset's oracle sources.
///
/// Admin-only. Designates which sources should be active for an asset and when
/// rotation should occur. Supports a graceful overlap period where old and new
/// sets coexist briefly.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset address to configure rotation for.
/// * `sources` - Initial set of active sources for this asset.
/// * `rotation_interval` - Ledgers between rotations (must be > 0).
/// * `overlap_period` - Ledgers where old and new sets coexist (for graceful transition).
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — if caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] — if rotation_interval is 0 or sources is empty.
pub fn set_source_schedule(
    env: &Env,
    asset: Address,
    sources: Vec<Address>,
    rotation_interval: u32,
    overlap_period: u32,
) {
    let admin = get_admin(env);
    admin.require_auth();

    if rotation_interval == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    if sources.is_empty() {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let schedule = SourceRotationSchedule {
        rotation_interval,
        next_rotation_ledger: env.ledger().sequence() + rotation_interval,
        overlap_period,
        enabled: true,
    };

    let schedule_key = DataKey::AssetRotationSchedule(asset.clone());
    env.storage().persistent().set(&schedule_key, &schedule);
    env.storage()
        .persistent()
        .extend_ttl(&schedule_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Set active sources
    let active_key = DataKey::AssetActiveSourceSet(asset.clone());
    env.storage().persistent().set(&active_key, &sources);
    env.storage()
        .persistent()
        .extend_ttl(&active_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SourceRotationSetEvent {
        asset,
        rotation_interval,
        overlap_period,
        num_sources: sources.len() as u32,
    }
    .publish(env);
}

/// Performs immediate rotation for an asset if scheduled.
///
/// This is called internally to check if rotation should occur based on
/// ledger time. If the next_rotation_ledger has been reached, swaps the
/// active and standby source sets.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset to rotate sources for.
///
/// # Returns
/// `true` if rotation occurred, `false` if no rotation was needed.
pub fn attempt_rotation(env: &Env, asset: &Address) -> bool {
    let schedule_key = DataKey::AssetRotationSchedule(asset.clone());
    let schedule: Option<SourceRotationSchedule> = env.storage().persistent().get(&schedule_key);

    if let Some(mut sched) = schedule {
        if !sched.enabled {
            return false;
        }

        let current_ledger = env.ledger().sequence();
        if current_ledger < sched.next_rotation_ledger {
            return false; // Not yet time for rotation
        }

        // Get current active and standby sets
        let active_key = DataKey::AssetActiveSourceSet(asset.clone());
        let standby_key = DataKey::AssetStandbySourceSet(asset.clone());

        let current_active: Vec<Address> = env
            .storage()
            .persistent()
            .get(&active_key)
            .unwrap_or_else(Vec::new);

        let standby: Vec<Address> = env
            .storage()
            .persistent()
            .get(&standby_key)
            .unwrap_or_else(Vec::new);

        if standby.is_empty() {
            return false; // No standby set to rotate to
        }

        // Swap: standby becomes active, active becomes standby
        env.storage().persistent().set(&active_key, &standby);
        env.storage()
            .persistent()
            .extend_ttl(&active_key, LEDGER_THRESHOLD, LEDGER_BUMP);

        env.storage()
            .persistent()
            .set(&standby_key, &current_active);
        env.storage()
            .persistent()
            .extend_ttl(&standby_key, LEDGER_THRESHOLD, LEDGER_BUMP);

        // Update next rotation ledger
        sched.next_rotation_ledger = current_ledger + sched.rotation_interval;
        env.storage().persistent().set(&schedule_key, &sched);
        env.storage()
            .persistent()
            .extend_ttl(&schedule_key, LEDGER_THRESHOLD, LEDGER_BUMP);

        SourcesRotatedEvent {
            asset: asset.clone(),
            new_active_count: standby.len() as u32,
            next_rotation_ledger: sched.next_rotation_ledger,
        }
        .publish(env);

        true
    } else {
        false
    }
}

/// Returns the currently active source set for an asset.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset to query.
///
/// # Returns
/// Vector of currently active source addresses, or empty vector if no rotation schedule.
pub fn get_active_sources(env: &Env, asset: &Address) -> Vec<Address> {
    let key = DataKey::AssetActiveSourceSet(asset.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(Vec::new)
}

/// Returns the standby source set for an asset.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset to query.
///
/// # Returns
/// Vector of standby source addresses, or empty vector if none set.
pub fn get_standby_sources(env: &Env, asset: &Address) -> Vec<Address> {
    let key = DataKey::AssetStandbySourceSet(asset.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(Vec::new)
}

/// Returns the rotation schedule for an asset.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset to query.
///
/// # Returns
/// [`SourceRotationSchedule`] if one is configured, `None` otherwise.
pub fn get_rotation_schedule(env: &Env, asset: &Address) -> Option<SourceRotationSchedule> {
    let key = DataKey::AssetRotationSchedule(asset.clone());
    env.storage().persistent().get(&key)
}

/// Disables rotation for an asset. Admin-only.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset to disable rotation for.
pub fn disable_rotation(env: &Env, asset: &Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let schedule_key = DataKey::AssetRotationSchedule(asset.clone());
    if let Some(mut schedule) = env
        .storage()
        .persistent()
        .get::<_, SourceRotationSchedule>(&schedule_key)
    {
        schedule.enabled = false;
        env.storage().persistent().set(&schedule_key, &schedule);
        env.storage()
            .persistent()
            .extend_ttl(&schedule_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}
