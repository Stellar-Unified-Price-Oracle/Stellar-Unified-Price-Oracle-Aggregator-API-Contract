//! # SEP-40 Price Verification Helpers (Issue #287)
//!
//! Provides reusable helper functions that consumer contracts can call to safely
//! verify SEP-40 oracle price data before acting on it.
//!
//! ## Functions
//! - [`verify_price_freshness`] — check that a price is not older than `max_age` seconds.
//! - [`verify_price_deviation`] — check that two prices differ by no more than
//!   `max_deviation_bps` basis points.
//! - [`verify_cross_oracle`] — compare the aggregate price of this oracle against a
//!   reference price from another oracle and verify the deviation is within tolerance.

use soroban_sdk::{Address, Env};

use crate::admin::get_decimals;
use crate::events::{
    emit_price_freshness_verified, CrossOracleDeviationEvent, PriceDeviationVerifiedEvent,
};
use crate::prices::get_price;
use crate::types::AggregatePrice;

// ─── Freshness ─────────────────────────────────────────────────────────────

/// Verifies that an aggregate price is fresh enough relative to the current ledger time.
///
/// Emits a [`PriceFreshnessVerifiedEvent`] with the result.
///
/// # Arguments
///
/// * `env` — The Soroban execution environment.
/// * `asset` — Asset contract address to verify.
/// * `max_age` — Maximum acceptable age of the price in seconds. A value of `0`
///   means "no freshness check" and always returns `true`.
///
/// # Returns
///
/// `true` if the price is fresh (or `max_age == 0`); `false` if it is stale or
/// no aggregate exists.
pub fn verify_price_freshness(env: &Env, asset: Address, max_age: u64) -> bool {
    if max_age == 0 {
        emit_price_freshness_verified(env, asset.clone(), true, 0, 0);
        return true;
    }

    let agg: Option<AggregatePrice> = get_price(env, asset.clone(), 0);
    let (is_fresh, price_age) = match agg {
        None => (false, u64::MAX),
        Some(a) => {
            let now = env.ledger().timestamp();
            let age = now.saturating_sub(a.timestamp);
            (age <= max_age, age)
        }
    };

    emit_price_freshness_verified(
        env,
        asset.clone(),
        is_fresh,
        max_age,
        if price_age == u64::MAX { 0 } else { price_age },
    );

    is_fresh
}

// ─── Deviation ─────────────────────────────────────────────────────────────

/// Verifies that two prices are within `max_deviation_bps` basis points of each other.
///
/// Basis points: 100 bp = 1 %. The deviation is computed as:
/// `|price_a - price_b| * 10_000 / reference_price`
/// where `reference_price` is the larger of the two values.
///
/// Emits a [`PriceDeviationVerifiedEvent`] with the result.
///
/// # Arguments
///
/// * `env` — The Soroban execution environment.
/// * `price_a` — First price value (raw, scaled by `10^decimals`).
/// * `price_b` — Second price value (raw, scaled by `10^decimals`).
/// * `max_deviation_bps` — Maximum allowed deviation in basis points (e.g. `500` = 5 %).
///
/// # Returns
///
/// `true` if the deviation is within `max_deviation_bps`; `false` otherwise.
pub fn verify_price_deviation(
    env: &Env,
    price_a: i128,
    price_b: i128,
    max_deviation_bps: u32,
) -> bool {
    let reference = if price_a >= price_b { price_a } else { price_b };

    let deviation_bps = if reference == 0 {
        0u32
    } else {
        let diff = if price_a >= price_b {
            price_a - price_b
        } else {
            price_b - price_a
        };
        // Safe: diff <= reference, so diff * 10_000 <= reference * 10_000 <= i128::MAX
        let bps = (diff * 10_000) / reference;
        if bps > u32::MAX as i128 {
            u32::MAX
        } else {
            bps as u32
        }
    };

    let within_tolerance = deviation_bps <= max_deviation_bps;

    PriceDeviationVerifiedEvent {
        price_a,
        price_b,
        deviation_bps,
        max_deviation_bps,
        within_tolerance,
    }
    .publish(env);

    within_tolerance
}

// ─── Cross-oracle comparison ────────────────────────────────────────────────

/// Compares the current aggregate price of this oracle for `asset` against an
/// externally supplied `reference_price` and verifies the deviation is within
/// `max_deviation_bps` basis points.
///
/// Typical use: the caller has already read a price from a second (reference)
/// oracle off-chain or via a cross-contract call and passes it here for comparison.
///
/// Emits a [`CrossOracleDeviationEvent`] with the result.
///
/// # Arguments
///
/// * `env` — The Soroban execution environment.
/// * `asset` — Asset contract address whose price is fetched from this oracle.
/// * `reference_price` — Raw price from the reference oracle (must use the same
///   decimal precision).
/// * `max_deviation_bps` — Maximum allowed deviation in basis points.
///
/// # Returns
///
/// `true` if the two prices are within tolerance or no aggregate exists (treated
/// as "cannot verify", returns `false`).
pub fn verify_cross_oracle(
    env: &Env,
    asset: Address,
    reference_price: i128,
    max_deviation_bps: u32,
) -> bool {
    let agg = match get_price(env, asset.clone(), 0) {
        None => {
            CrossOracleDeviationEvent {
                asset: asset.clone(),
                oracle_price: 0,
                reference_price,
                deviation_bps: u32::MAX,
                max_deviation_bps,
                within_tolerance: false,
            }
            .publish(env);
            return false;
        }
        Some(a) => a,
    };

    let oracle_price = agg.price;
    let reference = if oracle_price >= reference_price {
        oracle_price
    } else {
        reference_price
    };
    let deviation_bps = if reference == 0 {
        0u32
    } else {
        let diff = if oracle_price >= reference_price {
            oracle_price - reference_price
        } else {
            reference_price - oracle_price
        };
        let bps = (diff * 10_000) / reference;
        if bps > u32::MAX as i128 {
            u32::MAX
        } else {
            bps as u32
        }
    };

    let within_tolerance = deviation_bps <= max_deviation_bps;

    CrossOracleDeviationEvent {
        asset: asset.clone(),
        oracle_price,
        reference_price,
        deviation_bps,
        max_deviation_bps,
        within_tolerance,
    }
    .publish(env);

    within_tolerance
}

/// Returns the decimal precision configured in this oracle — useful for callers
/// that need to normalise prices before calling `verify_price_deviation`.
pub fn get_oracle_decimals(env: &Env) -> u32 {
    get_decimals(env)
}
