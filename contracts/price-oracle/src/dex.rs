//! # Stellar DEX Price Integration (#280)
//!
//! Reads prices from Stellar DEX liquidity pools using constant-product reserves.
//! Prices are exposed as [`DexPrice`] observations and can be fed into the
//! aggregation pipeline with a configurable weight.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::storage::LEDGER_BUMP;
use crate::types::{DataKey, DexPrice, ErrorCode};

// -----------------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------------

fn dex_pool_key(asset_a: &Address, asset_b: &Address) -> DataKey {
    DataKey::DexPool(asset_a.clone(), asset_b.clone())
}

fn read_dex_pool(env: &Env, asset_a: &Address, asset_b: &Address) -> Option<(i128, i128)> {
    let key = dex_pool_key(asset_a, asset_b);
    let result: Option<(i128, i128)> = env.storage().persistent().get(&key);
    if result.is_some() {
        env.storage().persistent().extend_ttl(&key, 10_000, 40_000);
    }
    result
}

fn write_dex_pool(env: &Env, asset_a: &Address, asset_b: &Address, rx: i128, ry: i128) {
    let key = dex_pool_key(asset_a, asset_b);
    env.storage().persistent().set(&key, &(rx, ry));
    env.storage().persistent().extend_ttl(&key, 10_000, 40_000);
}

// -----------------------------------------------------------------------------
// Public API
// -----------------------------------------------------------------------------

/// Registers a DEX pool pair with initial reserves. Admin-only.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — either reserve is ≤ 0.
pub fn register_dex_pool(
    env: &Env,
    asset_a: Address,
    asset_b: Address,
    reserve_a: i128,
    reserve_b: i128,
) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    if reserve_a <= 0 || reserve_b <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    write_dex_pool(env, &asset_a, &asset_b, reserve_a, reserve_b);
}

/// Returns the current DEX price for `asset` against its paired asset.
///
/// Uses the constant-product formula: `price = reserve_out / reserve_in`.
/// Returns `None` if the pool is not registered.
pub fn get_dex_price(env: &Env, asset: Address) -> Option<DexPrice> {
    // Find registered DEX pool containing `asset`. In practice this is configured
    // off-chain; here we scan the limited set of known pairs for demonstration.
    let registered_pairs: Vec<(Address, Address)> = env
        .storage()
        .persistent()
        .get(&DataKey::RegisteredAssets)
        .map(|assets: Vec<Address>| {
            let mut pairs = Vec::new(env);
            for i in 0..assets.len() {
                let a = assets.get_unchecked(i);
                if a != asset {
                    pairs.push_back((asset.clone(), a.clone()));
                }
            }
            pairs
        })
        .unwrap_or_else(|| Vec::new(env));

    for pair in registered_pairs.iter() {
        if let Some((rx, ry)) = read_dex_pool(env, &pair.0, &pair.1) {
            let price = if pair.0 == asset {
                ry * 1_000_000_000_000_000_000u128 / rx as u128
            } else {
                rx * 1_000_000_000_000_000_000u128 / ry as u128
            };
            return Some(DexPrice {
                asset,
                price: price as i128,
                reserve_x: rx,
                reserve_y: ry,
                timestamp: env.ledger().timestamp(),
            });
        }
    }

    None
}
