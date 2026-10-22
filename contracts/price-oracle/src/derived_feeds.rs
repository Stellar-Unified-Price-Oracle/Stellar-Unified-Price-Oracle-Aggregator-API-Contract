//! # On-chain derived price feeds (#478)
//!
//! See `docs/derived-feeds.md`.
//!
//! ## Model
//!
//! A derived feed is a pure function of **base prices** and nothing else. A base
//! price for `asset` resolves in exactly this order:
//!
//! 1. [`DataKey::DerivedFeedBase`] — an admin-pinned canonical `(price, ts)`.
//!    Resolved with `from_base = true`.
//! 2. [`DataKey::Aggregate`] — the live aggregate written by `submit_price`.
//!    Resolved with `from_base = false`.
//! 3. Neither → panic [`ErrorCode::UnknownDerivedPair`].
//!
//! The asset itself must be registered (checked with `check_registered_asset`,
//! otherwise [`ErrorCode::AssetNotRegistered`]).
//!
//! Because a derived feed is *only ever* computed from a base price — never from
//! another derived feed — and because a `DerivedFeed` value is never written
//! back to `DataKey::DerivedFeedBase` or `DataKey::Aggregate`, the derivation
//! graph has depth [`MAX_DERIVATION_DEPTH`] (1) and a cycle is *structurally*
//! impossible rather than merely detected. The explicit
//! [`ErrorCode::DerivedFeedCycle`] check is a belt-and-braces guard on the
//! degenerate same-asset request, and [`ErrorCode::DerivedFeedDepthExceeded`]
//! backs the depth bound in code so the invariant is testable.
//!
//! ## Arithmetic and rounding
//!
//! All derivation is `i128` arithmetic on values scaled by `scale = 10^decimals`,
//! where `decimals` is the contract-wide precision read from
//! [`DataKey::CfgDecimals`]. `decimals > 18` is rejected with
//! [`ErrorCode::InvalidConfiguration`]. Every intermediate product is formed in
//! `u128` by [`mul_div`], so a wide product never wraps: a narrow intermediate
//! would reject any price above roughly `170.0` at 18 decimals, which is an
//! ordinary price for most assets. A quotient that genuinely does not fit
//! `i128` is refused with [`ErrorCode::InvalidConfiguration`].
//!
//! | Kind            | Formula                                    | Rounding |
//! |-----------------|--------------------------------------------|----------|
//! | `Inverse`       | `scale * scale / p`                        | truncate toward zero |
//! | `Ratio`         | `p_base * scale / p_quote`                 | truncate toward zero |
//! | `Triangulation` | `r1 = p_base * scale / p_pivot`, then `price = r1 * p_pivot / p_quote` | truncate toward zero **at each step** |
//!
//! Truncation biases every derived value **low**: a consumer of an inverse or a
//! ratio never over-pays. For a triangulation the intermediate truncation is
//! deliberate — it keeps every intermediate inside `i128` and makes the result
//! reproducible from the same inputs — and it costs at most one unit of the
//! pivot's own scale per step, so the result is never above the exact
//! real-number cross rate `p_base / p_quote`.
//!
//! Every division input of `0` panics with
//! [`ErrorCode::DerivedFeedZeroDenominator`]: a zero canonical base models a
//! market with no valid price, and fabricating a `0` (or an infinite inverse)
//! out of it would be worse than refusing.
//!
//! ## Staleness
//!
//! `staleness_secs` is the **worst case** (maximum) age across *all* inputs and
//! `oldest_timestamp` is the minimum input timestamp. A derived feed is only
//! ever as fresh as its stalest input, so propagating the maximum is the only
//! safe rule: anything else would let a stale leg hide behind a fresh one.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::admin::{get_decimals, get_timestamp_threshold};
use crate::events::DerivedFeedComputedEvent;
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    AggregatePrice, DataKey, DerivedFeed, DerivedFeedInput, DerivedFeedKind, ErrorCode,
    MAX_DERIVATION_DEPTH,
};

/// Upper bound on the decimal precision the derivation engine will accept.
///
/// `10^18 * 10^18 = 10^36` is the widest product any formula needs and still
/// fits comfortably inside `i128`; anything above this is refused rather than
/// allowed to overflow.
pub const MAX_DERIVATION_DECIMALS: u32 = 18;

/// A resolved base price plus where it came from.
struct ResolvedBase {
    price: i128,
    timestamp: u64,
    from_base: bool,
}

/// Running worst-case (maximum) staleness accumulator.
///
/// A derived feed is only as fresh as its stalest input, so the propagated
/// staleness is the **maximum** age seen so far and `oldest` the minimum
/// timestamp seen so far.
struct Staleness {
    stalest: u64,
    oldest: u64,
}

impl Staleness {
    /// Seeds the accumulator with the first input's age, so the very first
    /// observation is not skipped.
    fn new(now: u64, timestamp: u64) -> Self {
        Staleness {
            stalest: now.saturating_sub(timestamp),
            oldest: timestamp,
        }
    }

    fn observe(&mut self, now: u64, timestamp: u64) {
        let age = now.saturating_sub(timestamp);
        if age > self.stalest {
            self.stalest = age;
        }
        if timestamp < self.oldest {
            self.oldest = timestamp;
        }
    }
}

/// Computes `scale = 10^decimals` with an explicit overflow guard.
pub fn scale_for(env: &Env, decimals: u32) -> i128 {
    if decimals > MAX_DERIVATION_DECIMALS {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let mut scale: i128 = 1;
    for _ in 0..decimals {
        scale = match scale.checked_mul(10) {
            Some(s) => s,
            None => panic_with_error!(env, ErrorCode::InvalidConfiguration),
        };
    }
    scale
}

/// `a * b / c` for non-negative `a`, `b` and a strictly positive `c`, computed
/// with the product formed in `u128` and the quotient checked back into
/// `i128`.
///
/// The widening is not defensive decoration. `p * 10^decimals` overflows `i128`
/// for any price above roughly `i128::MAX / 10^18` — about 170.0 at 18
/// decimals — which is an ordinary price for most assets, so a narrow
/// intermediate would reject perfectly valid derivations. `u128` widens the
/// headroom to the full `i128` input range, and a quotient that genuinely does
/// not fit `i128` is a configuration problem refused rather than wrapped.
fn mul_div(env: &Env, a: i128, b: i128, c: i128) -> i128 {
    if c <= 0 {
        panic_with_error!(env, ErrorCode::DerivedFeedZeroDenominator);
    }
    let wide = (a as u128) * (b as u128) / (c as u128);
    if wide > i128::MAX as u128 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    wide as i128
}

/// Inverse of a scaled price: `scale * scale / p`, truncated toward zero.
///
/// A `p` of `0` panics with [`ErrorCode::DerivedFeedZeroDenominator`].
pub fn invert(env: &Env, p: i128, scale: i128) -> i128 {
    if p == 0 {
        panic_with_error!(env, ErrorCode::DerivedFeedZeroDenominator);
    }
    mul_div(env, scale, scale, p)
}

/// Pairwise ratio `p_base * scale / p_quote`, truncated toward zero.
///
/// A `p_quote` of `0` panics with [`ErrorCode::DerivedFeedZeroDenominator`].
pub fn ratio(env: &Env, p_base: i128, p_quote: i128, scale: i128) -> i128 {
    if p_quote == 0 {
        panic_with_error!(env, ErrorCode::DerivedFeedZeroDenominator);
    }
    mul_div(env, p_base, scale, p_quote)
}

/// Triangulated cross rate `(base / pivot) * (pivot / quote)`.
///
/// Evaluated left to right in two steps, with the truncation applied at **each**
/// step:
///
/// ```text
/// r1    = p_base * scale / p_pivot      // the scaled leg `base / pivot`
/// price = r1 * p_pivot / p_quote       // r1 scaled by the pivot leg
/// ```
///
/// Note the second step multiplies by `p_pivot` (the pivot's own scaled price),
/// **not** by `scale`: the product `r1 * p_pivot` is already dimensionally a
/// scaled price, so scaling it again would yield `base / (pivot * quote)`
/// rather than the cross rate.
///
/// The second product is the widest intermediate in the module and can exceed
/// `i128` even when the *result* is comfortably in range, so both steps go
/// through [`mul_div`], which forms each product in `u128`. Both prices are
/// known to be `>= 0` here (`set_derived_feed_base` rejects negatives and
/// `submit_price` does too), so the widening is exact. A result that still does
/// not fit `i128` is a configuration problem and is refused rather than
/// wrapped.
///
/// A `p_pivot` or `p_quote` of `0` panics with
/// [`ErrorCode::DerivedFeedZeroDenominator`].
pub fn triangulate(env: &Env, p_base: i128, p_pivot: i128, p_quote: i128, scale: i128) -> i128 {
    if p_pivot == 0 {
        panic_with_error!(env, ErrorCode::DerivedFeedZeroDenominator);
    }
    if p_quote == 0 {
        panic_with_error!(env, ErrorCode::DerivedFeedZeroDenominator);
    }
    // Step 1 — the scaled leg `base / pivot`, truncated toward zero.
    let r1 = mul_div(env, p_base, scale, p_pivot);
    // Step 2 — apply the pivot leg, truncated toward zero.
    mul_div(env, r1, p_pivot, p_quote)
}

/// Enforces the derivation-graph depth bound.
///
/// [`MAX_DERIVATION_DEPTH`] is 1 because inputs are always base prices, so this
/// is trivially satisfied by the live code path; it exists so the bound is
/// explicit in code (and therefore directly testable) rather than only a
/// comment.
pub fn assert_depth(env: &Env, depth: u32) {
    if depth > MAX_DERIVATION_DEPTH {
        panic_with_error!(env, ErrorCode::DerivedFeedDepthExceeded);
    }
}

/// Admin endpoint pinning a canonical base price for derivation.
///
/// Admin only. The asset must be registered. `price == 0` is **accepted on
/// purpose**: a zero canonical base models "this market has no valid price
/// right now", and the derivation engine turns it into
/// [`ErrorCode::DerivedFeedZeroDenominator`] at read time rather than
/// inventing a value. A negative price is rejected with
/// [`ErrorCode::InvalidPrice`], and a timestamp further than the configured
/// `CfgTimestampThreshold` into the future is rejected with
/// [`ErrorCode::InvalidTimestamp`].
pub fn set_derived_feed_base(env: &Env, asset: Address, price: i128, timestamp: u64) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    if price < 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let now = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    if timestamp > now.saturating_add(threshold) {
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    let key = DataKey::DerivedFeedBase(asset);
    env.storage().persistent().set(&key, &(price, timestamp));
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Reads a base price for `asset` following the documented resolution order.
fn resolve_base(env: &Env, asset: &Address) -> ResolvedBase {
    check_registered_asset(env, asset);

    let base_key = DataKey::DerivedFeedBase(asset.clone());
    if let Some((price, timestamp)) = env.storage().persistent().get::<_, (i128, u64)>(&base_key) {
        env.storage()
            .persistent()
            .extend_ttl(&base_key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return ResolvedBase {
            price,
            timestamp,
            from_base: true,
        };
    }

    let agg_key = DataKey::Aggregate(asset.clone());
    if let Some(agg) = env
        .storage()
        .persistent()
        .get::<_, AggregatePrice>(&agg_key)
    {
        env.storage()
            .persistent()
            .extend_ttl(&agg_key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return ResolvedBase {
            price: agg.price,
            timestamp: agg.timestamp,
            from_base: false,
        };
    }

    panic_with_error!(env, ErrorCode::UnknownDerivedPair);
}

/// Returns the inverse feed `1 / asset`.
pub fn get_inverse_feed(env: &Env, asset: Address) -> DerivedFeed {
    compute_derived_feed(env, DerivedFeedKind::Inverse, asset.clone(), asset, None)
}

/// Returns the pairwise ratio feed `base / quote`.
pub fn get_ratio_feed(env: &Env, base: Address, quote: Address) -> DerivedFeed {
    compute_derived_feed(env, DerivedFeedKind::Ratio, base, quote, None)
}

/// Returns the triangulated cross-rate `base / pivot * pivot / quote`.
pub fn get_triangulated_feed(
    env: &Env,
    base: Address,
    pivot: Address,
    quote: Address,
) -> DerivedFeed {
    compute_derived_feed(
        env,
        DerivedFeedKind::Triangulation,
        base,
        quote,
        Some(pivot),
    )
}

/// Computes a derived feed for any kind. `pivot` is only read for triangulation.
///
/// Inputs are recorded in evaluation order — base, then quote, then pivot — and
/// every computation emits [`DerivedFeedComputedEvent`].
pub fn compute_derived_feed(
    env: &Env,
    kind: DerivedFeedKind,
    base: Address,
    quote: Address,
    pivot: Option<Address>,
) -> DerivedFeed {
    // A derived feed is computed only from base prices, so the hop count is
    // always exactly 1. Kept explicit so the bound is enforced, not assumed.
    assert_depth(env, 1);

    let decimals = get_decimals(env);
    let scale = scale_for(env, decimals);
    let now = env.ledger().timestamp();

    let mut inputs: Vec<DerivedFeedInput> = Vec::new(env);
    let mut stale: Option<Staleness> = None;

    // Records one input in evaluation order and folds its age into the
    // worst-case staleness accumulator.
    let record = |inputs: &mut Vec<DerivedFeedInput>,
                  stale: &mut Option<Staleness>,
                  asset: &Address,
                  r: &ResolvedBase| {
        inputs.push_back(DerivedFeedInput {
            asset: asset.clone(),
            price: r.price,
            timestamp: r.timestamp,
            from_base: r.from_base,
        });
        match stale.as_mut() {
            Some(acc) => acc.observe(now, r.timestamp),
            None => *stale = Some(Staleness::new(now, r.timestamp)),
        };
    };

    // For an inverse the `quote` leg *is* the asset, so `inputs` holds a single
    // entry and the feed's `quote` field is the base.
    let price = match kind {
        DerivedFeedKind::Inverse => {
            if pivot.is_some() {
                panic_with_error!(env, ErrorCode::InvalidConfiguration);
            }
            let rb = resolve_base(env, &base);
            record(&mut inputs, &mut stale, &base, &rb);
            invert(env, rb.price, scale)
        }
        DerivedFeedKind::Ratio => {
            if pivot.is_some() {
                panic_with_error!(env, ErrorCode::InvalidConfiguration);
            }
            // A self-pair `a / a` is a degenerate derivation, reported as a
            // cycle rather than an unknown pair: the pair is well known, it is
            // the *graph* that is degenerate.
            if base == quote {
                panic_with_error!(env, ErrorCode::DerivedFeedCycle);
            }
            let rb = resolve_base(env, &base);
            let rq = resolve_base(env, &quote);
            record(&mut inputs, &mut stale, &base, &rb);
            record(&mut inputs, &mut stale, &quote, &rq);
            ratio(env, rb.price, rq.price, scale)
        }
        DerivedFeedKind::Triangulation => {
            let pivot = match pivot {
                Some(p) => p,
                None => panic_with_error!(env, ErrorCode::InvalidConfiguration),
            };
            // Any repeated leg collapses the cross rate onto itself.
            if base == pivot || base == quote || pivot == quote {
                panic_with_error!(env, ErrorCode::DerivedFeedCycle);
            }
            let rb = resolve_base(env, &base);
            let rq = resolve_base(env, &quote);
            let rp = resolve_base(env, &pivot);
            record(&mut inputs, &mut stale, &base, &rb);
            record(&mut inputs, &mut stale, &quote, &rq);
            record(&mut inputs, &mut stale, &pivot, &rp);
            triangulate(env, rb.price, rp.price, rq.price, scale)
        }
    };

    let staleness_secs = stale.as_ref().map(|s| s.stalest).unwrap_or(0);
    let oldest_timestamp = stale.as_ref().map(|s| s.oldest).unwrap_or(now);
    let quote_field = match kind {
        DerivedFeedKind::Inverse => base.clone(),
        _ => quote,
    };

    DerivedFeedComputedEvent {
        base: base.clone(),
        quote: quote_field.clone(),
        kind: kind.as_u32(),
        price,
        staleness_secs,
    }
    .publish(env);

    DerivedFeed {
        kind,
        base,
        quote: quote_field,
        price,
        decimals,
        staleness_secs,
        oldest_timestamp,
        inputs,
    }
}
