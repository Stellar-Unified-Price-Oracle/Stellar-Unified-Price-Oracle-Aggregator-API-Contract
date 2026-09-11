//! # Exotic Asset Fair-Value Pricing Engine (#177)
//!
//! Provides fair-value calculations for complex asset types:
//! - **LP Tokens**: `2 * sqrt(reserve0 * reserve1) / total_supply`
//! - **Index/Basket**: `sum(price_i * weight_i) / sum(weight_i)`
//! - **Options (Black-Scholes)**: integer approximation via Abramowitz & Stegun
//!   rational polynomial for the normal CDF, all in `#![no_std]` pure Rust.
//!
//! A cycle-detection guard (max depth = 3, visited-asset tracking) prevents
//! infinite recursion when asset components reference each other.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AssetPricingConfig, AssetType, DataKey, ErrorCode};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Maximum recursion depth for component price resolution.
const MAX_RESOLUTION_DEPTH: u32 = 3;

/// Fixed-point scale factor: 10^18.
const SCALE: i128 = 1_000_000_000_000_000_000i128;

// ─────────────────────────────────────────────────────────────────────────────
// Storage helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_asset_config(env: &Env, asset: &Address) -> Option<AssetPricingConfig> {
    let key = DataKey::ExoticAssetConfig(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

fn write_asset_config(env: &Env, asset: &Address, config: &AssetPricingConfig) {
    let key = DataKey::ExoticAssetConfig(asset.clone());
    env.storage().persistent().set(&key, config);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

// ─────────────────────────────────────────────────────────────────────────────
// Admin configuration
// ─────────────────────────────────────────────────────────────────────────────

/// Registers the pricing configuration for an exotic asset (admin only).
pub fn set_exotic_asset_config(env: &Env, asset: Address, config: AssetPricingConfig) {
    let admin = get_admin(env);
    admin.require_auth();
    write_asset_config(env, &asset, &config);
    crate::events::ExoticAssetConfigSetEvent {
        asset: asset.clone(),
        admin: admin.clone(),
    }
    .publish(env);
}

/// Returns the pricing configuration for an exotic asset, or `None` if not configured.
pub fn get_exotic_asset_config(env: &Env, asset: Address) -> Option<AssetPricingConfig> {
    read_asset_config(env, &asset)
}

// ─────────────────────────────────────────────────────────────────────────────
// Fair-value computation
// ─────────────────────────────────────────────────────────────────────────────

/// Computes the fair value of an exotic asset.
///
/// Returns the price scaled by `10^18` (SCALE), or panics with `NoData` if
/// a required component price is unavailable.
///
/// Cycle detection is handled by a `visited` list passed through recursive calls.
pub fn get_exotic_price(env: &Env, asset: &Address) -> i128 {
    let mut visited: Vec<Address> = Vec::new(env);
    resolve_price(env, asset, 0, &mut visited)
}

fn resolve_price(env: &Env, asset: &Address, depth: u32, visited: &mut Vec<Address>) -> i128 {
    if depth > MAX_RESOLUTION_DEPTH {
        panic_with_error!(env, ErrorCode::ExoticCycleLimitExceeded);
    }

    // Cycle detection
    if visited.contains(asset) {
        panic_with_error!(env, ErrorCode::ExoticCycleDetected);
    }
    visited.push_back(asset.clone());

    let config = read_asset_config(env, asset)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ExoticAssetNotConfigured));

    let price = match config.asset_type {
        AssetType::Direct => resolve_direct_price(env, asset),
        AssetType::LPToken(reserve0, reserve1, total_supply) => {
            compute_lp_token_price(env, reserve0, reserve1, total_supply, depth, visited)
        }
        AssetType::Index(components, weights) => {
            compute_index_price(env, &components, &weights, depth, visited)
        }
        AssetType::Option(underlying, strike, expiry, is_call) => {
            compute_option_price(env, &underlying, strike, expiry, is_call, depth, visited)
        }
    };

    // Remove from visited on the way back up
    let mut new_visited: Vec<Address> = Vec::new(env);
    for i in 0..visited.len() {
        let v = visited.get_unchecked(i);
        if v != *asset {
            new_visited.push_back(v);
        }
    }
    *visited = new_visited;

    price
}

/// Resolves the price for a `Direct` asset from the standard aggregate price store.
fn resolve_direct_price(env: &Env, asset: &Address) -> i128 {
    let key = DataKey::Aggregate(asset.clone());
    let agg: crate::types::AggregatePrice = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));
    agg.price
}

// ─────────────────────────────────────────────────────────────────────────────
// LP Token fair value: 2 * sqrt(r0 * r1) / total_supply
// ─────────────────────────────────────────────────────────────────────────────

fn compute_lp_token_price(
    env: &Env,
    reserve0: u128,
    reserve1: u128,
    total_supply: u128,
    _depth: u32,
    _visited: &mut Vec<Address>,
) -> i128 {
    if total_supply == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    if reserve0 == 0 || reserve1 == 0 {
        panic_with_error!(env, ErrorCode::NoData);
    }

    // Compute sqrt(reserve0 * reserve1) using u128 integer square root
    // Both prices are SCALE-denominated; multiply then divide by SCALE to keep scale
    let product_u128 = reserve0.saturating_mul(reserve1) / (SCALE as u128);
    let sqrt_product = isqrt_u128(product_u128);

    // 2 * sqrt / total_supply * SCALE (re-scale result)
    let numerator = 2u128
        .saturating_mul(sqrt_product)
        .saturating_mul(SCALE as u128);
    let result = numerator / total_supply;
    result as i128
}

/// Integer square root via Newton's method (no_std compatible).
fn isqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

// ─────────────────────────────────────────────────────────────────────────────
// Index / basket: sum(price_i * weight_i) / sum(weight_i)
// ─────────────────────────────────────────────────────────────────────────────

fn compute_index_price(
    env: &Env,
    components: &Vec<Address>,
    weights: &Vec<u32>,
    depth: u32,
    visited: &mut Vec<Address>,
) -> i128 {
    if components.len() == 0 || components.len() != weights.len() {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let mut weighted_sum: i128 = 0;
    let mut weight_total: u64 = 0;

    for i in 0..components.len() {
        let component = components.get_unchecked(i);
        let weight = weights.get_unchecked(i) as u64;
        let price = resolve_price(env, &component, depth + 1, visited);
        if price <= 0 {
            panic_with_error!(env, ErrorCode::NoData);
        }
        weighted_sum = weighted_sum.saturating_add(price.saturating_mul(weight as i128));
        weight_total = weight_total.saturating_add(weight);
    }

    if weight_total == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    weighted_sum / (weight_total as i128)
}

// ─────────────────────────────────────────────────────────────────────────────
// Black-Scholes option pricing (integer fixed-point)
//
// Uses the closed-form Black-Scholes formula:
//   Call = S*N(d1) - K*e^(-rT)*N(d2)
//   Put  = K*e^(-rT)*N(-d2) - S*N(-d1)
//
// All values in SCALE (10^18) fixed point.
// Risk-free rate r = 0 (Soroban has no oracle for rates; safe default).
// Volatility is stored in the AssetPricingConfig.
// ─────────────────────────────────────────────────────────────────────────────

fn compute_option_price(
    env: &Env,
    underlying: &Address,
    strike: u128, // scaled by SCALE
    expiry: u64,  // Unix timestamp of expiry
    is_call: bool,
    depth: u32,
    visited: &mut Vec<Address>,
) -> i128 {
    let s = resolve_price(env, underlying, depth + 1, visited);
    if s <= 0 {
        panic_with_error!(env, ErrorCode::NoData);
    }

    let current_ts = env.ledger().timestamp();
    if expiry <= current_ts {
        // Expired option: intrinsic value only
        let intrinsic = if is_call {
            (s - strike as i128).max(0)
        } else {
            (strike as i128 - s).max(0)
        };
        return intrinsic;
    }

    // Time to expiry in years, fixed-point (SCALE = 1 year unit in our repr)
    // T_seconds / 31_536_000 (seconds per year), scaled by SCALE
    let t_secs = (expiry - current_ts) as i128;
    let secs_per_year: i128 = 31_536_000;
    // T in SCALE units
    let t = t_secs.saturating_mul(SCALE) / secs_per_year;

    // Read volatility from config (stored as basis points of SCALE, e.g. 2000 = 20%)
    let config = read_asset_config(env, underlying)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::ExoticAssetNotConfigured));
    let sigma = config.volatility_bps as i128 * (SCALE / 10_000); // convert bps → SCALE

    if sigma == 0 || t == 0 {
        // Degenerate case
        let intrinsic = if is_call {
            (s - strike as i128).max(0)
        } else {
            (strike as i128 - s).max(0)
        };
        return intrinsic;
    }

    // d1 = (ln(S/K) + 0.5*sigma^2*T) / (sigma * sqrt(T))
    // d2 = d1 - sigma * sqrt(T)
    //
    // All in SCALE fixed-point arithmetic.

    let k = strike as i128;

    // ln(S/K): approximate via integer log.  Both S and K are SCALE-denominated.
    let ln_sk = fixed_ln_ratio(s, k); // returns value in SCALE

    // 0.5 * sigma^2 * T
    let sigma_sq = sigma.saturating_mul(sigma) / SCALE;
    let half_sigma_sq_t = sigma_sq.saturating_mul(t) / SCALE / 2;

    // sigma * sqrt(T)
    let sqrt_t = fixed_sqrt(t); // SCALE-denominated sqrt
    let sigma_sqrt_t = sigma.saturating_mul(sqrt_t) / SCALE;

    if sigma_sqrt_t == 0 {
        let intrinsic = if is_call {
            (s - k).max(0)
        } else {
            (k - s).max(0)
        };
        return intrinsic;
    }

    let d1 = (ln_sk + half_sigma_sq_t).saturating_mul(SCALE) / sigma_sqrt_t;
    let d2 = d1 - sigma_sqrt_t;

    // N(d): Abramowitz & Stegun approximation, integer version
    let n_d1 = normal_cdf(d1);
    let n_d2 = normal_cdf(d2);
    let n_neg_d1 = SCALE - n_d1;
    let n_neg_d2 = SCALE - n_d2;

    // With r=0: e^(-rT) = 1
    let price = if is_call {
        // C = S*N(d1) - K*N(d2)
        s.saturating_mul(n_d1) / SCALE - k.saturating_mul(n_d2) / SCALE
    } else {
        // P = K*N(-d2) - S*N(-d1)
        k.saturating_mul(n_neg_d2) / SCALE - s.saturating_mul(n_neg_d1) / SCALE
    };

    price.max(0)
}

// ─────────────────────────────────────────────────────────────────────────────
// Fixed-point math helpers (no_std)
// ─────────────────────────────────────────────────────────────────────────────

/// Integer square root of a SCALE-denominated fixed-point value.
/// Input and output are both SCALE-denominated.
/// sqrt_fp(x * SCALE) = sqrt(x) * SCALE
fn fixed_sqrt(x: i128) -> i128 {
    if x <= 0 {
        return 0;
    }
    // We want sqrt(x/SCALE) * SCALE = sqrt(x * SCALE)
    // So compute isqrt(x * SCALE) using u128
    let product = (x as u128).saturating_mul(SCALE as u128);
    isqrt_u128(product) as i128
}

/// Fixed-point natural log approximation: ln(a/b) where a, b are SCALE-denominated.
///
/// Uses the identity: ln(x) ≈ 2 * atanh((x-1)/(x+1)) for x near 1, and
/// range-reduction via ln(x) = n*ln(2) + ln(x/2^n) for larger x.
///
/// Returns SCALE-denominated result.
fn fixed_ln_ratio(a: i128, b: i128) -> i128 {
    if b == 0 || a <= 0 {
        return 0;
    }

    // Compute ratio r = a * SCALE / b (SCALE-denominated)
    let r = a.saturating_mul(SCALE) / b;

    fixed_ln(r)
}

/// Integer ln approximation for SCALE-denominated input.
/// Returns SCALE-denominated output.
/// Accurate to ~0.1% for typical price ratios.
fn fixed_ln(x: i128) -> i128 {
    if x <= 0 {
        return -SCALE * 40; // approximate -infinity cap
    }
    if x == SCALE {
        return 0;
    }

    // ln2 in SCALE: 0.693147... * SCALE
    let ln2: i128 = 693_147_180_559_945_309i128; // ln(2) * 10^18

    // Range reduce: find n such that x = m * 2^n where 0.5 <= m < 1 (SCALE units)
    let mut val = x;
    let mut n: i32 = 0;
    while val >= 2 * SCALE {
        val /= 2;
        n += 1;
    }
    while val < SCALE {
        val *= 2;
        n -= 1;
    }
    // Now val is in [SCALE, 2*SCALE)

    // Use Padé approximation for ln(1+u) where u = (val - SCALE) / SCALE
    // ln(1+u) ≈ u - u²/2 + u³/3 - u⁴/4 (Taylor, but we use the Padé for better convergence)
    // With val in [1, 2], u = val/SCALE - 1 ∈ [0, 1]
    // Better: use atanh form. Let t = (val - SCALE) / (val + SCALE) in [-1,1]
    // ln(val/SCALE) = 2 * atanh(t) where t = (val - SCALE)/(val + SCALE)

    let num = val - SCALE;
    let den = val + SCALE;
    // t = num / den (value in [-0.33, 0.33] for val in [SCALE, 2*SCALE])
    // t is SCALE-denominated when computed as:
    let t = num.saturating_mul(SCALE) / den; // SCALE-denominated fraction

    // atanh(t) ≈ t + t³/3 + t⁵/5 (converges well for |t| < 0.5)
    let t2 = t.saturating_mul(t) / SCALE;
    let t3 = t2.saturating_mul(t) / SCALE;
    let t5 = t3.saturating_mul(t2) / SCALE;
    let t7 = t5.saturating_mul(t2) / SCALE;

    let atanh_t = t + t3 / 3 + t5 / 5 + t7 / 7;
    let ln_val = 2 * atanh_t; // ln(val/SCALE)

    // Result: ln(x/SCALE) = n * ln(2) + ln(val/SCALE)
    let n_ln2 = ln2.saturating_mul(n as i128);
    n_ln2 + ln_val
}

/// Abramowitz & Stegun approximation for the standard normal CDF N(d).
///
/// Input `d` is SCALE-denominated (represents the z-score scaled by 10^18).
/// Returns SCALE-denominated probability in [0, SCALE].
///
/// Uses the rational approximation from A&S 26.2.17 with maximum error 7.5e-8.
///
/// Coefficients (× 10^7 for integer arithmetic):
///   p  = 0.2316419
///   b1 = 0.319381530
///   b2 = -0.356563782
///   b3 = 1.781477937
///   b4 = -1.821255978
///   b5 = 1.330274429
fn normal_cdf(d: i128) -> i128 {
    // A&S 26.2.17 coefficients × 10^9
    const P_RECIP_DENOM: i128 = 2_316_419; // p = 0.2316419 — coefficient denominator multiplier
                                           // b coefficients × 10^9
    const B1: i128 = 319_381_530;
    const B2: i128 = -356_563_782;
    const B3: i128 = 1_781_477_937;
    const B4: i128 = -1_821_255_978;
    const B5: i128 = 1_330_274_429;
    // Scaling factor for coefficients
    const BSCALE: i128 = 1_000_000_000i128; // 10^9

    let negative = d < 0;
    let d_abs = if negative { -d } else { d }; // SCALE-denominated |d|

    // t = 1 / (1 + p * |d|)
    // where p = 0.2316419 ≈ P_RECIP_DENOM / 10^7
    // To avoid floating point: t_denom = SCALE + p_scaled * |d| / SCALE
    // p_scaled = P_RECIP_DENOM (units: 10^7), d_abs in SCALE (10^18)
    // p * |d| in SCALE = P_RECIP_DENOM * d_abs / 10^7
    let p_d = P_RECIP_DENOM.saturating_mul(d_abs) / 10_000_000i128; // SCALE-denominated
    let t_denom = SCALE + p_d; // SCALE + SCALE = SCALE * (1 + p*|d|) in SCALE units
    if t_denom == 0 {
        return if negative { 0 } else { SCALE };
    }
    let t = SCALE.saturating_mul(SCALE) / t_denom; // SCALE-denominated t = 1/(1+p|d|)

    // Horner polynomial evaluation: ((((b5*t + b4)*t + b3)*t + b2)*t + b1)*t
    // Each b_i is × BSCALE (10^9), t is × SCALE (10^18)
    // Product bi*t → needs /BSCALE after multiply, then /SCALE per Horner step
    let t_b = |b: i128| -> i128 {
        // Returns SCALE-denominated coefficient * t
        b.saturating_mul(t) / BSCALE
    };

    let poly = {
        // Start with b5 * t (SCALE-denominated)
        let mut acc = t_b(B5);
        // acc*t + b4*t → but we need: (b5*t + b4)*t etc.
        // Actually Horner: acc = b5; acc = acc*t + b4; acc = acc*t + b3 ...
        // We track in SCALE units:
        let b5_t = B5.saturating_mul(t) / BSCALE / SCALE; // dimensionless intermediate
        let _ = (acc, b5_t);

        // Redo cleanly in SCALE arithmetic:
        // poly = t*(b1 + t*(b2 + t*(b3 + t*(b4 + t*b5))))
        // All b_i are in BSCALE units; t in SCALE units
        // Step from inside out:
        let step5 = B5; // × BSCALE
        let step4 = B4 + step5.saturating_mul(t) / SCALE; // × BSCALE
        let step3 = B3 + step4.saturating_mul(t) / SCALE;
        let step2 = B2 + step3.saturating_mul(t) / SCALE;
        let step1 = B1 + step2.saturating_mul(t) / SCALE;
        acc = step1.saturating_mul(t) / SCALE; // final poly, × BSCALE/SCALE
                                               // Convert to SCALE: multiply by SCALE / BSCALE
        acc = acc.saturating_mul(SCALE) / BSCALE;
        acc
    };

    // pdf(d) = exp(-d²/2) / sqrt(2π)
    // We use the rational approximation: poly * pdf(d)
    // But A&S 26.2.17 gives: N(x) ≈ 1 - n(x)*(b1*t + b2*t² + b3*t³ + b4*t⁴ + b5*t⁵)
    // where n(x) = (1/sqrt(2π)) * exp(-x²/2)
    // We approximate n(x) as the standard normal PDF.

    let n_x = standard_normal_pdf(d_abs);
    let tail = n_x.saturating_mul(poly) / SCALE; // SCALE-denominated tail probability

    // Clamp to [0, SCALE]
    let tail_clamped = tail.max(0).min(SCALE);
    let n_pos = (SCALE - tail_clamped).max(0).min(SCALE);

    if negative {
        SCALE - n_pos
    } else {
        n_pos
    }
}

/// Approximates the standard normal PDF: (1/sqrt(2π)) * exp(-d²/2).
/// Input and output are SCALE-denominated.
fn standard_normal_pdf(d_abs: i128) -> i128 {
    // 1/sqrt(2π) ≈ 0.3989422804 in SCALE
    const INV_SQRT_2PI: i128 = 398_942_280_401_432_678i128; // 0.39894... * SCALE

    // exp(-d²/2): approximate via exp(-x) ≈ e^(-x)
    let d_sq = d_abs.saturating_mul(d_abs) / SCALE; // d² in SCALE
    let neg_half_d_sq = d_sq / 2; // d²/2 in SCALE

    let exp_val = fixed_exp_neg(neg_half_d_sq);
    INV_SQRT_2PI.saturating_mul(exp_val) / SCALE
}

/// Approximates exp(-x) for x >= 0 (SCALE-denominated input/output).
/// Uses the series: e^(-x) = 1 - x + x²/2! - x³/3! + ...
/// Capped at 20 terms for sufficient accuracy within the Black-Scholes range.
fn fixed_exp_neg(x: i128) -> i128 {
    if x <= 0 {
        return SCALE;
    }
    // For large x, exp(-x) → 0
    // If x > 20 * SCALE (i.e., |d| > ~6.3σ), probability is essentially 0
    if x > 20 * SCALE {
        return 0;
    }

    // Taylor series: sum_{k=0}^{N} (-x)^k / k!
    let mut result: i128 = SCALE; // k=0 term
    let mut term: i128 = SCALE; // current term (SCALE-denominated)

    // We compute 16 terms — sufficient for convergence when x <= 20*SCALE
    for k in 1i128..=16i128 {
        term = term.saturating_mul(x) / SCALE; // multiply by x (SCALE)
        term = term / k; // divide by k (dimensionless)
        if k % 2 == 0 {
            result = result.saturating_add(term);
        } else {
            result = result.saturating_sub(term);
        }
    }

    result.max(0)
}
