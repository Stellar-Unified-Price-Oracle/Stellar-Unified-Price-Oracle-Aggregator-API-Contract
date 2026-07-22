use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, Vec};

use crate::events::{emit_admin_action, AssetRegisteredEvent, AssetUnregisteredEvent};
use crate::storage::{
    get_admin, read_registered_assets, write_registered_assets, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{AssetMetadata, DataKey, ErrorCode};

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

    AssetRegisteredEvent {
        asset: asset.clone(),
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("reg_asset"), admin, Bytes::new(env));
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
    let key = DataKey::AssetMinPrice(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0)
}
