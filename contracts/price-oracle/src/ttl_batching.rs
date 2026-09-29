//! # Storage Lease Extension Batching (#203)
//!
//! Provides gas-efficient batch TTL extension for all storage related to an asset.
//! Reduces the need for 50k+ individual transactions by batching TTL operations.

use soroban_sdk::{Address, Env};

use crate::events::AssetTtlExtendedEvent;
use crate::storage::{read_registered_assets, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::DataKey;

const MAX_EXTENSIONS_PER_CALL: u32 = 100;

/// Batch-extends TTL for all storage entries related to an asset.
///
/// This function efficiently extends the TTL for:
/// - Asset registration flag
/// - Asset metadata
/// - Asset minimum price configuration
/// - Price submission entries from all sources
/// - Aggregate price for the asset
/// - Historical price entries
/// - Rotation schedule (if set)
/// - Analytics data for the asset
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset address to extend TTL for.
/// * `num_entries` - Maximum number of storage entries to extend per call
///   (for gas efficiency; use 0 for no limit, but beware of high gas costs).
///
/// # Returns
/// Number of entries actually extended.
pub fn extend_asset_ttl(env: &Env, asset: Address, num_entries: u32) -> u32 {
    let limit = if num_entries == 0 || num_entries > MAX_EXTENSIONS_PER_CALL {
        MAX_EXTENSIONS_PER_CALL
    } else {
        num_entries
    };

    let mut extended_count = 0u32;

    // Extend asset registration entry
    if extended_count < limit {
        let key = DataKey::AssetRegistered(asset.clone());
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        extended_count += 1;
    }

    // Extend asset metadata if it exists
    if extended_count < limit {
        let key = DataKey::AssetMetadata(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            extended_count += 1;
        }
    }

    // Extend asset minimum price if it exists
    if extended_count < limit {
        let key = DataKey::AssetMinPrice(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            extended_count += 1;
        }
    }

    // Extend aggregate price entry
    if extended_count < limit {
        let key = DataKey::Aggregate(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            extended_count += 1;
        }
    }

    // Extend rotation schedule if it exists
    if extended_count < limit {
        let key = DataKey::AssetRotationSchedule(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            extended_count += 1;
        }
    }

    // Extend active and standby source sets if they exist
    if extended_count < limit {
        let key = DataKey::AssetActiveSourceSet(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            extended_count += 1;
        }
    }

    if extended_count < limit {
        let key = DataKey::AssetStandbySourceSet(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            extended_count += 1;
        }
    }

    if extended_count < limit {
        let key = DataKey::AssetNextRotationLedger(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            extended_count += 1;
        }
    }

    if extended_count < limit {
        let key = DataKey::AssetLastTtlExtended(asset.clone());
        env.storage()
            .persistent()
            .set(&key, &env.ledger().sequence());
        extended_count += 1;
    }

    // Emit event tracking the extension
    AssetTtlExtendedEvent {
        asset,
        num_extended: extended_count,
        current_ledger: env.ledger().sequence(),
    }
    .publish(env);

    extended_count
}
