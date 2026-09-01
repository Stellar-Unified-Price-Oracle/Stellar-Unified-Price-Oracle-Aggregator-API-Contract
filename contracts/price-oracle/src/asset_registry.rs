//! # Canonical Cross-Chain Asset Registry
//!
//! A single source of truth mapping a Stellar asset address to its
//! representation(s) on foreign chains (contract/token address, decimals).
//! Cross-chain modules — Axelar GMP ([`crate::axelar_gmp`]), LayerZero
//! ([`crate::layerzero`]), and manual cross-reference checks
//! ([`crate::cross_chain_verify`]) — resolve foreign asset identifiers
//! through this registry instead of maintaining their own ad-hoc mappings.
//!
//! ## Schema
//!
//! A mapping is keyed by `(chain, foreign_address)`:
//! * `chain` — a short canonical chain identifier (e.g. `"ethereum"`,
//!   `"polygon"`). Bridge integrations that identify chains differently
//!   (LayerZero's numeric `src_eid`, for example) translate their own
//!   identifier onto this canonical namespace rather than inventing a
//!   parallel one — see [`crate::layerzero::set_lz_chain_name`].
//! * `foreign_address` — the 32-byte canonical representation of the
//!   asset's contract/token address on that chain (left-padded for
//!   shorter addresses such as 20-byte EVM addresses).
//!
//! Each mapping also carries the foreign asset's `decimals` so callers can
//! rescale a price without a second lookup, and an `enabled` flag so admins
//! can suspend a mapping without losing its history.

use soroban_sdk::{panic_with_error, Address, BytesN, Env, String, Vec};

use crate::events::{
    ForeignAssetMappedEvent, ForeignAssetMappingRemovedEvent, ForeignAssetMappingUpdatedEvent,
};
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, ForeignAssetMapping};

fn mapping_key(chain: &String, foreign_address: &BytesN<32>) -> DataKey {
    DataKey::ForeignAssetMapping(chain.clone(), foreign_address.clone())
}

fn append_index(env: &Env, key: DataKey, entry: (String, BytesN<32>)) {
    let mut list: Vec<(String, BytesN<32>)> = env.storage().persistent().get(&key).unwrap_or(Vec::new(env));
    list.push_back(entry);
    env.storage().persistent().set(&key, &list);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn remove_from_index(env: &Env, key: DataKey, entry: &(String, BytesN<32>)) {
    let list: Vec<(String, BytesN<32>)> = env.storage().persistent().get(&key).unwrap_or(Vec::new(env));
    let mut updated: Vec<(String, BytesN<32>)> = Vec::new(env);
    for item in list.iter() {
        if item != *entry {
            updated.push_back(item);
        }
    }
    env.storage().persistent().set(&key, &updated);
}

/// Registers a new canonical mapping from `stellar_asset` to its representation
/// on `chain`. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — `stellar_asset` is not a registered asset.
/// * [`ErrorCode::ForeignAssetAlreadyMapped`] — a mapping already exists for
///   `(chain, foreign_address)`.
pub fn register_foreign_asset_mapping(
    env: &Env,
    stellar_asset: Address,
    chain: String,
    foreign_address: BytesN<32>,
    decimals: u32,
) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &stellar_asset);

    let key = mapping_key(&chain, &foreign_address);
    if env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::ForeignAssetAlreadyMapped);
    }

    let mapping = ForeignAssetMapping {
        stellar_asset: stellar_asset.clone(),
        chain: chain.clone(),
        foreign_address: foreign_address.clone(),
        decimals,
        enabled: true,
    };
    env.storage().persistent().set(&key, &mapping);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    append_index(
        env,
        DataKey::ForeignAssetRegistryList,
        (chain.clone(), foreign_address.clone()),
    );
    append_index(
        env,
        DataKey::AssetForeignMappings(stellar_asset.clone()),
        (chain.clone(), foreign_address.clone()),
    );

    ForeignAssetMappedEvent {
        stellar_asset,
        chain,
        foreign_address,
        decimals,
    }
    .publish(env);
}

/// Updates an existing mapping's `decimals` and `enabled` flag. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::ForeignAssetNotMapped`] — no mapping exists for `(chain, foreign_address)`.
pub fn update_foreign_asset_mapping(
    env: &Env,
    chain: String,
    foreign_address: BytesN<32>,
    decimals: u32,
    enabled: bool,
) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = mapping_key(&chain, &foreign_address);
    let mut mapping: ForeignAssetMapping = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ForeignAssetNotMapped));

    mapping.decimals = decimals;
    mapping.enabled = enabled;
    env.storage().persistent().set(&key, &mapping);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    ForeignAssetMappingUpdatedEvent {
        stellar_asset: mapping.stellar_asset,
        chain,
        foreign_address,
        decimals,
        enabled,
    }
    .publish(env);
}

/// Permanently removes a foreign asset mapping. Admin-only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::ForeignAssetNotMapped`] — no mapping exists for `(chain, foreign_address)`.
pub fn remove_foreign_asset_mapping(env: &Env, chain: String, foreign_address: BytesN<32>) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = mapping_key(&chain, &foreign_address);
    let mapping: ForeignAssetMapping = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ForeignAssetNotMapped));

    env.storage().persistent().remove(&key);

    let entry = (chain.clone(), foreign_address.clone());
    remove_from_index(env, DataKey::ForeignAssetRegistryList, &entry);
    remove_from_index(
        env,
        DataKey::AssetForeignMappings(mapping.stellar_asset.clone()),
        &entry,
    );

    ForeignAssetMappingRemovedEvent {
        stellar_asset: mapping.stellar_asset,
        chain,
        foreign_address,
    }
    .publish(env);
}

/// Returns the mapping for `(chain, foreign_address)`, if any — regardless of
/// its `enabled` state.
pub fn get_foreign_asset_mapping(
    env: &Env,
    chain: String,
    foreign_address: BytesN<32>,
) -> Option<ForeignAssetMapping> {
    env.storage().persistent().get(&mapping_key(&chain, &foreign_address))
}

/// Returns every foreign-chain mapping currently registered for `asset`.
pub fn get_foreign_mappings_for_asset(env: &Env, asset: Address) -> Vec<ForeignAssetMapping> {
    let keys: Vec<(String, BytesN<32>)> = env
        .storage()
        .persistent()
        .get(&DataKey::AssetForeignMappings(asset))
        .unwrap_or(Vec::new(env));

    let mut result = Vec::new(env);
    for (chain, foreign_address) in keys.iter() {
        if let Some(mapping) = get_foreign_asset_mapping(env, chain, foreign_address) {
            result.push_back(mapping);
        }
    }
    result
}

/// Resolves an *enabled* mapping for `(chain, foreign_address)`, panicking if
/// the asset has no mapping or the mapping has been disabled.
///
/// Used internally by bridge integrations ([`crate::axelar_gmp`],
/// [`crate::layerzero`]) to turn a wire-format foreign asset id into the
/// Stellar asset it should update.
///
/// # Errors
///
/// * [`ErrorCode::ForeignAssetNotMapped`] — no mapping exists.
/// * [`ErrorCode::ForeignAssetMappingDisabled`] — the mapping exists but is disabled.
pub(crate) fn resolve_enabled_mapping(
    env: &Env,
    chain: &String,
    foreign_address: &BytesN<32>,
) -> ForeignAssetMapping {
    let mapping: ForeignAssetMapping = env
        .storage()
        .persistent()
        .get(&mapping_key(chain, foreign_address))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ForeignAssetNotMapped));

    if !mapping.enabled {
        panic_with_error!(env, ErrorCode::ForeignAssetMappingDisabled);
    }
    mapping
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    fn setup(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let asset = Address::generate(env);
        crate::admin::initialize(
            env,
            admin.clone(),
            1,
            100,
            18,
            String::from_str(env, "Oracle"),
        );
        crate::assets::register_asset(env, asset.clone());
        (admin, asset)
    }

    #[test]
    fn test_register_and_resolve_mapping() {
        let env = Env::default();
        let (_, asset) = setup(&env);

        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[7u8; 32]);

        register_foreign_asset_mapping(&env, asset.clone(), chain.clone(), foreign_address.clone(), 6);

        let mapping = resolve_enabled_mapping(&env, &chain, &foreign_address);
        assert_eq!(mapping.stellar_asset, asset);
        assert_eq!(mapping.decimals, 6);
        assert!(mapping.enabled);
    }

    #[test]
    #[should_panic]
    fn test_duplicate_mapping_rejected() {
        let env = Env::default();
        let (_, asset) = setup(&env);

        let chain = String::from_str(&env, "ethereum");
        let foreign_address = BytesN::from_array(&env, &[1u8; 32]);
        register_foreign_asset_mapping(&env, asset.clone(), chain.clone(), foreign_address.clone(), 6);
        register_foreign_asset_mapping(&env, asset, chain, foreign_address, 6);
    }

    #[test]
    fn test_disable_mapping_updates_flag() {
        let env = Env::default();
        let (_, asset) = setup(&env);

        let chain = String::from_str(&env, "polygon");
        let foreign_address = BytesN::from_array(&env, &[2u8; 32]);
        register_foreign_asset_mapping(&env, asset, chain.clone(), foreign_address.clone(), 18);

        update_foreign_asset_mapping(&env, chain.clone(), foreign_address.clone(), 18, false);

        let mapping = get_foreign_asset_mapping(&env, chain, foreign_address).unwrap();
        assert!(!mapping.enabled);
    }

    #[test]
    #[should_panic]
    fn test_disabled_mapping_rejected_by_bridges() {
        let env = Env::default();
        let (_, asset) = setup(&env);

        let chain = String::from_str(&env, "polygon");
        let foreign_address = BytesN::from_array(&env, &[2u8; 32]);
        register_foreign_asset_mapping(&env, asset, chain.clone(), foreign_address.clone(), 18);
        update_foreign_asset_mapping(&env, chain.clone(), foreign_address.clone(), 18, false);

        resolve_enabled_mapping(&env, &chain, &foreign_address);
    }

    #[test]
    fn test_remove_mapping() {
        let env = Env::default();
        let (_, asset) = setup(&env);

        let chain = String::from_str(&env, "avalanche");
        let foreign_address = BytesN::from_array(&env, &[3u8; 32]);
        register_foreign_asset_mapping(&env, asset.clone(), chain.clone(), foreign_address.clone(), 18);
        assert!(get_foreign_asset_mapping(&env, chain.clone(), foreign_address.clone()).is_some());

        remove_foreign_asset_mapping(&env, chain.clone(), foreign_address.clone());
        assert!(get_foreign_asset_mapping(&env, chain, foreign_address).is_none());

        let remaining = get_foreign_mappings_for_asset(&env, asset);
        assert_eq!(remaining.len(), 0);
    }

    #[test]
    fn test_multiple_chains_for_one_asset() {
        let env = Env::default();
        let (_, asset) = setup(&env);

        let eth = String::from_str(&env, "ethereum");
        let poly = String::from_str(&env, "polygon");
        register_foreign_asset_mapping(&env, asset.clone(), eth, BytesN::from_array(&env, &[4u8; 32]), 6);
        register_foreign_asset_mapping(&env, asset.clone(), poly, BytesN::from_array(&env, &[5u8; 32]), 18);

        let mappings = get_foreign_mappings_for_asset(&env, asset);
        assert_eq!(mappings.len(), 2);
    }
}
