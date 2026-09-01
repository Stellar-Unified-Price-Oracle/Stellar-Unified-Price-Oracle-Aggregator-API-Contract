/// Per-asset decimal precision configuration (#227)
///
/// Allows different assets to have different decimal precisions (e.g., BTC=8, USDC=6, tokens=18)
/// instead of being restricted to a single contract-wide setting.
use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env};

use crate::events::emit_admin_action;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AssetDecimalConfig, DataKey, ErrorCode};

/// Get the decimal precision for a specific asset.
/// Returns the asset-specific setting if configured, otherwise falls back to contract-wide decimals.
pub fn get_asset_decimals(env: &Env, asset: &Address) -> u32 {
    if let Some(config) = env
        .storage()
        .persistent()
        .get::<_, AssetDecimalConfig>(&DataKey::AssetDecimals(asset.clone()))
    {
        config.decimals
    } else {
        // Fall back to contract-wide decimals
        crate::admin::get_decimals(env)
    }
}

/// Set the decimal precision for a specific asset.
/// Only the admin can call this.
pub fn set_asset_decimals(env: &Env, asset: Address, decimals: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    // Validate asset is registered
    crate::storage::check_registered_asset(env, &asset);

    // Decimals must be valid (0-18 is reasonable for most tokens)
    if decimals > 18 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let config = AssetDecimalConfig {
        decimals,
        set_ledger: env.ledger().sequence(),
    };

    env.storage()
        .persistent()
        .set(&DataKey::AssetDecimals(asset.clone()), &config);

    env.storage().persistent().bump(
        &DataKey::AssetDecimals(asset),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    emit_admin_action(env, symbol_short!("set_dec"), admin, Bytes::new(env));
}

/// Get the decimal precision for an asset, considering per-asset overrides.
/// Used internally during price aggregation and storage.
pub fn get_effective_decimals(env: &Env, asset: &Address) -> u32 {
    get_asset_decimals(env, asset)
}

/// Clear the per-asset decimal override for an asset, reverting to contract-wide decimals.
pub fn clear_asset_decimals(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    crate::storage::check_registered_asset(env, &asset);

    env.storage()
        .persistent()
        .remove(&DataKey::AssetDecimals(asset.clone()));

    emit_admin_action(env, symbol_short!("clr_dec"), admin, Bytes::new(env));
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Address, Env};

    #[test]
    fn test_set_and_get_per_asset_decimals() {
        let env = Env::default();
        let admin = Address::random(&env);
        let asset = Address::random(&env);

        // Initialize contract
        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        // Register asset
        crate::assets::register_asset(&env, asset.clone());

        // Set per-asset decimals to 8
        set_asset_decimals(&env, asset.clone(), 8);

        // Verify the decimals are set
        assert_eq!(get_asset_decimals(&env, &asset), 8);
    }

    #[test]
    fn test_fallback_to_contract_decimals() {
        let env = Env::default();
        let admin = Address::random(&env);
        let asset = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        crate::assets::register_asset(&env, asset.clone());

        // Without setting per-asset decimals, should return contract-wide
        assert_eq!(get_asset_decimals(&env, &asset), 18);
    }

    #[test]
    fn test_clear_asset_decimals() {
        let env = Env::default();
        let admin = Address::random(&env);
        let asset = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        crate::assets::register_asset(&env, asset.clone());

        // Set and then clear
        set_asset_decimals(&env, asset.clone(), 8);
        assert_eq!(get_asset_decimals(&env, &asset), 8);

        clear_asset_decimals(&env, asset.clone());
        assert_eq!(get_asset_decimals(&env, &asset), 18); // Back to contract-wide
    }

    #[test]
    #[should_panic(expected = "InvalidConfiguration")]
    fn test_decimals_too_high() {
        let env = Env::default();
        let admin = Address::random(&env);
        let asset = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        crate::assets::register_asset(&env, asset.clone());

        // Try to set decimals > 18
        set_asset_decimals(&env, asset, 19);
    }
}
