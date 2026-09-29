//! Price freeze mechanism for market emergencies (#223)
//!
//! Lets the admin freeze an asset's aggregate price during a market disruption
//! event (flash crash, exchange hack, etc). While frozen, `get_price` returns the
//! frozen snapshot regardless of new activity, and price submissions are rejected
//! until the asset is explicitly unfrozen.

use soroban_sdk::{panic_with_error, Address, Env, String};

use crate::events::{PriceFrozenEvent, PriceUnfrozenEvent};
use crate::storage::{check_registered_asset, get_admin};
use crate::types::{AggregatePrice, DataKey, ErrorCode, FrozenPrice};

/// Freezes the current aggregate price for an asset, preventing further updates.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
/// * [`ErrorCode::ReasonTooLong`] — reason exceeds 256 characters.
/// * [`ErrorCode::PriceFrozen`] — asset is already frozen.
/// * [`ErrorCode::NoData`] — asset has no aggregate price to freeze yet.
pub fn freeze_price(env: &Env, asset: Address, reason: String) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    if reason.len() > 256 {
        panic_with_error!(env, ErrorCode::ReasonTooLong);
    }

    let key = DataKey::FrozenPrice(asset.clone());
    if env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::PriceFrozen);
    }

    let aggregate: AggregatePrice = env
        .storage()
        .persistent()
        .get(&DataKey::Aggregate(asset.clone()))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));

    let current_ledger = env.ledger().sequence();
    let frozen = FrozenPrice {
        price: aggregate.price,
        timestamp: aggregate.timestamp,
        decimals: aggregate.decimals,
        reason: reason.clone(),
        frozen_at_ledger: current_ledger,
    };
    env.storage().persistent().set(&key, &frozen);

    PriceFrozenEvent {
        asset,
        reason,
        price: aggregate.price,
        frozen_at_ledger: current_ledger,
    }
    .publish(env);
}

/// Unfreezes a previously frozen asset, resuming normal price updates.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::PriceNotFrozen`] — asset is not currently frozen.
pub fn unfreeze_price(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::FrozenPrice(asset.clone());
    if !env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::PriceNotFrozen);
    }
    env.storage().persistent().remove(&key);

    PriceUnfrozenEvent {
        asset,
        unfrozen_at_ledger: env.ledger().sequence(),
    }
    .publish(env);
}

/// Returns whether an asset's price is currently frozen.
pub fn is_price_frozen(env: &Env, asset: Address) -> bool {
    env.storage().persistent().has(&DataKey::FrozenPrice(asset))
}

/// Returns the frozen price snapshot for an asset, if one is active.
pub fn get_frozen_price(env: &Env, asset: Address) -> Option<FrozenPrice> {
    env.storage().persistent().get(&DataKey::FrozenPrice(asset))
}

/// Panics with [`ErrorCode::PriceFrozen`] if `asset` is currently frozen.
///
/// Called by every price-submission entry point to enforce the freeze.
pub fn check_not_frozen(env: &Env, asset: &Address) {
    if env
        .storage()
        .persistent()
        .has(&DataKey::FrozenPrice(asset.clone()))
    {
        panic_with_error!(env, ErrorCode::PriceFrozen);
    }
}
