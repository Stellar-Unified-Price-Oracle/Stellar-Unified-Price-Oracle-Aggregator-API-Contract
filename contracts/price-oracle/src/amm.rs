//! # Automated Market Maker (AMM) for Oracle Data Feeds (Issue #180)
//!
//! Implements a constant-product AMM (`x * y = k`) data-feed mechanism. This allows
//! push-based oracle pricing with bounded price-manipulation safeguards.
//!
//! ## Constant-Product Formula
//!
//! For a pool with reserves `(reserve_x, reserve_y)`:
//! ```text
//! k = reserve_x * reserve_y
//! dy = reserve_y - (k / (reserve_x + amount_in_after_fee))
//! ```
//!
//! ## Fee & Overflow Safety
//!
//! * 0.3 % swap fee: `amount_in_after_fee = amount_in * 997 / 1000`
//! * All intermediate products use `u128` arithmetic to handle reserves up to ~1.7 × 10³⁸.
//! * On-chain checked arithmetic (`checked_mul`, `checked_div`) panic on overflow.
//!
//! ## Price-Manipulation Guardrail
//!
//! After each swap the effective marginal price is compared against the oracle's
//! external aggregate median price. If the deviation exceeds the configured
//! `max_deviation_bps` the swap is reverted.

use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AmmPool, AmmWeightConfig, DataKey, ErrorCode, SoroswapPool};
use soroban_sdk::{panic_with_error, symbol_short, Address, Env, Symbol};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Default swap fee in basis points (30 bps = 0.3 %).
pub const DEFAULT_FEE_BPS: u32 = 30;

/// Default maximum deviation between AMM price and oracle median (500 bps = 5 %).
pub const DEFAULT_MAX_DEVIATION_BPS: u32 = 500;

// ─────────────────────────────────────────────────────────────────────────────
// Internal storage helpers
// ─────────────────────────────────────────────────────────────────────────────

fn pool_key(asset: &Symbol) -> DataKey {
    DataKey::AmmPool(asset.clone())
}

fn read_pool(env: &Env, asset: &Symbol) -> Option<AmmPool> {
    let key = pool_key(asset);
    let result = env.storage().persistent().get(&key);
    if result.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    result
}

fn write_pool(env: &Env, asset: &Symbol, pool: &AmmPool) {
    let key = pool_key(asset);
    env.storage().persistent().set(&key, pool);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_max_deviation_bps(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AmmMaxDeviationBps)
        .unwrap_or(DEFAULT_MAX_DEVIATION_BPS)
}

// ─────────────────────────────────────────────────────────────────────────────
// Safe u128 arithmetic helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Checked u128 multiplication — panics with `ErrorCode::ArithmeticOverflow` on overflow.
fn safe_mul_u128(env: &Env, a: u128, b: u128) -> u128 {
    match a.checked_mul(b) {
        Some(v) => v,
        None => panic_with_error!(env, ErrorCode::ArithmeticOverflow),
    }
}

/// Checked u128 division — panics with `ErrorCode::InvalidConfiguration` on divide-by-zero.
fn safe_div_u128(env: &Env, a: u128, b: u128) -> u128 {
    if b == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    a / b
}

// ─────────────────────────────────────────────────────────────────────────────
// Deviation check
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the deviation in basis points between `a` and `b` relative to `b`.
///
/// `deviation_bps = |a - b| * 10_000 / b`
fn compute_deviation_bps(a: u128, b: u128) -> u32 {
    if b == 0 {
        return u32::MAX;
    }
    let diff = if a > b { a - b } else { b - a };
    // Scale to BPS (multiply before divide to preserve precision)
    let bps = (diff.saturating_mul(10_000)) / b;
    if bps > u32::MAX as u128 {
        u32::MAX
    } else {
        bps as u32
    }
}

/// Fetches the oracle's aggregate median price for asset `asset_addr` from persistent storage.
/// Returns `0` if no aggregate price is available (non-fatal — deviation check will be skipped).
fn oracle_price_for_asset(env: &Env, asset_addr: &Address) -> u128 {
    let key = DataKey::Aggregate(asset_addr.clone());
    let agg: Option<crate::types::AggregatePrice> = env.storage().persistent().get(&key);
    match agg {
        Some(a) if a.price > 0 => a.price as u128,
        _ => 0,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Public API
// ─────────────────────────────────────────────────────────────────────────────

/// Initialises the AMM pool for `asset` with the provided token pair and initial reserves.
///
/// Admin-only. The pool is seeded with `initial_x` and `initial_y` units of the
/// respective tokens, and `k = initial_x * initial_y` is computed using safe u128
/// arithmetic. The pool starts enabled with `fee_bps = DEFAULT_FEE_BPS`.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`]        — caller is not the admin.
/// * [`ErrorCode::PoolAlreadyExists`]    — pool for this asset already exists.
/// * [`ErrorCode::InvalidConfiguration`] — either initial reserve is zero.
/// * [`ErrorCode::ArithmeticOverflow`]   — `k` computation overflows u128.
pub fn init_amm(
    env: &Env,
    asset: Symbol,
    asset_x: Address,
    asset_y: Address,
    initial_x: i128,
    initial_y: i128,
) {
    // Admin guard
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    if initial_x <= 0 || initial_y <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    if read_pool(env, &asset).is_some() {
        panic_with_error!(env, ErrorCode::PoolAlreadyExists);
    }

    let rx = initial_x as u128;
    let ry = initial_y as u128;
    let k = safe_mul_u128(env, rx, ry);

    let pool = AmmPool {
        asset_x,
        asset_y,
        reserve_x: initial_x,
        reserve_y: initial_y,
        k,
        enabled: true,
        fee_bps: DEFAULT_FEE_BPS,
    };
    write_pool(env, &asset, &pool);

    env.events().publish(
        (symbol_short!("amm_init"), asset),
        (initial_x, initial_y, k),
    );
}

/// Adds liquidity to an existing AMM pool.
///
/// Transfers tokens to the pool reserves and recomputes `k`.
///
/// # Panics
///
/// * [`ErrorCode::PoolNotFound`]  — pool for this asset does not exist.
/// * [`ErrorCode::InvalidPrice`]  — either amount is ≤ 0.
pub fn add_liquidity(env: &Env, caller: Address, asset: Symbol, amount_x: i128, amount_y: i128) {
    caller.require_auth();

    if amount_x <= 0 || amount_y <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let mut pool = match read_pool(env, &asset) {
        Some(p) => p,
        None => panic_with_error!(env, ErrorCode::PoolNotFound),
    };

    // Transfer tokens from caller to this contract
    let contract_address = env.current_contract_address();
    let token_x = soroban_sdk::token::Client::new(env, &pool.asset_x);
    let token_y = soroban_sdk::token::Client::new(env, &pool.asset_y);
    token_x.transfer(&caller, &contract_address, &amount_x);
    token_y.transfer(&caller, &contract_address, &amount_y);

    pool.reserve_x = pool.reserve_x.saturating_add(amount_x);
    pool.reserve_y = pool.reserve_y.saturating_add(amount_y);
    pool.k = safe_mul_u128(env, pool.reserve_x as u128, pool.reserve_y as u128);

    write_pool(env, &asset, &pool);

    env.events().publish(
        (symbol_short!("amm_liq"), asset),
        (amount_x, amount_y, pool.k),
    );
}

/// Performs a constant-product swap in the pool for `asset`.
///
/// The caller swaps `amount_in` units of `from_asset` for at least `min_return`
/// units of `to_asset`. A 0.3 % fee is applied before the constant-product
/// calculation. The post-swap marginal price is validated against the oracle
/// median — if it deviates by more than `max_deviation_bps` the swap reverts.
///
/// # Returns
///
/// The actual amount of `to_asset` received after the fee.
///
/// # Panics
///
/// * [`ErrorCode::PoolNotFound`]         — pool for `asset` does not exist or is disabled.
/// * [`ErrorCode::InvalidPrice`]         — `amount_in` ≤ 0.
/// * [`ErrorCode::SlippageExceeded`]     — output < `min_return`.
/// * [`ErrorCode::AmmPriceManipulation`] — post-swap price deviates beyond the threshold.
/// * [`ErrorCode::ArithmeticOverflow`]   — intermediate arithmetic overflows.
pub fn swap(
    env: &Env,
    caller: Address,
    asset: Symbol,
    from_asset: Address,
    to_asset: Address,
    amount_in: i128,
    min_return: i128,
) -> i128 {
    caller.require_auth();

    if amount_in <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let mut pool = match read_pool(env, &asset) {
        Some(p) if p.enabled => p,
        _ => panic_with_error!(env, ErrorCode::PoolNotFound),
    };

    // Determine direction: X → Y or Y → X
    let (reserve_in, reserve_out, x_to_y) =
        if from_asset == pool.asset_x && to_asset == pool.asset_y {
            (pool.reserve_x as u128, pool.reserve_y as u128, true)
        } else if from_asset == pool.asset_y && to_asset == pool.asset_x {
            (pool.reserve_y as u128, pool.reserve_x as u128, false)
        } else {
            panic_with_error!(env, ErrorCode::InvalidConfiguration)
        };

    // Apply fee: amount_in_after_fee = amount_in * (10000 - fee_bps) / 10000
    let fee_bps = pool.fee_bps as u128;
    let amount_in_u128 = amount_in as u128;
    let amount_in_after_fee = safe_div_u128(
        env,
        safe_mul_u128(env, amount_in_u128, 10_000 - fee_bps),
        10_000,
    );

    // Constant-product output: dy = reserve_out - k / (reserve_in + amount_in_after_fee)
    let new_reserve_in = reserve_in
        .checked_add(amount_in_after_fee)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ArithmeticOverflow));
    let new_reserve_out = safe_div_u128(env, pool.k, new_reserve_in);
    if new_reserve_out >= reserve_out {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }
    let amount_out_u128 = reserve_out - new_reserve_out;
    let amount_out = amount_out_u128 as i128;

    if amount_out < min_return {
        panic_with_error!(env, ErrorCode::SlippageExceeded);
    }

    // Post-swap marginal price: effective_price = new_reserve_out / new_reserve_in (x-units per y)
    // For the manipulation guardrail we compare the spot price of asset_x in terms of asset_y
    // against the oracle median for asset_x (denominated in the same scale).
    // spot_price = new_reserve_y / new_reserve_x
    let (new_rx, new_ry) = if x_to_y {
        (new_reserve_in, new_reserve_out)
    } else {
        (new_reserve_out, new_reserve_in)
    };

    // Only run manipulation check if oracle price is available
    let oracle_px = oracle_price_for_asset(env, &pool.asset_x);
    if oracle_px > 0 && new_rx > 0 {
        // AMM spot price: price_x_in_y_units = reserve_y / reserve_x (scaled by 1e18)
        let scale: u128 = 1_000_000_000_000_000_000; // 1e18
        let amm_spot = safe_div_u128(env, safe_mul_u128(env, new_ry, scale), new_rx);
        let max_dev = read_max_deviation_bps(env);
        let dev = compute_deviation_bps(amm_spot, oracle_px);
        if dev > max_dev {
            panic_with_error!(env, ErrorCode::AmmPriceManipulation);
        }
    }

    // Update reserves
    if x_to_y {
        pool.reserve_x = pool.reserve_x.saturating_add(amount_in);
        pool.reserve_y = (pool.reserve_y as u128).saturating_sub(amount_out_u128) as i128;
    } else {
        pool.reserve_y = pool.reserve_y.saturating_add(amount_in);
        pool.reserve_x = (pool.reserve_x as u128).saturating_sub(amount_out_u128) as i128;
    }
    // Recompute k to stay consistent with updated reserves
    pool.k = safe_mul_u128(env, pool.reserve_x as u128, pool.reserve_y as u128);

    write_pool(env, &asset, &pool);

    // Transfer out_asset from contract to caller
    let token_out = soroban_sdk::token::Client::new(env, &to_asset);
    let contract_address = env.current_contract_address();
    token_out.transfer(&contract_address, &caller, &amount_out);

    env.events().publish(
        (symbol_short!("amm_swap"), asset),
        (amount_in, amount_out, pool.reserve_x, pool.reserve_y),
    );

    amount_out
}

/// Enables or disables a pool. Admin-only.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::PoolNotFound`]  — pool does not exist.
pub fn set_amm_status(env: &Env, asset: Symbol, enabled: bool) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    let mut pool = match read_pool(env, &asset) {
        Some(p) => p,
        None => panic_with_error!(env, ErrorCode::PoolNotFound),
    };
    pool.enabled = enabled;
    write_pool(env, &asset, &pool);

    env.events()
        .publish((symbol_short!("amm_stat"), asset), (enabled,));
}

/// Returns the current state of a pool, or `None` if it does not exist.
pub fn get_amm_pool(env: &Env, asset: Symbol) -> Option<AmmPool> {
    read_pool(env, &asset)
}

/// Sets the maximum allowed AMM-to-oracle price deviation (in basis points). Admin-only.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`]        — caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] — `bps > 100_000`.
pub fn set_amm_max_deviation_bps(env: &Env, bps: u32) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    if bps > 100_000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::AmmMaxDeviationBps, &bps);
}

/// Returns the current AMM max-deviation setting (basis points). Default: 500.
pub fn get_amm_max_deviation_bps(env: &Env) -> u32 {
    read_max_deviation_bps(env)
}

// -----------------------------------------------------------------------------
// #281 — Soroswap Integration
// -----------------------------------------------------------------------------

/// Sets the AMM weight configuration for an asset. Admin-only.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — `weight_bps > 10_000`.
pub fn set_amm_weight(env: &Env, asset: Address, weight_bps: u32, enabled: bool) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    if weight_bps > 10_000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    env.storage().persistent().set(
        &DataKey::AmmWeight(asset.clone()),
        &AmmWeightConfig {
            asset,
            weight_bps,
            enabled,
        },
    );
}

/// Returns the AMM weight configuration for an asset, or `None` if not set.
pub fn get_amm_weight(env: &Env, asset: Address) -> Option<AmmWeightConfig> {
    env.storage().persistent().get(&DataKey::AmmWeight(asset))
}

/// Reads a Soroswap pool price for an asset pair.
///
/// Returns the spot price derived from pool reserves using the constant-product formula.
/// Returns `None` if the pool is not registered or disabled.
pub fn read_soroswap_price(env: &Env, asset_a: Address, asset_b: Address) -> Option<i128> {
    let key = DataKey::SoroswapPool(asset_a.clone(), asset_b.clone());
    let pool: Option<SoroswapPool> = env.storage().persistent().get(&key);
    match pool {
        Some(p) if p.enabled => {
            if p.reserve_a <= 0 || p.reserve_b <= 0 {
                return None;
            }
            let scale: u128 = 1_000_000_000_000_000_000; // 1e18
            let price = (p.reserve_b as u128)
                .saturating_mul(scale)
                .saturating_div(p.reserve_a as u128);
            Some(price as i128)
        }
        _ => None,
    }
}

/// Registers a Soroswap pool configuration. Admin-only.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — either reserve is ≤ 0.
pub fn register_soroswap_pool(
    env: &Env,
    asset_a: Address,
    asset_b: Address,
    reserve_a: i128,
    reserve_b: i128,
    fee_bps: u32,
) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    if reserve_a <= 0 || reserve_b <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    env.storage().persistent().set(
        &DataKey::SoroswapPool(asset_a.clone(), asset_b.clone()),
        &SoroswapPool {
            asset_a,
            asset_b,
            reserve_a,
            reserve_b,
            fee_bps,
            enabled: true,
        },
    );
}

/// Enables or disables a Soroswap pool. Admin-only.
pub fn set_soroswap_pool_status(env: &Env, asset_a: Address, asset_b: Address, enabled: bool) {
    let admin = crate::storage::get_admin(env);
    admin.require_auth();

    let key = DataKey::SoroswapPool(asset_a.clone(), asset_b.clone());
    let pool: Option<SoroswapPool> = env.storage().persistent().get(&key);
    if let Some(mut p) = pool {
        p.enabled = enabled;
        env.storage().persistent().set(&key, &p);
    }
}

/// Returns the Soroswap pool configuration, or `None` if not found.
pub fn get_soroswap_pool(env: &Env, asset_a: Address, asset_b: Address) -> Option<SoroswapPool> {
    env.storage()
        .persistent()
        .get(&DataKey::SoroswapPool(asset_a, asset_b))
}
