/// Cross-chain price verification (#226)
///
/// Compares prices with the same oracle on other chains. Deviation beyond threshold
/// triggers alerts and freezes aggregation for that asset.

use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String};

use crate::events::emit_admin_action;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{CrossChainPriceEntry, DataKey, ErrorCode};

const DEFAULT_CROSS_CHAIN_DEVIATION_THRESHOLD: u32 = 500; // 5% in basis points

/// Enable or disable cross-chain verification globally.
pub fn set_cross_chain_verification_enabled(env: &Env, enabled: bool) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .set(&DataKey::CrossChainVerificationEnabled, &enabled);

    emit_admin_action(env, symbol_short!("xc_vrfy"), admin, Bytes::new(env));
}

/// Check if cross-chain verification is enabled.
pub fn is_cross_chain_verification_enabled(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get::<_, bool>(&DataKey::CrossChainVerificationEnabled)
        .unwrap_or(false)
}

/// Set the maximum allowed deviation (in basis points) between this chain and a reference chain.
pub fn set_cross_chain_deviation_threshold(env: &Env, threshold_bps: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    // 10000 bps = 100%, so max reasonable threshold is < 10000
    if threshold_bps >= 10000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    env.storage()
        .persistent()
        .set(&DataKey::CrossChainDeviationThreshold, &threshold_bps);

    emit_admin_action(env, symbol_short!("xc_dev"), admin, Bytes::new(env));
}

/// Get the cross-chain deviation threshold in basis points.
pub fn get_cross_chain_deviation_threshold(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get::<_, u32>(&DataKey::CrossChainDeviationThreshold)
        .unwrap_or(DEFAULT_CROSS_CHAIN_DEVIATION_THRESHOLD)
}

/// Store a cross-chain price observation for later verification.
/// Called externally with prices fetched from other chains.
pub fn submit_cross_chain_price(
    env: &Env,
    asset: Address,
    oracle_chain: Address,
    price: i128,
    decimals: u32,
    chain_id: String,
    timestamp: u64,
) {
    let admin = get_admin(env);
    admin.require_auth();

    record_reference_price(env, asset, oracle_chain, price, decimals, chain_id, timestamp);
}

/// Records a cross-chain reference price observation without an authorization
/// check, so it can be shared by [`submit_cross_chain_price`] (admin-submitted)
/// and the bridge integrations in [`crate::bridge_common`] (Axelar GMP,
/// LayerZero), which authenticate the observation through their own trust
/// model before calling this.
pub(crate) fn record_reference_price(
    env: &Env,
    asset: Address,
    oracle_chain: Address,
    price: i128,
    decimals: u32,
    chain_id: String,
    timestamp: u64,
) {
    crate::storage::check_registered_asset(env, &asset);

    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let entry = CrossChainPriceEntry {
        price,
        decimals,
        chain_id,
        ledger: env.ledger().sequence(),
        timestamp,
    };

    env.storage().persistent().set(
        &DataKey::CrossChainPrice(asset.clone(), oracle_chain.clone()),
        &entry,
    );

    env.storage().persistent().extend_ttl(
        &DataKey::CrossChainPrice(asset, oracle_chain),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

/// Get the most recent cross-chain price for an asset from a specific oracle.
pub fn get_cross_chain_price(
    env: &Env,
    asset: &Address,
    oracle_chain: &Address,
) -> Option<CrossChainPriceEntry> {
    env.storage()
        .persistent()
        .get::<_, CrossChainPriceEntry>(&DataKey::CrossChainPrice(
            asset.clone(),
            oracle_chain.clone(),
        ))
}

/// Verify cross-chain price consistency.
/// Returns `true` if deviation is within acceptable threshold, `false` if it exceeds.
pub fn verify_cross_chain_price(
    env: &Env,
    our_price: i128,
    our_decimals: u32,
    cross_chain_entry: &CrossChainPriceEntry,
) -> bool {
    if !is_cross_chain_verification_enabled(env) {
        return true; // Verification disabled, so it passes
    }

    if cross_chain_entry.price <= 0 || our_price <= 0 {
        return false; // Invalid prices fail verification
    }

    let threshold = get_cross_chain_deviation_threshold(env);

    // Normalize both prices to the same scale for comparison
    // deviation_bps = |our_price - ref_price| / max(our_price, ref_price) * 10000
    let our_scaled = our_price as u128;
    let ref_scaled = cross_chain_entry.price as u128;

    // Adjust for decimal differences if needed
    let (our_adjusted, ref_adjusted) = if our_decimals > cross_chain_entry.decimals {
        let factor = 10u128.pow(our_decimals - cross_chain_entry.decimals);
        (our_scaled, ref_scaled * factor)
    } else if cross_chain_entry.decimals > our_decimals {
        let factor = 10u128.pow(cross_chain_entry.decimals - our_decimals);
        (our_scaled * factor, ref_scaled)
    } else {
        (our_scaled, ref_scaled)
    };

    let max_price = our_adjusted.max(ref_adjusted);
    let deviation_bps = if max_price > 0 {
        let abs_diff = if our_adjusted > ref_adjusted {
            our_adjusted - ref_adjusted
        } else {
            ref_adjusted - our_adjusted
        };
        ((abs_diff as u128 * 10000) / max_price) as u32
    } else {
        0
    };

    deviation_bps <= threshold
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Env, Address, String as SorobanString};

    #[test]
    fn test_cross_chain_verification_flag() {
        let env = Env::default();
        let admin = Address::generate(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            SorobanString::from_str(&env, "Oracle"),
        );

        // Initially disabled
        assert!(!is_cross_chain_verification_enabled(&env));

        // Enable it
        set_cross_chain_verification_enabled(&env, true);
        assert!(is_cross_chain_verification_enabled(&env));

        // Disable it
        set_cross_chain_verification_enabled(&env, false);
        assert!(!is_cross_chain_verification_enabled(&env));
    }

    #[test]
    fn test_price_within_threshold() {
        let env = Env::default();
        let admin = Address::generate(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            SorobanString::from_str(&env, "Oracle"),
        );

        set_cross_chain_verification_enabled(&env, true);

        // 2% deviation threshold
        set_cross_chain_deviation_threshold(&env, 200);

        let our_price = 100_000_000_000_000_000i128; // 100 * 10^16
        let cross_chain = CrossChainPriceEntry {
            price: 101_500_000_000_000_000i128, // 101.5 * 10^16 (1.5% deviation)
            decimals: 18,
            chain_id: SorobanString::from_str(&env, "ethereum"),
            ledger: 1,
            timestamp: 1000,
        };

        assert!(verify_cross_chain_price(&env, our_price, 18, &cross_chain));
    }

    #[test]
    fn test_price_exceeds_threshold() {
        let env = Env::default();
        let admin = Address::generate(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            SorobanString::from_str(&env, "Oracle"),
        );

        set_cross_chain_verification_enabled(&env, true);

        // 2% deviation threshold
        set_cross_chain_deviation_threshold(&env, 200);

        let our_price = 100_000_000_000_000_000i128; // 100 * 10^16
        let cross_chain = CrossChainPriceEntry {
            price: 106_000_000_000_000_000i128, // 106 * 10^16 (6% deviation)
            decimals: 18,
            chain_id: SorobanString::from_str(&env, "ethereum"),
            ledger: 1,
            timestamp: 1000,
        };

        assert!(!verify_cross_chain_price(&env, our_price, 18, &cross_chain));
    }

    #[test]
    fn test_verification_disabled() {
        let env = Env::default();
        let admin = Address::generate(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            SorobanString::from_str(&env, "Oracle"),
        );

        // Verification disabled by default
        set_cross_chain_verification_enabled(&env, false);
        set_cross_chain_deviation_threshold(&env, 200);

        let our_price = 100_000_000_000_000_000i128;
        let cross_chain = CrossChainPriceEntry {
            price: 150_000_000_000_000_000i128, // 50% deviation - should pass because verification disabled
            decimals: 18,
            chain_id: SorobanString::from_str(&env, "ethereum"),
            ledger: 1,
            timestamp: 1000,
        };

        assert!(verify_cross_chain_price(&env, our_price, 18, &cross_chain));
    }
}
