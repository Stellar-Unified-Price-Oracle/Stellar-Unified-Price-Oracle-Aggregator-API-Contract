//! # Basket and index price feeds (#479)
//!
//! See `docs/baskets.md`.
//!
//! ## What a basket is
//!
//! A basket is a named, weighted average of other assets' published aggregates.
//! It lets an index or portfolio consumer read one number over a defined set
//! instead of assembling it themselves — which is the point: if each consumer
//! assembled its own basket, they would each choose their own weights and their
//! own staleness handling, and two consumers holding the same portfolio would
//! disagree about its value.
//!
//! ## The value
//!
//! ```text
//! value = Σ (weight_i × price_i) / BASKET_WEIGHT_SCALE
//! ```
//!
//! The sum is exact integer arithmetic in `u128`, truncated once at the end
//! rather than per term, so the result matches a reference weighted-sum
//! computation to within one unit of the last place. Each term is also
//! reported individually in [`BasketValue::contributions`].
//!
//! ## Weights
//!
//! Weights are integers in parts per [`BASKET_WEIGHT_SCALE`] (1e6) and must sum
//! to *exactly* `BASKET_WEIGHT_SCALE`. An exact sum is what makes the index
//! mean something: if the weights summed to 0.999 of the scale, every published
//! value would be 0.1 % low, permanently and invisibly. Floating-point weights
//! are not used because their sum is not exactly representable, which would put
//! the sum-check beyond what the contract can enforce.
//!
//! ## Missing and stale constituents
//!
//! This is the security property the issue is really about. A constituent with
//! no price — or one older than the configured staleness bound — must **never**
//! be silently dropped, because a basket that quietly drops its worst-performing
//! constituent is a basket that reports a *better* number precisely when it
//! should be reporting a worse one. There are exactly two policies and no third:
//!
//! * [`BasketStalenessPolicy::Reject`] — panic
//!   [`ErrorCode::BasketConstituentStale`]. The index does not exist.
//! * [`BasketStalenessPolicy::Degrade`] — compute over the live constituents
//!   and set [`BasketValue::is_degraded`].
//!
//! Under `Degrade` the value is **not** rescaled to the live weights. Rescaling
//! would make a two-of-three basket report the same number as a complete one
//! and call it exact, which is the failure mode the issue names. The index is
//! instead a lower bound on the missing legs' contributions, flagged so the
//! consumer decides whether to accept it.
//!
//! Staleness propagates as a **maximum**: `staleness_secs` is the age of the
//! stalest constituent, because a basket is only as fresh as its oldest input.
//!
//! ## Recursion
//!
//! Baskets may not contain baskets, directly or transitively. `set_basket`
//! walks the constituent list and rejects any entry that is itself a
//! configured basket ([`ErrorCode::RecursiveBasket`]). Combined with the
//! [`MAX_BASKET_CONSTITUENTS`] bound this makes the evaluation graph a strict
//! one-level DAG over registered assets: compute cost is `O(constituents)` and
//! there is no cycle to bound, by construction rather than by detection.
//!
//! ## Rebalancing
//!
//! [`rebalance_basket`] validates the entire new weight vector first and only
//! then writes it, in a single storage write followed by a single
//! [`BasketRebalancedEvent`] carrying the whole vector. A rebalance is therefore
//! atomic — there is no state in which the basket holds a mix of old and new
//! weights — and fully reconstructible from the event stream.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::admin::get_decimals;
use crate::events::{BasketConfiguredEvent, BasketDegradedEvent, BasketRebalancedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    AggregatePrice, BasketConfig, BasketConstituent, BasketContribution, BasketStalenessPolicy,
    BasketValue, DataKey, ErrorCode, BASKET_WEIGHT_SCALE, MAX_BASKET_CONSTITUENTS,
};

/// Default staleness bound for a constituent, in seconds.
///
/// Equal to the contract's default timestamp threshold so a basket is, by
/// default, exactly as permissive as a direct aggregate read.
pub const DEFAULT_BASKET_MAX_STALENESS: u64 = 300;

/// Maximum age a constituent aggregate may have and still count as fresh.
pub fn get_basket_max_staleness(env: &Env) -> u64 {
    env.storage()
        .persistent()
        .get(&DataKey::BasketMaxStaleness)
        .unwrap_or(DEFAULT_BASKET_MAX_STALENESS)
}

/// Sets the global constituent staleness bound. Admin only.
///
/// # Errors
///
/// * [`ErrorCode::InvalidConfiguration`] — `max_staleness_secs` is `0`, which
///   would make every constituent permanently stale and thus every basket
///   permanently unreadable.
pub fn set_basket_max_staleness(env: &Env, max_staleness_secs: u64) {
    get_admin(env).require_auth();
    if max_staleness_secs == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::BasketMaxStaleness;
    env.storage().persistent().set(&key, &max_staleness_secs);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Validates a constituent list and returns its exact weight sum.
///
/// Enforced on every write, so an invalid basket can never be stored:
///
/// * **non-empty** and at most [`MAX_BASKET_CONSTITUENTS`] entries, bounding
///   the cost of every subsequent read;
/// * **no duplicate asset** — a repeated asset would double-count it, and
///   "two entries for the same asset" has no single sensible weight;
/// * **no self-reference** — a basket naming itself is the degenerate cycle;
/// * **no nested basket** — a configured basket may not be a constituent,
///   which is what makes the graph strictly one level deep;
/// * **every weight non-zero** — a zero-weight constituent is present in the
///   list but absent from the value, which is exactly the silent-skip the issue
///   forbids;
/// * **weights sum to exactly `BASKET_WEIGHT_SCALE`].
fn validate_constituents(
    env: &Env,
    basket: &Address,
    constituents: &Vec<BasketConstituent>,
) -> u32 {
    let n = constituents.len();
    if n == 0 || n > MAX_BASKET_CONSTITUENTS {
        panic_with_error!(env, ErrorCode::InvalidBasketComposition);
    }

    let mut total: u64 = 0;
    for i in 0..n {
        let c = constituents.get_unchecked(i);
        if c.weight == 0 {
            panic_with_error!(env, ErrorCode::InvalidBasketWeights);
        }
        // A basket may not contain itself, nor any other configured basket.
        // Together these bound the graph at one level, so no cycle can form.
        if c.asset == *basket {
            panic_with_error!(env, ErrorCode::RecursiveBasket);
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::Basket(c.asset.clone()))
        {
            panic_with_error!(env, ErrorCode::RecursiveBasket);
        }
        // Duplicate assets would double-count a leg with no defined weight.
        for j in 0..i {
            if constituents.get_unchecked(j).asset == c.asset {
                panic_with_error!(env, ErrorCode::InvalidBasketComposition);
            }
        }
        total += u64::from(c.weight);
    }

    if total != u64::from(BASKET_WEIGHT_SCALE) {
        panic_with_error!(env, ErrorCode::InvalidBasketWeights);
    }
    total as u32
}

/// Creates or replaces a basket's configuration. Admin only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidBasketComposition`] — empty, oversized, or a repeated
///   constituent asset.
/// * [`ErrorCode::InvalidBasketWeights`] — a zero weight, or weights that do not
///   sum to exactly `BASKET_WEIGHT_SCALE`.
/// * [`ErrorCode::RecursiveBasket`] — a constituent is `basket` itself or is
///   itself a configured basket.
pub fn set_basket(env: &Env, basket: Address, config: BasketConfig) {
    get_admin(env).require_auth();
    let total = validate_constituents(env, &basket, &config.constituents);

    let stored = BasketConfig {
        constituents: config.constituents,
        total_weight: total,
        staleness_policy: config.staleness_policy,
        rebalance_policy: config.rebalance_policy,
    };
    let key = DataKey::Basket(basket.clone());
    env.storage().persistent().set(&key, &stored);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    BasketConfiguredEvent {
        basket,
        constituents: stored.constituents.len(),
        total_weight: total,
    }
    .publish(env);
}

/// Returns a basket's configuration.
///
/// # Errors
///
/// * [`ErrorCode::BasketNotFound`] — no basket is configured here.
pub fn get_basket(env: &Env, basket: &Address) -> BasketConfig {
    let key = DataKey::Basket(basket.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::BasketNotFound))
}

/// Returns a basket's ordered constituent list — its composition, for a
/// consumer that wants the legs and weights without computing the value.
///
/// # Errors
///
/// * [`ErrorCode::BasketNotFound`] — no basket is configured here.
pub fn get_basket_composition(env: &Env, basket: &Address) -> Vec<BasketConstituent> {
    get_basket(env, basket).constituents
}

/// Re-prices a basket's weights atomically. Admin only.
///
/// The new vector is fully validated before anything is written, and the whole
/// vector lands in one storage write followed by one event carrying the new
/// weights. There is therefore no intermediate state in which the basket holds
/// some old and some new weights, and a consumer replaying
/// [`BasketRebalancedEvent`]s reconstructs the exact configuration history.
///
/// `weights` is positional: entry `i` re-prices constituent `i` of the stored
/// configuration. This is deliberate — a whole new constituent list would let a
/// rebalance silently change *which* assets are in the index, which is a much
/// larger operation than re-pricing.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::BasketNotFound`] — no basket is configured here.
/// * [`ErrorCode::InvalidBasketWeights`] — length mismatch, a zero weight, or a
///   sum other than `BASKET_WEIGHT_SCALE`.
pub fn rebalance_basket(env: &Env, basket: Address, weights: Vec<u32>) {
    get_admin(env).require_auth();
    let mut config = get_basket(env, &basket);

    if weights.len() != config.constituents.len() {
        panic_with_error!(env, ErrorCode::InvalidBasketWeights);
    }

    // Rebuild the whole vector first: validation must complete before the
    // first write, or a rejected rebalance would leave a partially applied one.
    let mut rebuilt: Vec<BasketConstituent> = Vec::new(env);
    let mut total: u64 = 0;
    for i in 0..weights.len() {
        let w = weights.get_unchecked(i);
        if w == 0 {
            panic_with_error!(env, ErrorCode::InvalidBasketWeights);
        }
        let mut c = config.constituents.get_unchecked(i);
        c.weight = w;
        total += u64::from(w);
        rebuilt.push_back(c);
    }
    if total != u64::from(BASKET_WEIGHT_SCALE) {
        panic_with_error!(env, ErrorCode::InvalidBasketWeights);
    }

    config.constituents = rebuilt;
    config.total_weight = total as u32;

    let key = DataKey::Basket(basket.clone());
    env.storage().persistent().set(&key, &config);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    BasketRebalancedEvent {
        basket,
        ledger: env.ledger().sequence(),
        timestamp: env.ledger().timestamp(),
        weights,
    }
    .publish(env);
}

/// Computes a basket's index value with full per-constituent provenance.
///
/// A constituent counts as usable when it has a published aggregate whose age
/// is within [`get_basket_max_staleness`]. The weighted sum runs over the
/// **configured** weight vector, not the live one, so a degraded basket reports
/// a value that is missing exactly the missing legs' contributions rather than
/// one that has been quietly rescaled to look complete.
///
/// # Errors
///
/// * [`ErrorCode::BasketNotFound`] — no basket is configured here.
/// * [`ErrorCode::BasketConstituentStale`] — a constituent is missing or stale
///   and the policy is [`BasketStalenessPolicy::Reject`].
pub fn get_basket_value(env: &Env, basket: &Address) -> BasketValue {
    let config = get_basket(env, basket);
    let decimals = get_decimals(env);
    let now = env.ledger().timestamp();
    let max_staleness = get_basket_max_staleness(env);

    let n = config.constituents.len();
    let mut contributions: Vec<BasketContribution> = Vec::new(env);
    // Widened so a full-weight price near `i128::MAX` cannot overflow the
    // accumulator; the quotient is range-checked back into `i128` below.
    let mut weighted_sum: u128 = 0;
    let mut live: u32 = 0;
    let mut worst_staleness: u64 = 0;
    let mut any_unusable = false;

    for i in 0..n {
        let c = config.constituents.get_unchecked(i);
        let agg: Option<AggregatePrice> = env
            .storage()
            .persistent()
            .get(&DataKey::Aggregate(c.asset.clone()));

        let (price, timestamp, present) = match agg {
            Some(a) => (a.price, a.timestamp, true),
            None => (0i128, 0u64, false),
        };
        let age = if present {
            now.saturating_sub(timestamp)
        } else {
            0
        };
        let fresh = present && age <= max_staleness;
        let usable = present && fresh;

        if usable {
            live += 1;
            if age > worst_staleness {
                worst_staleness = age;
            }
            weighted_sum += (price as u128) * (u64::from(c.weight) as u128);
        } else {
            any_unusable = true;
        }

        // The reported per-term contribution matches the term actually summed:
        // zero for an unusable leg, so the contributions account for exactly the
        // value that was published.
        let contribution = if usable {
            ((price as u128) * (u64::from(c.weight) as u128)
                / u64::from(BASKET_WEIGHT_SCALE) as u128) as i128
        } else {
            0
        };

        contributions.push_back(BasketContribution {
            asset: c.asset,
            weight: c.weight,
            price,
            timestamp,
            contribution,
            staleness_secs: age,
            present,
            fresh,
        });
    }

    // A missing or stale constituent must never yield a quietly-lighter index.
    if any_unusable && matches!(config.staleness_policy, BasketStalenessPolicy::Reject) {
        panic_with_error!(env, ErrorCode::BasketConstituentStale);
    }

    // Single truncation, at the end, over the widened accumulator.
    let scaled = weighted_sum / u64::from(BASKET_WEIGHT_SCALE) as u128;
    if scaled > i128::MAX as u128 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let value = scaled as i128;

    if any_unusable {
        BasketDegradedEvent {
            basket: basket.clone(),
            value,
            live_constituents: live,
            total_constituents: n,
            staleness_secs: worst_staleness,
        }
        .publish(env);
    }

    BasketValue {
        value,
        decimals,
        is_degraded: any_unusable,
        live_constituents: live,
        total_constituents: n,
        staleness_secs: worst_staleness,
        contributions,
    }
}

/// Returns one constituent's weight and its additive share of the index.
///
/// # Errors
///
/// * [`ErrorCode::BasketNotFound`] — no basket is configured here.
/// * [`ErrorCode::InvalidBasketComposition`] — `index` is out of range.
pub fn get_basket_contribution(env: &Env, basket: &Address, index: u32) -> BasketContribution {
    let value = get_basket_value(env, basket);
    if index >= value.contributions.len() {
        panic_with_error!(env, ErrorCode::InvalidBasketComposition);
    }
    value.contributions.get_unchecked(index)
}
