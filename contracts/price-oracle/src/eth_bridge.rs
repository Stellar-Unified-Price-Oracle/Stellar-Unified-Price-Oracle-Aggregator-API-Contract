//! # Ethereum Bridge Price Source
//!
//! Consumes prices originating from Ethereum-based oracles (e.g. Chainlink
//! aggregators) via a trusted off-chain bridge relayer, mapping ERC-20 token
//! addresses to registered Stellar assets.
//!
//! ## Trust model
//!
//! Unlike `ibc_oracle`'s light-client verification, Ethereum finality is not
//! cheaply provable inside a Soroban contract (no BLS/secp256k1 beacon-chain
//! sync committee verification here), so this module follows the same
//! trusted-relayer pattern already used by `cross_chain_verify.rs` and
//! `bridge_oracle.rs` in this contract: a single authorized `relayer` address
//! (set by governance in [`EthBridgeConfig`]) submits price messages, and the
//! contract enforces:
//!
//! - **Finality**: `confirmations >= min_confirmations`.
//! - **Staleness**: `now - eth_block_timestamp <= max_staleness`.
//! - **Ordering**: `eth_block_number` must be strictly increasing per ERC-20
//!   address, rejecting out-of-order or duplicate relays.

use soroban_sdk::{panic_with_error, Address, BytesN, Env};

use crate::events::{EthAssetMappedEvent, EthBridgeConfigUpdatedEvent, EthPriceSubmittedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, EthBridgeConfig, EthBridgedPrice, EthPriceMessage};

/// Sets (or updates) the Ethereum bridge configuration. Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — `min_confirmations` or `max_staleness` is zero.
pub fn set_eth_bridge_config(env: &Env, config: EthBridgeConfig) {
    let admin = get_admin(env);
    admin.require_auth();

    if config.min_confirmations == 0 || config.max_staleness == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    env.storage()
        .persistent()
        .set(&DataKey::EthBridgeConfig, &config);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::EthBridgeConfig, LEDGER_THRESHOLD, LEDGER_BUMP);

    EthBridgeConfigUpdatedEvent {
        relayer: config.relayer,
        min_confirmations: config.min_confirmations,
        max_staleness: config.max_staleness,
    }
    .publish(env);
}

/// Returns the current Ethereum bridge configuration, or `None`.
pub fn get_eth_bridge_config(env: &Env) -> Option<EthBridgeConfig> {
    env.storage().persistent().get(&DataKey::EthBridgeConfig)
}

/// Maps an ERC-20 token address to a registered Stellar asset. Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not a registered oracle asset.
pub fn map_erc20_asset(env: &Env, erc20: BytesN<20>, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    crate::storage::check_registered_asset(env, &asset);

    let fwd_key = DataKey::EthAssetMapping(erc20.clone());
    env.storage().persistent().set(&fwd_key, &asset);
    env.storage()
        .persistent()
        .extend_ttl(&fwd_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let rev_key = DataKey::EthAssetReverse(asset.clone());
    env.storage().persistent().set(&rev_key, &erc20);
    env.storage()
        .persistent()
        .extend_ttl(&rev_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    EthAssetMappedEvent { asset, erc20 }.publish(env);
}

/// Returns the Stellar asset mapped to an ERC-20 address, or `None`.
pub fn get_asset_for_erc20(env: &Env, erc20: BytesN<20>) -> Option<Address> {
    env.storage()
        .persistent()
        .get(&DataKey::EthAssetMapping(erc20))
}

/// Returns the ERC-20 address mapped to a Stellar asset, or `None`.
pub fn get_erc20_for_asset(env: &Env, asset: Address) -> Option<BytesN<20>> {
    env.storage()
        .persistent()
        .get(&DataKey::EthAssetReverse(asset))
}

/// Submits a bridged Ethereum price update. Must be authorized by the
/// configured bridge relayer.
///
/// # Panics
/// * [`ErrorCode::EthBridgeNotConfigured`] — no bridge configuration set.
/// * [`ErrorCode::NotAuthorized`] — caller does not match the configured relayer.
/// * [`ErrorCode::InvalidPrice`] — `msg.price` is non-positive.
/// * [`ErrorCode::EthAssetNotMapped`] — the ERC-20 address has no asset mapping.
/// * [`ErrorCode::EthInsufficientFinality`] — `confirmations < min_confirmations`.
/// * [`ErrorCode::EthPriceStale`] — the ETH block timestamp is older than `max_staleness`.
/// * [`ErrorCode::EthOutOfOrder`] — `eth_block_number` did not advance.
pub fn submit_eth_price(env: &Env, msg: EthPriceMessage) {
    let config = get_eth_bridge_config(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::EthBridgeNotConfigured));
    config.relayer.require_auth();

    if msg.price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    if msg.confirmations < config.min_confirmations {
        panic_with_error!(env, ErrorCode::EthInsufficientFinality);
    }

    let now = env.ledger().timestamp();
    if now.saturating_sub(msg.eth_block_timestamp) > config.max_staleness {
        panic_with_error!(env, ErrorCode::EthPriceStale);
    }

    let asset = get_asset_for_erc20(env, msg.erc20.clone())
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::EthAssetNotMapped));

    let price_key = DataKey::EthLatestPrice(msg.erc20.clone());
    let previous: Option<EthBridgedPrice> = env.storage().persistent().get(&price_key);
    if let Some(prev) = &previous {
        if msg.eth_block_number <= prev.eth_block_number {
            panic_with_error!(env, ErrorCode::EthOutOfOrder);
        }
    }

    let decimals = crate::admin::get_decimals(env);
    let normalized_price = normalize_decimals(msg.price, msg.decimals, decimals);

    let entry = EthBridgedPrice {
        erc20: msg.erc20.clone(),
        asset: asset.clone(),
        price: normalized_price,
        decimals,
        eth_block_number: msg.eth_block_number,
        eth_block_timestamp: msg.eth_block_timestamp,
        received_ledger: env.ledger().sequence(),
    };

    env.storage().persistent().set(&price_key, &entry);
    env.storage()
        .persistent()
        .extend_ttl(&price_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    EthPriceSubmittedEvent {
        asset,
        price: normalized_price,
        eth_block_number: msg.eth_block_number,
        eth_block_timestamp: msg.eth_block_timestamp,
    }
    .publish(env);
}

/// Returns the latest bridged Ethereum price for an ERC-20 address, or `None`.
pub fn get_eth_price(env: &Env, erc20: BytesN<20>) -> Option<EthBridgedPrice> {
    env.storage()
        .persistent()
        .get(&DataKey::EthLatestPrice(erc20))
}

/// Returns the latest bridged Ethereum price for a mapped Stellar asset, or `None`.
pub fn get_eth_price_for_asset(env: &Env, asset: Address) -> Option<EthBridgedPrice> {
    let erc20 = get_erc20_for_asset(env, asset)?;
    get_eth_price(env, erc20)
}

fn normalize_decimals(raw_price: i128, source_decimals: u32, target_decimals: u32) -> i128 {
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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Env, String as SorobanString};

    fn setup(env: &Env) -> (Address, Address) {
        let admin = Address::generate(env);
        let relayer = Address::generate(env);
        env.ledger().with_mut(|l| l.timestamp = 10_000);
        crate::admin::initialize(
            env,
            admin.clone(),
            1,
            100,
            8,
            SorobanString::from_slice(env, "Oracle"),
        );
        (admin, relayer)
    }

    fn sample_erc20(env: &Env) -> BytesN<20> {
        BytesN::from_array(env, &[0xAA; 20])
    }

    #[test]
    fn test_submit_eth_price_end_to_end() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, relayer) = setup(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());

        set_eth_bridge_config(
            &env,
            EthBridgeConfig {
                relayer: relayer.clone(),
                min_confirmations: 12,
                max_staleness: 900,
            },
        );

        let erc20 = sample_erc20(&env);
        map_erc20_asset(&env, erc20.clone(), asset.clone());

        submit_eth_price(
            &env,
            EthPriceMessage {
                erc20: erc20.clone(),
                price: 350_000_000_000, // 3500.00000000 at 8 decimals
                decimals: 8,
                eth_block_number: 100,
                eth_block_timestamp: 9_800,
                confirmations: 15,
            },
        );

        let stored = get_eth_price(&env, erc20).unwrap();
        assert_eq!(stored.price, 350_000_000_000);
        assert_eq!(stored.asset, asset.clone());

        let by_asset = get_eth_price_for_asset(&env, asset).unwrap();
        assert_eq!(by_asset.eth_block_number, 100);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #125)")]
    fn test_insufficient_confirmations_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, relayer) = setup(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());
        set_eth_bridge_config(
            &env,
            EthBridgeConfig {
                relayer,
                min_confirmations: 12,
                max_staleness: 900,
            },
        );
        let erc20 = sample_erc20(&env);
        map_erc20_asset(&env, erc20.clone(), asset);

        submit_eth_price(
            &env,
            EthPriceMessage {
                erc20,
                price: 1_000_000,
                decimals: 8,
                eth_block_number: 1,
                eth_block_timestamp: 9_999,
                confirmations: 3,
            },
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #126)")]
    fn test_stale_block_timestamp_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, relayer) = setup(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());
        set_eth_bridge_config(
            &env,
            EthBridgeConfig {
                relayer,
                min_confirmations: 1,
                max_staleness: 60,
            },
        );
        let erc20 = sample_erc20(&env);
        map_erc20_asset(&env, erc20.clone(), asset);

        submit_eth_price(
            &env,
            EthPriceMessage {
                erc20,
                price: 1_000_000,
                decimals: 8,
                eth_block_number: 1,
                // ledger timestamp is 10_000; this is 1000s old, past max_staleness=60.
                eth_block_timestamp: 9_000,
                confirmations: 5,
            },
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #127)")]
    fn test_out_of_order_block_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, relayer) = setup(&env);

        let asset = Address::generate(&env);
        crate::assets::register_asset(&env, asset.clone());
        set_eth_bridge_config(
            &env,
            EthBridgeConfig {
                relayer,
                min_confirmations: 1,
                max_staleness: 900,
            },
        );
        let erc20 = sample_erc20(&env);
        map_erc20_asset(&env, erc20.clone(), asset);

        submit_eth_price(
            &env,
            EthPriceMessage {
                erc20: erc20.clone(),
                price: 1_000_000,
                decimals: 8,
                eth_block_number: 50,
                eth_block_timestamp: 9_900,
                confirmations: 5,
            },
        );

        // Older block number relayed second — must be rejected.
        submit_eth_price(
            &env,
            EthPriceMessage {
                erc20,
                price: 1_100_000,
                decimals: 8,
                eth_block_number: 49,
                eth_block_timestamp: 9_950,
                confirmations: 5,
            },
        );
    }
}
