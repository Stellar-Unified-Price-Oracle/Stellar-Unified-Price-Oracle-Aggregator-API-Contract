//! # Market Impact / Slippage Analytics
//!
//! Estimates the market impact (slippage) of trading a given size of one asset for
//! another, using the depth of the DEX/AMM pools already ingested by [`crate::amm`]
//! (Soroswap-style constant-product pools, registered via `register_soroswap_pool`).
//!
//! Consumers such as lending protocols and DEX aggregators combine the oracle's
//! spot price with this liquidity-aware impact estimate to decide whether a given
//! trade size is safe to execute against on-chain liquidity, without having to
//! replicate constant-product math themselves.
//!
//! ## Model
//!
//! For a pool with reserves `(reserve_in, reserve_out)` and fee `fee_bps`:
//! ```text
//! amount_in_after_fee = amount_in * (10_000 - fee_bps) / 10_000
//! amount_out          = reserve_out - k / (reserve_in + amount_in_after_fee)
//! spot_price          = reserve_out * 1e18 / reserve_in
//! execution_price     = amount_out * 1e18 / amount_in
//! price_impact_bps    = |execution_price - spot_price| * 10_000 / spot_price
//! ```
//! This mirrors the pricing math in [`crate::amm::swap`] but is read-only: no
//! reserves are mutated and no token transfer occurs.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::amm::get_soroswap_pool;
use crate::types::{ErrorCode, ImpactCurvePoint, MarketImpactEstimate, SoroswapPool};

/// Fixed-point scale used for spot/execution price values (1e18).
const PRICE_SCALE: u128 = 1_000_000_000_000_000_000;

/// Default trade sizes (as basis points of pool reserve) used to build an impact
/// curve when the caller does not supply explicit sizes: 1%, 5%, 10%, 25%, 50%.
pub const DEFAULT_CURVE_STEPS_BPS: [u32; 5] = [100, 500, 1_000, 2_500, 5_000];

fn safe_mul_u128(env: &Env, a: u128, b: u128) -> u128 {
    a.checked_mul(b)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ArithmeticOverflow))
}

fn safe_div_u128(env: &Env, a: u128, b: u128) -> u128 {
    if b == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    a / b
}

fn deviation_bps(env: &Env, a: u128, b: u128) -> u32 {
    if b == 0 {
        return 0;
    }
    let diff = if a > b { a - b } else { b - a };
    let bps = safe_div_u128(env, safe_mul_u128(env, diff, 10_000), b);
    bps.min(u32::MAX as u128) as u32
}

/// Looks up the Soroswap pool for `(asset_in, asset_out)` in either registration
/// order. Returns `(pool, reserve_in, reserve_out, direct)` where `direct` is
/// `true` when the pool was registered as `(asset_in, asset_out)`.
fn find_pool_reserves(
    env: &Env,
    asset_in: &Address,
    asset_out: &Address,
) -> Option<(SoroswapPool, i128, i128)> {
    if let Some(p) = get_soroswap_pool(env, asset_in.clone(), asset_out.clone()) {
        if p.enabled && p.reserve_a > 0 && p.reserve_b > 0 {
            let reserve_in = p.reserve_a;
            let reserve_out = p.reserve_b;
            return Some((p, reserve_in, reserve_out));
        }
    }
    if let Some(p) = get_soroswap_pool(env, asset_out.clone(), asset_in.clone()) {
        if p.enabled && p.reserve_a > 0 && p.reserve_b > 0 {
            let reserve_in = p.reserve_b;
            let reserve_out = p.reserve_a;
            return Some((p, reserve_in, reserve_out));
        }
    }
    None
}

/// Computes `(amount_out, spot_price, execution_price, price_impact_bps)` for a
/// trade of `amount_in` against reserves `(reserve_in, reserve_out)` at `fee_bps`.
fn compute_impact(
    env: &Env,
    amount_in: i128,
    reserve_in: i128,
    reserve_out: i128,
    fee_bps: u32,
) -> (i128, i128, i128, u32) {
    if amount_in <= 0 || amount_in >= reserve_in {
        panic_with_error!(env, ErrorCode::InvalidTradeSize);
    }

    let r_in = reserve_in as u128;
    let r_out = reserve_out as u128;
    let k = safe_mul_u128(env, r_in, r_out);

    let amount_in_after_fee = safe_div_u128(
        env,
        safe_mul_u128(env, amount_in as u128, 10_000 - fee_bps as u128),
        10_000,
    );
    let new_reserve_in = r_in.saturating_add(amount_in_after_fee);
    let new_reserve_out = safe_div_u128(env, k, new_reserve_in);
    if new_reserve_out >= r_out {
        panic_with_error!(env, ErrorCode::InvalidTradeSize);
    }
    let amount_out_u128 = r_out - new_reserve_out;

    let spot_price = safe_div_u128(env, safe_mul_u128(env, r_out, PRICE_SCALE), r_in);
    let execution_price = safe_div_u128(
        env,
        safe_mul_u128(env, amount_out_u128, PRICE_SCALE),
        amount_in as u128,
    );
    let impact_bps = deviation_bps(env, execution_price, spot_price);

    (
        amount_out_u128 as i128,
        spot_price as i128,
        execution_price as i128,
        impact_bps,
    )
}

/// Estimates the market impact of trading `amount_in` of `asset_in` for
/// `asset_out`, using the depth of the registered Soroswap pool for that pair.
///
/// # Panics
/// * [`ErrorCode::PoolNotFound`] — no enabled pool is registered for the pair.
/// * [`ErrorCode::InvalidTradeSize`] — `amount_in <= 0` or `amount_in >= reserve_in`
///   (a trade that would fully drain the pool).
pub fn estimate_market_impact(
    env: &Env,
    asset_in: Address,
    asset_out: Address,
    amount_in: i128,
) -> MarketImpactEstimate {
    let (pool, reserve_in, reserve_out) = find_pool_reserves(env, &asset_in, &asset_out)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::PoolNotFound));

    let (amount_out, spot_price, execution_price, price_impact_bps) =
        compute_impact(env, amount_in, reserve_in, reserve_out, pool.fee_bps);

    MarketImpactEstimate {
        asset_in,
        asset_out,
        amount_in,
        amount_out,
        spot_price,
        execution_price,
        price_impact_bps,
    }
}

/// Computes a market-impact curve: price impact at each of `sizes` (units of
/// `asset_in`). When `sizes` is empty, defaults to
/// [`DEFAULT_CURVE_STEPS_BPS`] percentages of the pool's `asset_in` reserve.
///
/// Sizes that are non-positive or would drain the pool are skipped rather than
/// causing the whole call to panic, so callers get a best-effort curve.
///
/// # Panics
/// * [`ErrorCode::PoolNotFound`] — no enabled pool is registered for the pair.
pub fn get_market_impact_curve(
    env: &Env,
    asset_in: Address,
    asset_out: Address,
    sizes: Vec<i128>,
) -> Vec<ImpactCurvePoint> {
    let (pool, reserve_in, reserve_out) = find_pool_reserves(env, &asset_in, &asset_out)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::PoolNotFound));

    let mut points: Vec<ImpactCurvePoint> = Vec::new(env);

    let push_point = |amount_in: i128, points: &mut Vec<ImpactCurvePoint>| {
        if amount_in <= 0 || amount_in >= reserve_in {
            return;
        }
        let (amount_out, _, _, price_impact_bps) =
            compute_impact(env, amount_in, reserve_in, reserve_out, pool.fee_bps);
        points.push_back(ImpactCurvePoint {
            amount_in,
            amount_out,
            price_impact_bps,
        });
    };

    if sizes.is_empty() {
        for step_bps in DEFAULT_CURVE_STEPS_BPS {
            let amount_in = (reserve_in as u128)
                .saturating_mul(step_bps as u128)
                .saturating_div(10_000) as i128;
            push_point(amount_in, &mut points);
        }
    } else {
        for i in 0..sizes.len() {
            push_point(sizes.get_unchecked(i), &mut points);
        }
    }

    points
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::String as SorobanString;

    fn init_with_pool(env: &Env, reserve_a: i128, reserve_b: i128) -> (Address, Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        crate::admin::initialize(
            env,
            admin.clone(),
            1,
            100,
            18,
            SorobanString::from_str(env, "Oracle"),
        );
        let asset_a = Address::generate(env);
        let asset_b = Address::generate(env);
        crate::amm::register_soroswap_pool(
            env,
            asset_a.clone(),
            asset_b.clone(),
            reserve_a,
            reserve_b,
            30,
        );
        (admin, asset_a, asset_b)
    }

    #[test]
    fn test_estimate_market_impact_direct_pool() {
        let env = Env::default();
        let (_, asset_a, asset_b) = init_with_pool(&env, 1_000_000, 2_000_000);

        let estimate = estimate_market_impact(&env, asset_a.clone(), asset_b.clone(), 10_000);
        assert_eq!(estimate.asset_in, asset_a);
        assert_eq!(estimate.asset_out, asset_b);
        assert!(estimate.amount_out > 0);
        // A 1% trade against a constant-product pool should have a small but
        // non-zero impact.
        assert!(estimate.price_impact_bps > 0);
        assert!(estimate.price_impact_bps < 500);
    }

    #[test]
    fn test_estimate_market_impact_reversed_pool() {
        let env = Env::default();
        let (_, asset_a, asset_b) = init_with_pool(&env, 1_000_000, 2_000_000);

        // Query in the reverse direction from how the pool was registered.
        let estimate = estimate_market_impact(&env, asset_b.clone(), asset_a.clone(), 20_000);
        assert_eq!(estimate.asset_in, asset_b);
        assert!(estimate.amount_out > 0 && estimate.amount_out < 10_000);
    }

    #[test]
    fn test_larger_trade_has_more_impact() {
        let env = Env::default();
        let (_, asset_a, asset_b) = init_with_pool(&env, 1_000_000, 1_000_000);

        let small = estimate_market_impact(&env, asset_a.clone(), asset_b.clone(), 1_000);
        let large = estimate_market_impact(&env, asset_a.clone(), asset_b.clone(), 100_000);
        assert!(large.price_impact_bps > small.price_impact_bps);
    }

    #[test]
    fn test_impact_curve_default_steps() {
        let env = Env::default();
        let (_, asset_a, asset_b) = init_with_pool(&env, 1_000_000, 1_000_000);

        let curve = get_market_impact_curve(&env, asset_a, asset_b, Vec::new(&env));
        assert_eq!(curve.len(), DEFAULT_CURVE_STEPS_BPS.len() as u32);
        // Impact should be monotonically non-decreasing as trade size grows.
        let mut last_impact = 0u32;
        for i in 0..curve.len() {
            let point = curve.get_unchecked(i);
            assert!(point.price_impact_bps >= last_impact);
            last_impact = point.price_impact_bps;
        }
    }

    #[test]
    #[should_panic]
    fn test_no_pool_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        crate::admin::initialize(
            &env,
            admin,
            1,
            100,
            18,
            SorobanString::from_str(&env, "Oracle"),
        );
        let asset_a = Address::generate(&env);
        let asset_b = Address::generate(&env);
        estimate_market_impact(&env, asset_a, asset_b, 1_000);
    }
}
