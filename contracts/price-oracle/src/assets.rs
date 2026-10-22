use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, Vec};

use crate::events::{
    emit_admin_action, AssetRegisteredEvent, AssetUnregisteredEvent, CircuitBreakerEventEntry,
    CircuitBreakerResetEvent, CircuitBreakerTrippedEvent,
};
use crate::storage::{
    get_admin, read_registered_assets, write_registered_assets, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{AssetMetadata, AssetMetadataUpdate, DataKey, ErrorCode, PriceBounds};

pub const MAX_ASSET_QUERY_SIZE: u32 = 100;

pub fn register_asset(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if env
        .storage()
        .persistent()
        .has(&DataKey::AssetRegistered(asset.clone()))
    {
        panic_with_error!(env, ErrorCode::AssetAlreadyRegistered);
    }

    let max_assets: u32 = crate::admin::get_max_assets(env);
    let mut assets = read_registered_assets(env);
    if assets.len() as u32 >= max_assets {
        panic_with_error!(env, ErrorCode::MaxAssetsReached);
    }

    env.storage()
        .persistent()
        .set(&DataKey::AssetRegistered(asset.clone()), &true);

    // O(1) membership index (new): keep in sync with the Vec.
    env.storage()
        .persistent()
        .set(&DataKey::AssetRegistryIndex(asset.clone()), &true);

    assets.push_back(asset.clone());
    write_registered_assets(env, &assets);
    initialize_asset_price_bounds(env, asset.clone());

    AssetRegisteredEvent {
        asset: asset.clone(),
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("reg_asset"), admin, Bytes::new(env));
}

pub fn get_registered_assets_paginated(
    env: &Env,
    cursor: Option<Address>,
    limit: u32,
) -> (Vec<Address>, Option<Address>) {
    if limit == 0 || limit > crate::history::MAX_PAGE_SIZE {
        panic_with_error!(env, ErrorCode::InvalidPageSize);
    }

    let registered = read_registered_assets(env);
    let start = match cursor {
        Some(last_asset) => {
            let mut next_index = None;
            for i in 0..registered.len() {
                if registered.get_unchecked(i) == last_asset {
                    next_index = Some(i + 1);
                    break;
                }
            }
            match next_index {
                Some(index) => index,
                None => panic_with_error!(env, ErrorCode::InvalidPageCursor),
            }
        }
        None => 0,
    };

    let mut page: Vec<Address> = Vec::new(env);
    let mut index = start;
    while index < registered.len() && page.len() < limit {
        page.push_back(registered.get_unchecked(index));
        index += 1;
    }

    let next_cursor = if index < registered.len() {
        Some(page.get_unchecked(page.len() - 1))
    } else {
        None
    };
    (page, next_cursor)
}

pub fn unregister_asset(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);
    env.storage()
        .persistent()
        .remove(&DataKey::AssetRegistered(asset.clone()));

    // O(1) membership index (new).
    env.storage()
        .persistent()
        .remove(&DataKey::AssetRegistryIndex(asset.clone()));

    env.storage()
        .persistent()
        .remove(&DataKey::Aggregate(asset.clone()));

    let assets = read_registered_assets(env);
    let mut new_assets: Vec<Address> = Vec::new(env);
    for i in 0..assets.len() {
        let a = assets.get_unchecked(i);
        if a != asset {
            new_assets.push_back(a);
        }
    }
    write_registered_assets(env, &new_assets);
    AssetUnregisteredEvent {
        asset: asset.clone(),
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("unreg_ast"), admin, Bytes::new(env));
}

pub fn is_asset_registered(env: &Env, asset: Address) -> bool {
    // Prefer the O(1) index. For backwards compatibility with older
    // deployments, fall back to the legacy `AssetRegistered(addr)` flag and
    // lazily (re)build the index when needed.
    let index_key = DataKey::AssetRegistryIndex(asset.clone());
    let indexed: bool = env.storage().persistent().get(&index_key).unwrap_or(false);
    if indexed {
        env.storage()
            .persistent()
            .extend_ttl(&index_key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return true;
    }

    let legacy_key = DataKey::AssetRegistered(asset.clone());
    let exists: bool = env.storage().persistent().get(&legacy_key).unwrap_or(false);
    if exists {
        env.storage()
            .persistent()
            .extend_ttl(&legacy_key, LEDGER_THRESHOLD, LEDGER_BUMP);

        // Lazy migration: populate index entry.
        env.storage().persistent().set(&index_key, &true);
        env.storage()
            .persistent()
            .extend_ttl(&index_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    exists
}

#[allow(dead_code)]
pub fn set_asset_metadata(env: &Env, asset: Address, metadata: AssetMetadata) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);
    env.storage()
        .persistent()
        .set(&DataKey::AssetMetadata(asset.clone()), &metadata);

    crate::events::AssetMetadataUpdatedEvent {
        asset,
        admin: admin.clone(),
        name: metadata.name,
        symbol: metadata.symbol,
        decimals: metadata.decimals,
        logo_uri: Some(metadata.logo_uri),
    }
    .publish(env);
}

pub fn batch_set_asset_metadata(env: &Env, updates: Vec<AssetMetadataUpdate>) {
    let admin = get_admin(env);
    admin.require_auth();

    for i in 0..updates.len() {
        let update = updates.get_unchecked(i);
        crate::storage::check_registered_asset(env, &update.asset);

        let metadata = AssetMetadata {
            name: update.name.clone(),
            symbol: update.symbol.clone(),
            decimals: update.decimals,
            logo_uri: update.logo_uri.clone(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::AssetMetadata(update.asset.clone()), &metadata);

        crate::events::AssetMetadataUpdatedEvent {
            asset: update.asset.clone(),
            admin: admin.clone(),
            name: update.name,
            symbol: update.symbol,
            decimals: update.decimals,
            logo_uri: Some(update.logo_uri),
        }
        .publish(env);
    }
}

#[allow(dead_code)]
pub fn get_asset_metadata(env: &Env, asset: Address) -> Option<AssetMetadata> {
    crate::storage::check_registered_asset(env, &asset);
    let key = DataKey::AssetMetadata(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

#[allow(dead_code)]
pub fn set_min_price(env: &Env, asset: Address, min_price: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);
    env.storage()
        .persistent()
        .set(&DataKey::AssetMinPrice(asset.clone()), &min_price);
}

pub fn get_min_price(env: &Env, asset: Address) -> i128 {
    crate::storage::check_registered_asset(env, &asset);
    let bounds = get_price_bounds(env, asset.clone());
    if bounds.min_price > 0 {
        return bounds.min_price;
    }
    let key = DataKey::AssetMinPrice(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}

pub fn initialize_asset_price_bounds(env: &Env, asset: Address) {
    let bounds = PriceBounds {
        min_price: 0,
        max_price: i128::MAX,
        max_change_bps_per_ledger: 0,
    };
    env.storage()
        .persistent()
        .set(&DataKey::AssetPriceBounds(asset.clone()), &bounds);
    env.storage()
        .persistent()
        .set(&DataKey::AssetPauseFlag(asset.clone()), &false);
    env.storage()
        .persistent()
        .set(&DataKey::AssetCircuitBreakerTripped(asset.clone()), &false);
    env.storage()
        .persistent()
        .set(&DataKey::AssetCircuitBreakerLogCount(asset.clone()), &0u32);
}

pub fn set_price_bounds(
    env: &Env,
    asset: Address,
    min_price: i128,
    max_price: i128,
    max_change_bps_per_ledger: u32,
) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);
    if min_price > max_price {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    if max_change_bps_per_ledger > 100_000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let bounds = PriceBounds {
        min_price,
        max_price,
        max_change_bps_per_ledger,
    };
    env.storage()
        .persistent()
        .set(&DataKey::AssetPriceBounds(asset.clone()), &bounds);
    emit_admin_action(env, symbol_short!("st_bounds"), admin, Bytes::new(env));
}

pub fn get_price_bounds(env: &Env, asset: Address) -> PriceBounds {
    crate::storage::check_registered_asset(env, &asset);
    let key = DataK

/* … truncated 3334 chars — edit only what you need near the top … */
