//! # Two-tier price bounds (#484)
//!
//! A single bounds mechanism forces a choice between failing the whole feed
//! (reject) and silently distorting it (clamp). This module splits that into
//! two explicitly-configured tiers per asset:
//!
//! ```text
//! hard_min  <=  soft_min  <=  published  <=  soft_max  <=  hard_max
//! ```
//!
//! * **Soft bounds** clamp the aggregate and mark it `clamped` with a
//!   consumer-visible reason code ([`BoundReason::ClampedToSoftMin`] /
//!   [`BoundReason::ClampedToSoftMax`]). Clamping is never silent: the
//!   [`BoundStatus`] query and the `PriceClampedEvent` both carry the raw
//!   value, the published value and the reason code.
//! * **Hard bounds** reject the aggregate outright: it is not published, not
//!   written to history and not allowed into further aggregation. The
//!   `PriceBoundRejectedEvent` records the rejected value and the previous
//!   aggregate stays live, so the feed degrades loudly instead of halting.
//!
//! Ordering is validated on write ([`set_bounds`]), so a stored tier is always
//! well-formed; a violation panics with [`ErrorCode::InvalidBoundOrdering`].
//! See `docs/price-bounds-tiers.md` for consumer guidance.

use soroban_sdk::{contractevent, panic_with_error, Address, Env};

use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{BoundReason, BoundStatus, BoundsTier, DataKey, ErrorCode, PublicationGuards};

/// Emitted when an aggregate is clamped to a soft bound (#484).
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct PriceClampedEvent {
    #[topic]
    pub asset: Address,
    /// The aggregate before clamping.
    pub raw_price: i128,
    /// The published (clamped) value.
    pub clamped_price: i128,
    /// `BoundReason::ClampedToSoftMin` or `BoundReason::ClampedToSoftMax`.
    pub reason_code: u32,
    /// Ledger of the clamp.
    pub ledger: u32,
}

/// Emitted when an aggregate is rejected by the hard bounds (#484).
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct PriceBoundRejectedEvent {
    #[topic]
    pub asset: Address,
    /// The rejected aggregate value.
    pub raw_price: i128,
    /// `BoundReason::RejectedBelowHardMin` or `RejectedAboveHardMax`.
    pub reason_code: u32,
    /// The last aggregate that stayed live, for context.
    pub last_published_price: i128,
    /// Ledger of the rejection.
    pub ledger: u32,
}

/// `0 < hard_min <= soft_min` and `soft_min <= soft_max <= hard_max`.
pub fn validate(env: &Env, t: &BoundsTier) {
    if t.soft_min <= 0
        || t.soft_max < t.soft_min
        || t.hard_min <= 0
        || t.hard_min > t.soft_min
        || t.hard_max < t.soft_max
    {
        panic_with_error!(env, ErrorCode::InvalidBoundOrdering);
    }
}

/// Sets the soft/hard bounds for `asset`. Admin only; ordering is validated.
pub fn set_bounds(env: &Env, asset: Address, tier: BoundsTier) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    validate(env, &tier);
    let key = DataKey::AssetBoundsTier(asset.clone());
    env.storage().persistent().set(&key, &tier);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    note_bounds_configured(env);
}

/// The global publication-path guards, read once per aggregation pass.
pub fn guards(env: &Env) -> PublicationGuards {
    env.storage()
        .persistent()
        .get(&DataKey::PublicationGuards)
        .unwrap_or_default()
}

/// Records that at least one asset has bounds configured.
pub fn note_bounds_configured(env: &Env) {
    update_guards(env, |g| g.any_bounds_configured = true);
}

/// Records that at least one asset defers publication.
pub fn note_deferral_configured(env: &Env) {
    update_guards(env, |g| g.any_deferral_configured = true);
}

/// Records that corrections have been enabled for the oracle.
pub fn note_correction_scope_configured(env: &Env) {
    update_guards(env, |g| g.any_corrections_enabled = true);
}

/// Records that at least one source is ineligible to contribute.
pub fn note_source_excluded(env: &Env) {
    update_guards(env, |g| g.any_source_excluded = true);
}

/// Records that at least one asset has a risk tier assigned (#487).
pub fn note_tier_assigned(env: &Env) {
    update_guards(env, |g| g.any_tier_assigned = true);
}

/// Records that at least one cross-asset sanity relation exists (#488).
pub fn note_sanity_relation(env: &Env) {
    update_guards(env, |g| g.any_sanity_relations = true);
}

/// Records that at least one freshness window is configured (#489).
pub fn note_freshness_window(env: &Env) {
    update_guards(env, |g| g.any_freshness_window = true);
}

/// Turns scorecard collection on or off (#490).
pub fn set_scorecards_enabled(env: &Env, enabled: bool) {
    update_guards(env, |g| g.scorecards_enabled = enabled);
}

fn update_guards(env: &Env, f: impl FnOnce(&mut PublicationGuards)) {
    let key = DataKey::PublicationGuards;
    let mut g = guards(env);
    f(&mut g);
    env.storage().persistent().set(&key, &g);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns the configured tier for `asset`, if any.
pub fn get_bounds(env: &Env, asset: &Address) -> Option<BoundsTier> {
    let key = DataKey::AssetBoundsTier(asset.clone());
    let v: Option<BoundsTier> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Clears the tier for `asset`, restoring unclamped, unvalidated aggregation.
pub fn clear_bounds(env: &Env, asset: Address) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    env.storage()
        .persistent()
        .remove(&DataKey::AssetBoundsTier(asset));
}

/// Outcome of evaluating a value against an asset's tier.
pub struct BoundDecision {
    /// The value that may be published.
    pub price: i128,
    /// True when the value was clamped into the soft band.
    pub clamped: bool,
    /// True when the value was outside the hard band and must not be published.
    pub rejected: bool,
    /// Consumer-visible reason code ([`BoundReason`]).
    pub reason_code: u32,
}

/// Evaluates `price` against `asset`'s tier without touching storage.
///
/// With no tier configured the value passes through unchanged, so assets that
/// never opted in keep the pre-#484 behaviour.
pub fn evaluate(env: &Env, asset: &Address, price: i128) -> BoundDecision {
    // Short-circuit the common case: with no asset opted in there is no stored
    // tier to look up. (`apply` has already read the guards, so this is only
    // reached on the direct-call path.)
    if !guards(env).any_bounds_configured {
        return BoundDecision {
            price,
            clamped: false,
            rejected: false,
            reason_code: BoundReason::InBounds as u32,
        };
    }
    match get_bounds(env, asset) {
        Some(t) => evaluate_tier(&t, price),
        None => BoundDecision {
            price,
            clamped: false,
            rejected: false,
            reason_code: BoundReason::InBounds as u32,
        },
    }
}

/// Pure tier evaluation, split out so it can be property-tested without storage.
pub fn evaluate_tier(t: &BoundsTier, price: i128) -> BoundDecision {
    if price < t.hard_min {
        return BoundDecision {
            price,
            clamped: false,
            rejected: true,
            reason_code: BoundReason::RejectedBelowHardMin as u32,
        };
    }
    if price > t.hard_max {
        return BoundDecision {
            price,
            clamped: false,
            rejected: true,
            reason_code: BoundReason::RejectedAboveHardMax as u32,
        };
    }
    if price < t.soft_min {
        return BoundDecision {
            price: t.soft_min,
            clamped: true,
            rejected: false,
            reason_code: BoundReason::ClampedToSoftMin as u32,
        };
    }
    if price > t.soft_max {
        return BoundDecision {
            price: t.soft_max,
            clamped: true,
            rejected: false,
            reason_code: BoundReason::ClampedToSoftMax as u32,
        };
    }
    BoundDecision {
        price,
        clamped: false,
        rejected: false,
        reason_code: BoundReason::InBounds as u32,
    }
}

/// Records the decision, emits the matching event, and returns the publishable
/// value. Called by the aggregation path with the raw aggregate.
///
/// Returns `(publishable, is_degraded)`; `publishable` is `None` when the
/// aggregate was rejected and must not be published or fed into any further
/// aggregation step. An asset without the bounds bit set is a pure pass-through
/// and writes nothing at all, so opting out costs the single flag read.
pub fn apply(env: &Env, asset: &Address, raw_price: i128) -> (Option<i128>, bool) {
    let guard_bounds = guards(env).any_bounds_configured;
    if !guard_bounds {
        return (Some(raw_price), false);
    }
    let d = evaluate(env, asset, raw_price);
    let ledger = env.ledger().sequence();

    if d.rejected {
        let last_published: i128 = env
            .storage()
            .persistent()
            .get::<DataKey, crate::types::AggregatePrice>(&DataKey::Aggregate(asset.clone()))
            .map(|a| a.price)
            .unwrap_or(0);
        PriceBoundRejectedEvent {
            asset: asset.clone(),
            raw_price,
            reason_code: d.reason_code,
            last_published_price: last_published,
            ledger,
        }
        .publish(env);
        write_status(env, asset, raw_price, raw_price, d.reason_code, false);
        return (None, true);
    }

    if d.clamped {
        PriceClampedEvent {
            asset: asset.clone(),
            raw_price,
            clamped_price: d.price,
            reason_code: d.reason_code,
            ledger,
        }
        .publish(env);
    }
    write_status(env, asset, raw_price, d.price, d.reason_code, d.clamped);
    (Some(d.price), d.clamped)
}

fn write_status(env: &Env, asset: &Address, raw: i128, price: i128, reason: u32, clamped: bool) {
    let status = BoundStatus {
        raw_price: raw,
        price,
        clamped,
        // A rejection is not a clamp: the two flags are mutually exclusive so
        // a consumer can branch on either without ambiguity.
        rejected: !clamped && reason >= BoundReason::RejectedBelowHardMin as u32,
        reason_code: reason,
        ledger: env.ledger().sequence(),
    };
    let key = DataKey::AssetBoundStatus(asset.clone());
    env.storage().persistent().set(&key, &status);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// The last bound decision published for `asset`, for consumer inspection.
pub fn get_status(env: &Env, asset: &Address) -> Option<BoundStatus> {
    let key = DataKey::AssetBoundStatus(asset.clone());
    let v: Option<BoundStatus> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}
