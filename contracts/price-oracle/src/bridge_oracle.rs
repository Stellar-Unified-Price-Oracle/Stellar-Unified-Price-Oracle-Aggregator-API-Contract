use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String, Vec};

use crate::events::{emit_admin_action, BridgeOracleRegisteredEvent, BridgePriceSubmittedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{BridgeOracleConfig, BridgedPrice, DataKey, ErrorCode};

const MAX_BRIDGE_ASSETS: u32 = 256;

/// Registers a bridge oracle contract for a non-Stellar asset.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — config validation fails.
pub fn register_bridge_oracle(env: &Env, config: BridgeOracleConfig) {
    let admin = get_admin(env);
    admin.require_auth();

    if config.source_asset == config.target_asset {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let key = DataKey::BridgeOracle(config.source_asset.clone(), config.target_asset.clone());
    env.storage().persistent().set(&key, &config);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    BridgeOracleRegisteredEvent {
        source_asset: config.source_asset.clone(),
        target_asset: config.target_asset.clone(),
        oracle_contract: config.oracle_contract.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("reg_brdg"), admin, Bytes::new(env));
}

/// Returns the bridge oracle configuration for an asset pair, or `None`.
pub fn get_bridge_oracle(
    env: &Env,
    source_asset: Address,
    target_asset: Address,
) -> Option<BridgeOracleConfig> {
    let key = DataKey::BridgeOracle(source_asset, target_asset);
    env.storage().persistent().get(&key)
}

/// Submits a bridged price observation for an asset.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the bridge oracle contract.
/// * [`ErrorCode::InvalidConfiguration`] — price is non-positive.
pub fn submit_bridged_price(
    env: &Env,
    source_asset: Address,
    target_asset: Address,
    price: i128,
    timestamp: u64,
) {
    let config = get_bridge_oracle(env, source_asset.clone(), target_asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::InvalidConfiguration));

    let oracle = &config.oracle_contract;
    let current = env.current_contract_address();
    if *oracle != current {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let decimals = crate::admin::get_decimals(env);
    let normalized = normalize_bridged_price(env, price, decimals, &config);

    let entry = BridgedPrice {
        asset: source_asset.clone(),
        price: normalized,
        timestamp,
        decimals,
        source_contract: config.oracle_contract.clone(),
    };

    env.storage().persistent().set(
        &DataKey::BridgedPrice(source_asset.clone(), target_asset.clone()),
        &entry,
    );
    env.storage().persistent().extend_ttl(
        &DataKey::BridgedPrice(source_asset.clone(), target_asset.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    BridgePriceSubmittedEvent {
        asset: source_asset,
        price: normalized,
        timestamp,
        decimals,
    }
    .publish(env);
}

/// Normalizes a raw bridge price into the oracle's decimal scale.
pub fn normalize_bridged_price(
    env: &Env,
    raw_price: i128,
    target_decimals: u32,
    config: &BridgeOracleConfig,
) -> i128 {
    let source_decimals = config.decimals;
    if source_decimals == target_decimals {
        return raw_price;
    }

    if source_decimals < target_decimals {
        let factor = 10i128.pow(target_decimals - source_decimals);
        raw_price.saturating_mul(factor)
    } else {
        let factor = 10i128.pow(source_decimals - target_decimals);
        raw_price.saturating_div(factor)
    }
}

/// Returns the latest bridged price for an asset pair, or `None`.
pub fn get_bridged_price(
    env: &Env,
    source_asset: Address,
    target_asset: Address,
) -> Option<BridgedPrice> {
    let key = DataKey::BridgedPrice(source_asset, target_asset);
    env.storage().persistent().get(&key)
}
