//! Storage migration mechanism for contract upgrades (Issue #112).
//!
//! # Design
//!
//! Storage migrations are executed incrementally to work within Soroban's gas
//! limits.  A single [`migrate_storage`] call processes up to `batch_size` items
//! and then either completes or leaves a [`MigrationState`] cursor so the next
//! call can resume exactly where the previous one stopped.
//!
//! ## Version history
//!
//! | Version | Description |
//! |---------|-------------|
//! | 1       | Initial storage layout (baseline; no migration needed). |
//! | 2       | Example extension — adds `decimals` field to `PriceEntry` when absent. |
//!
//! ## Adding a new version
//!
//! 1. Increment `CURRENT_VERSION`.
//! 2. Add a branch in [`run_migration_step`] for `(old, new)`.
//! 3. Implement the data-transformation logic.
//! 4. Add tests in `migration_tests` below.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{MigrationCompletedEvent, MigrationResumedEvent, MigrationStartedEvent};
use crate::storage::{get_admin, read_registered_assets, read_oracle_sources, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, MigrationState, MigrationStatus};

/// The storage schema version that this build of the contract targets.
/// Increment this when releasing a schema-changing upgrade.
pub const CURRENT_VERSION: u32 = 2;

/// Maximum items processed per [`migrate_storage`] call when the caller does not
/// supply a `batch_size`.
const DEFAULT_BATCH_SIZE: u32 = 50;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Returns the current storage schema version stored on-chain.
///
/// Returns `1` when no version key is present (contracts deployed before #112).
pub fn get_storage_version(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::StorageVersion)
        .unwrap_or(1)
}

/// Returns the active [`MigrationState`], or `None` when no migration is running.
pub fn get_migration_state(env: &Env) -> Option<MigrationState> {
    env.storage()
        .persistent()
        .get(&DataKey::MigrationState)
}

/// Starts or resumes a storage migration.
///
/// The admin must authorize this call.
///
/// * If no migration is in progress, a new one is started from
///   `get_storage_version()` → `CURRENT_VERSION`.
/// * If a migration is already in progress it is resumed from the stored cursor.
/// * Each call processes at most `batch_size` items.  When `batch_size` is `0`
///   [`DEFAULT_BATCH_SIZE`] is used.
/// * When all items have been processed the version key is updated, the
///   migration state is removed, and a [`MigrationCompletedEvent`] is emitted.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`]        — caller is not the admin.
/// * [`ErrorCode::MigrationInProgress`]  — (currently unused; reserved for future
///   guard if re-entry detection is desired).
pub fn migrate_storage(env: &Env, batch_size: u32) {
    let admin: Address = get_admin(env);
    admin.require_auth();

    let effective_batch = if batch_size == 0 { DEFAULT_BATCH_SIZE } else { batch_size };

    let state_opt = get_migration_state(env);

    let mut state = match state_opt {
        Some(s) => {
            // Resuming an in-progress migration.
            MigrationResumedEvent {
                admin: admin.clone(),
                cursor: s.cursor,
            }
            .publish(env);
            s
        }
        None => {
            // Start a fresh migration.
            let from = get_storage_version(env);
            let to = CURRENT_VERSION;

            if from >= to {
                // Already up-to-date; write the version key if absent and return.
                env.storage()
                    .persistent()
                    .set(&DataKey::StorageVersion, &CURRENT_VERSION);
                return;
            }

            let new_state = MigrationState {
                from_version: from,
                to_version: to,
                cursor: 0,
                started_ledger: env.ledger().sequence(),
                status: MigrationStatus::InProgress,
            };

            MigrationStartedEvent {
                admin: admin.clone(),
                from_version: from,
                to_version: to,
                started_ledger: new_state.started_ledger,
            }
            .publish(env);

            new_state
        }
    };

    // Run the migration step for this batch.
    let (new_cursor, done) = run_migration_step(env, &state, effective_batch);

    if done {
        // Migration complete — update version, remove state, emit event.
        env.storage()
            .persistent()
            .set(&DataKey::StorageVersion, &state.to_version);
        env.storage()
            .persistent()
            .remove(&DataKey::MigrationState);

        MigrationCompletedEvent {
            admin,
            from_version: state.from_version,
            to_version: state.to_version,
            items_processed: new_cursor,
        }
        .publish(env);
    } else {
        // Pause — save progress for the next call.
        state.cursor = new_cursor;
        env.storage()
            .persistent()
            .set(&DataKey::MigrationState, &state);
        env.storage().persistent().extend_ttl(
            &DataKey::MigrationState,
            LEDGER_THRESHOLD,
            LEDGER_BUMP,
        );
    }
}

// ---------------------------------------------------------------------------
// Internal migration logic
// ---------------------------------------------------------------------------

/// Executes one batch of the migration described by `state`.
///
/// Returns `(new_cursor, finished)`.
///
/// * `new_cursor` — the index of the next unprocessed item (cumulative across
///   all calls for this migration run).
/// * `finished`   — `true` when every item has been processed.
fn run_migration_step(env: &Env, state: &MigrationState, batch_size: u32) -> (u32, bool) {
    match (state.from_version, state.to_version) {
        (1, 2) => migrate_v1_to_v2(env, state.cursor, batch_size),
        // Future migrations: (2, 3) => migrate_v2_to_v3(...)
        _ => {
            // No-op migration — nothing to transform; mark as done.
            (state.cursor, true)
        }
    }
}

/// Migration from v1 → v2.
///
/// In this example migration the task is to ensure every registered asset has a
/// `StorageVersion`-aware aggregate entry.  Concretely we iterate over the
/// registered asset list and confirm/re-write the `Aggregate` entry for any
/// asset whose aggregate is missing, so downstream code always finds a typed
/// record after the migration.
///
/// Returning `(cursor, done)`.
fn migrate_v1_to_v2(env: &Env, cursor: u32, batch_size: u32) -> (u32, bool) {
    let assets = read_registered_assets(env);
    let total = assets.len();

    if cursor >= total {
        return (cursor, true);
    }

    let end = (cursor + batch_size).min(total);

    for i in cursor..end {
        let asset: Address = assets.get_unchecked(i);

        // For each asset that has no aggregate stored yet, initialize a zero
        // aggregate so consumers never encounter a missing key after migration.
        let agg_key = DataKey::Aggregate(asset.clone());
        if !env.storage().persistent().has(&agg_key) {
            let zero_agg = crate::types::AggregatePrice {
                price: 0,
                timestamp: 0,
                num_sources: 0,
                decimals: crate::admin::get_decimals(env),
                is_override: false,
            };
            env.storage().persistent().set(&agg_key, &zero_agg);
        }

        // Extend TTL so the entry survives through the new schema epoch.
        env.storage()
            .persistent()
            .extend_ttl(&agg_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    let new_cursor = end;
    let done = new_cursor >= total;
    (new_cursor, done)
}
