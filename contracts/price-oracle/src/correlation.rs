//! # Cross-Asset Price Correlation & Sanity Checks (#172)
//!
//! Allows the admin to register correlation ratio bands between pairs of assets.
//! When a price is submitted for an asset that participates in a correlation pair,
//! the ratio `price_base * RATIO_PRECISION / price_quote` is checked against the
//! configured `[min_ratio, max_ratio]` band. Out-of-band submissions are rejected
//! and a reputation penalty is applied to the submitting source.
//!
//! ## Precision
//! All ratio values are scaled by `RATIO_PRECISION = 10_000_000` (10^7) to represent
//! up to seven decimal places without floating-point math.
//!
//! ## Storage Layout
//! - `CorrelationBand(base_asset, quote_asset)` → `CorrelationBand` struct.
//! - `CorrelationPairList` → `Vec<CorrelationPair>` (for enumeration).

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{
    CorrelationBandSetEvent, CorrelationPriceFlaggedEvent, CorrelationViolationEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{CorrelationBand, CorrelationPair, DataKey, ErrorCode};

/// Scaling precision for ratio calculations (10^7).
pub const RATIO_PRECISION: u128 = 10_000_000;

// ─── Admin: configure correlation pairs ──────────────────────────────────────

/// Registers or updates a correlation ratio band for (`base_asset`, `quote_asset`).
///
/// Admin-only. Pass `enabled = false` to disable the check without removing the
/// configuration.
///
/// The ratio checked at submission time is:
/// `ratio = price_base * RATIO_PRECISION / price_quote`
///
/// # Arguments
/// * `env` - Execution environment.
/// * `base_asset` - Asset whose price is the numerator.
/// * `quote_asset` - Asset whose price is the denominator.
/// * `min_ratio` - Minimum acceptable ratio scaled by `RATIO_PRECISION`.
/// * `max_ratio` - Maximum acceptable ratio scaled by `RATIO_PRECISION`.
/// * `enabled` - Whether to enforce this check.
pub fn set_correlation_pair(
    env: &Env,
    base_asset: Address,
    quote_asset: Address,
    min_ratio: u128,
    max_ratio: u128,
    enabled: bool,
) {
    let admin = get_admin(env);
    admin.require_auth();

    if min_ratio >= max_ratio {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let band = CorrelationBand {
        min_ratio,
        max_ratio,
        enabled,
    };

    let key = DataKey::CorrelationBand(base_asset.clone(), quote_asset.clone());
    let is_new = !env.storage().persistent().has(&key);
    env.storage().persistent().set(&key, &band);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Maintain the enumeration list.
    if is_new {
        let list_key = DataKey::CorrelationPairList;
        let mut pairs: Vec<CorrelationPair> = env
            .storage()
            .persistent()
            .get(&list_key)
            .unwrap_or_else(|| Vec::new(env));
        pairs.push_back(CorrelationPair {
            base_asset: base_asset.clone(),
            quote_asset: quote_asset.clone(),
        });
        env.storage().persistent().set(&list_key, &pairs);
        env.storage()
            .persistent()
            .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    CorrelationBandSetEvent {
        base_asset,
        quote_asset,
        min_ratio,
        max_ratio,
        enabled,
    }
    .publish(env);
}

/// Removes the correlation band for (`base_asset`, `quote_asset`). Admin-only.
pub fn remove_correlation_pair(env: &Env, base_asset: Address, quote_asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::CorrelationBand(base_asset.clone(), quote_asset.clone());
    env.storage().persistent().remove(&key);

    // Remove from enumeration list.
    let list_key = DataKey::CorrelationPairList;
    if let Some(pairs) = env
        .storage()
        .persistent()
        .get::<_, Vec<CorrelationPair>>(&list_key)
    {
        let mut new_pairs: Vec<CorrelationPair> = Vec::new(env);
        for i in 0..pairs.len() {
            let p = pairs.get_unchecked(i);
            if p.base_asset != base_asset || p.quote_asset != quote_asset {
                new_pairs.push_back(p);
            }
        }
        env.storage().persistent().set(&list_key, &new_pairs);
    }
}

/// Returns all registered correlation pairs.
pub fn get_correlation_pairs(env: &Env) -> Vec<CorrelationPair> {
    let list_key = DataKey::CorrelationPairList;
    env.storage()
        .persistent()
        .get(&list_key)
        .unwrap_or_else(|| Vec::new(env))
}

/// Returns the correlation band for a pair, if configured.
pub fn get_correlation_band(
    env: &Env,
    base_asset: Address,
    quote_asset: Address,
) -> Option<CorrelationBand> {
    let key = DataKey::CorrelationBand(base_asset, quote_asset);
    env.storage().persistent().get(&key)
}

// ─── Validation (called from submit_price) ────────────────────────────────────

/// Checks all active correlation pairs that involve `submitted_asset` as either
/// base or quote. For each such pair, looks up the current aggregate of the
/// counterpart asset and validates the ratio.
///
/// Returns `true` if all checks pass (or no pairs apply). Returns `false` if any
/// ratio is out of band — the caller should apply a reputation penalty and reject
/// the submission.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `submitted_asset` - The asset that just had a price submitted.
/// * `submitted_price` - The newly submitted price (must be > 0).
/// * `source` - The submitting source (for event emission).
pub fn validate_correlation(
    env: &Env,
    submitted_asset: &Address,
    submitted_price: i128,
    source: &Address,
) -> bool {
    if submitted_price <= 0 {
        return false;
    }

    let pairs = get_correlation_pairs(env);
    if pairs.is_empty() {
        return true;
    }

    let mut all_valid = true;

    for i in 0..pairs.len() {
        let pair = pairs.get_unchecked(i);

        // Determine role of submitted_asset in this pair.
        let (role, counterpart) = if pair.base_asset == *submitted_asset {
            (PairRole::Base, pair.quote_asset.clone())
        } else if pair.quote_asset == *submitted_asset {
            (PairRole::Quote, pair.base_asset.clone())
        } else {
            continue; // this pair doesn't involve the submitted asset
        };

        let band_key = DataKey::CorrelationBand(pair.base_asset.clone(), pair.quote_asset.clone());
        let band: CorrelationBand = match env.storage().persistent().get(&band_key) {
            Some(b) => b,
            None => continue,
        };

        if !band.enabled {
            continue;
        }

        // Look up counterpart's latest aggregate price.
        let counterpart_key = DataKey::Aggregate(counterpart.clone());
        let counterpart_price: i128 = match env
            .storage()
            .persistent()
            .get::<_, crate::types::AggregatePrice>(&counterpart_key)
        {
            Some(agg) => agg.price,
            None => continue, // no price yet for counterpart, skip check
        };

        if counterpart_price <= 0 {
            continue;
        }

        // Compute ratio: base_price * RATIO_PRECISION / quote_price
        let (base_price, quote_price) = match role {
            PairRole::Base => (submitted_price, counterpart_price),
            PairRole::Quote => (counterpart_price, submitted_price),
        };

        // Safe u128 arithmetic. Prices are i128 but must be positive here.
        let base_u = base_price as u128;
        let quote_u = quote_price as u128;

        // Guard against overflow: if base * RATIO_PRECISION would overflow u128,
        // the ratio is astronomically large and therefore out of band.
        let ratio: u128 = match base_u.checked_mul(RATIO_PRECISION) {
            Some(num) => num / quote_u,
            None => u128::MAX,
        };

        if ratio < band.min_ratio || ratio > band.max_ratio {
            CorrelationViolationEvent {
                base_asset: pair.base_asset.clone(),
                quote_asset: pair.quote_asset.clone(),
                source: source.clone(),
                submitted_price,
                counterpart_price,
                ratio,
                min_ratio: band.min_ratio,
                max_ratio: band.max_ratio,
            }
            .publish(env);

            // Flag the (source, submitted_asset) pair so aggregate_asset excludes it.
            let flag_key = DataKey::CorrelationFlagged(source.clone(), submitted_asset.clone());
            env.storage().persistent().set(&flag_key, &true);
            env.storage()
                .persistent()
                .extend_ttl(&flag_key, LEDGER_THRESHOLD, LEDGER_BUMP);

            CorrelationPriceFlaggedEvent {
                asset: submitted_asset.clone(),
                source: source.clone(),
                flagged_price: submitted_price,
            }
            .publish(env);

            all_valid = false;
        }
    }

    all_valid
}

/// Internal role marker so we don't need an enum import at call sites.
enum PairRole {
    Base,
    Quote,
}

/// Returns `true` if the (source, asset) pair has an active correlation flag.
pub fn is_correlation_flagged(env: &Env, source: &Address, asset: &Address) -> bool {
    let key = DataKey::CorrelationFlagged(source.clone(), asset.clone());
    env.storage().persistent().has(&key)
}

/// Clears the correlation flag for a (source, asset) pair. Admin-only.
pub fn clear_correlation_flag(env: &Env, source: Address, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    let key = DataKey::CorrelationFlagged(source, asset);
    env.storage().persistent().remove(&key);
}
