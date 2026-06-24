use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::admin::get_decimals;
use crate::events::{AssetRegisteredEvent, AssetUnregisteredEvent};
use crate::storage::{get_admin, read_registered_assets, write_registered_assets, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregatePrice, DataKey, ErrorCode};

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
    env.storage()
        .persistent()
        .set(&DataKey::AssetRegistered(asset.clone()), &true);
    env.storage().persistent().set(
        &DataKey::Aggregate(asset.clone()),
        &AggregatePrice {
            price: 0,
            timestamp: 0,
            num_sources: 0,
            decimals: get_decimals(env),
        },
    );
    let mut assets = read_registered_assets(env);
    assets.push_back(asset.clone());
    write_registered_assets(env, &assets);
    AssetRegisteredEvent {
        asset: asset.clone(),
    }
    .publish(env);
}

pub fn unregister_asset(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);
    env.storage()
        .persistent()
        .remove(&DataKey::AssetRegistered(asset.clone()));
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
    }
    .publish(env);
}

pub fn is_asset_registered(env: &Env, asset: Address) -> bool {
    let key = DataKey::AssetRegistered(asset);
    let exists: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if exists {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    exists
}
