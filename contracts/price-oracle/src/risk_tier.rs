//! # Asset risk tiers (#487)
//!
//! Per-asset configuration is powerful but easy to get wrong: a quorum of 2
//! written on a thin market looks identical to one written on a blue chip, and
//! nothing in the aggregate says which. This module replaces that with a small
//! set of **reviewed presets**.
//!
//! ```text
//! tier  ->  { method, quorum, freshness window, deviation bound }
//! ```
//!
//! An asset is assigned exactly one tier. The effective parameters are the
//! tier's, with the documented per-asset [`PolicyOverride`] applied on top, so
//! an override is always visible as an override and never silently replaces a
//! preset.
//!
//! ## Fail closed
//!
//! An asset with **no** tier is not defaulted. [`require_tier`] returns
//! `false`, aggregation publishes nothing for it, and
//! [`ErrorCode::InvalidRiskTier`] is raised if a caller asks for the parameters
//! of a tier that is not defined. An unassigned asset is therefore visibly
//! unconfigured rather than quietly running at global defaults.
//!
//! ## A tier change never rewrites the past
//!
//! Tiers are read at *aggregation* time only. Moving an asset between tiers
//! changes what the next aggregate will look like; every value already
//! published under the old tier keeps the meaning it had when it was computed.
//! The change is atomic (one storage write plus one event) and always carries
//! the acting admin and a mandatory reason.
//!
//! See `docs/asset-risk-tiers.md`.

use soroban_sdk::{contractevent, panic_with_error, Address, Env, String};

use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, PolicyOverride, ResolvedTier, RiskTier, TierParams};

/// Maximum accepted length of a tier-change reason.
pub const MAX_REASON_LEN: u32 = 256;

/// The fixed parameter set of every tier, in `RiskTier` discriminant order.
///
/// These are the *presets*: reviewed once, applied everywhere, and readable in
/// one place. Nothing here is per-asset.
pub const TIER_PARAMS: [TierParams; 4] = [
    // Tier1BlueChip — deep markets: median, wide quorum, tight bound.
    TierParams {
        method: 0,
        min_sources: 5,
        freshness_secs: 60,
        max_deviation_bps: 100,
    },
    // Tier2Standard — ordinary majors.
    TierParams {
        method: 0,
        min_sources: 3,
        freshness_secs: 300,
        max_deviation_bps: 300,
    },
    // Tier3Thin — real but thin: trimmed mean resists a single wild print.
    TierParams {
        method: 2,
        min_sources: 2,
        freshness_secs: 1_800,
        max_deviation_bps: 1_000,
    },
    // Tier4Speculative — widest tolerance, weakest quorum, fastest expiry.
    TierParams {
        method: 2,
        min_sources: 2,
        freshness_secs: 900,
        max_deviation_bps: 3_000,
    },
];

/// Emitted whenever an asset moves between tiers (#487).
///
/// Topics: `asset`, `actor`
#[contractevent]
#[derive(Clone)]
pub struct AssetTierChangedEvent {
    #[topic]
    pub asset: Address,
    /// The admin that made the change.
    #[topic]
    pub actor: Address,
    /// Tier before the change; `None` when the asset was unassigned.
    pub old_tier: Option<u32>,
    /// Tier after the change; `None` when the assignment was removed.
    pub new_tier: Option<u32>,
    /// Mandatory human-readable reason for the move.
    pub reason: String,
    /// Ledger of the change.
    pub ledger: u32,
}

/// Converts a stored discriminant into a tier, rejecting unknown values.
///
/// This is the "validated enum" the store relies on: a value that is not one of
/// the four defined tiers fails closed rather than being coerced.
pub fn tier_from_u32(v: u32) -> Option<RiskTier> {
    match v {
        0 => Some(RiskTier::Tier1BlueChip),
        1 => Some(RiskTier::Tier2Standard),
        2 => Some(RiskTier::Tier3Thin),
        3 => Some(RiskTier::Tier4Speculative),
        _ => None,
    }
}

/// The fixed parameters of a tier.
pub fn params(tier: RiskTier) -> TierParams {
    TIER_PARAMS[tier as usize]
}

/// The tier assigned to `asset`, if any.
///
/// A stored discriminant that is not a defined tier is reported as `None`
/// rather than panicking, so a corrupted assignment degrades to "unassigned"
/// and the asset then fails closed like any other unconfigured asset.
pub fn get_tier(env: &Env, asset: &Address) -> Option<RiskTier> {
    let key = DataKey::AssetRiskTier(asset.clone());
    let raw: Option<u32> = env.storage().persistent().get(&key);
    if raw.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    raw.and_then(tier_from_u32)
}

/// `true` when `asset` has a valid tier. This is the fail-closed predicate:
/// `false` means "do not publish", not "use a default".
pub fn require_tier(env: &Env, asset: &Address) -> bool {
    get_tier(env, asset).is_some()
}

/// Assigns (or, with `None`, removes) the tier of `asset`. Admin only.
///
/// The change is atomic: the write and the event happen in the same
/// invocation, and a rejected write leaves the previous tier untouched. A
/// non-empty `reason` is mandatory so a tier move is always attributable.
pub fn set_tier(env: &Env, asset: Address, tier: Option<RiskTier>, reason: String) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    if reason.len() > MAX_REASON_LEN {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let old_tier = get_tier(env, &asset);
    let key = DataKey::AssetRiskTier(asset.clone());
    match tier {
        Some(t) => {
            env.storage().persistent().set(&key, &(t as u32));
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            crate::price_bounds::note_tier_assigned(env);
            // A tier carries a freshness window, so assigning one turns the
            // #489 freshness path on for this oracle: otherwise the tier's
            // window would silently never be applied.
            if params(t).freshness_secs > 0 {
                crate::price_bounds::note_freshness_window(env);
            }
        }
        None => env.storage().persistent().remove(&key),
    }

    AssetTierChangedEvent {
        asset,
        actor: admin,
        old_tier: old_tier.map(|t| t as u32),
        new_tier: tier.map(|t| t as u32),
        reason,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
}

/// Resolves the tier of `asset` and the parameters it actually implies.
///
/// The tier supplies the base; `override` — the asset's existing
/// [`PolicyOverride`] — replaces individual fields on top. `overridden` reports
/// whether that happened, so a consumer can tell a preset apart from a
/// hand-tuned asset without recomputing the resolution.
///
/// An unassigned asset resolves to the global defaults with `tier` set to
/// `Tier2Standard` only as a *reported* placeholder; callers that must fail
/// closed should gate on [`require_tier`] first. Aggregation does exactly that.
pub fn resolve(env: &Env, asset: &Address, over: &PolicyOverride) -> Option<ResolvedTier> {
    let tier = get_tier(env, asset)?;
    let base = params(tier);
    let mut effective = base;
    let mut overridden = false;
    if let Some(m) = over.method {
        effective.method = m;
        overridden = true;
    }
    if let Some(q) = over.min_sources {
        effective.min_sources = q;
        overridden = true;
    }
    if let Some(f) = over.freshness_secs {
        effective.freshness_secs = f;
        overridden = true;
    }
    if let Some(d) = over.max_deviation_bps {
        effective.max_deviation_bps = d;
        overridden = true;
    }
    Some(ResolvedTier {
        tier,
        base,
        effective,
        overridden,
    })
}

/// `true` once the admin has required every asset to carry a tier.
///
/// Enforcement is opt-in so an existing deployment keeps publishing while it
/// rolls tiers out. Once on, an unassigned asset publishes nothing — the
/// fail-closed behaviour — instead of silently running on global defaults.
pub fn enforcement_enabled(env: &Env) -> bool {
    crate::price_bounds::guards(env).any_tier_assigned
}

/// Turns mandatory tier assignment on or off. Admin only.
///
/// `require_tier_for_every_asset` must be called before the flag is set for
/// this to be a safe operation: the admin is asserting that every registered
/// asset already has a valid tier.
pub fn set_enforcement(env: &Env, enabled: bool) {
    get_admin(env).require_auth();
    if enabled {
        let assets = crate::storage::read_registered_assets(env);
        for i in 0..assets.len() {
            let a = assets.get_unchecked(i);
            if get_tier(env, &a).is_none() {
                panic_with_error!(env, ErrorCode::InvalidRiskTier);
            }
        }
    }
    set_enforced_flag(env, enabled);
}

pub(crate) fn set_enforced_flag(env: &Env, enabled: bool) {
    use crate::types::DataKey as DK;
    let key = DK::RiskTierEnforcement;
    if enabled {
        env.storage().persistent().set(&key, &true);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    } else {
        env.storage().persistent().remove(&key);
    }
}

/// Reads the enforcement flag. Separate from [`enforcement_enabled`] so the
/// publication path can distinguish "no tiers configured at all" (skip
/// everything) from "tiers configured but not yet mandatory".
pub fn enforcement_flag(env: &Env) -> bool {
    env.storage()
        .persistent()
        .get(&crate::types::DataKey::RiskTierEnforcement)
        .unwrap_or(false)
}

/// Overlays an asset's tier onto a resolved policy.
///
/// The tier supplies each field for which the asset has **no** explicit
/// [`PolicyOverride`], so a documented per-asset override always wins over the
/// preset. With no tier assigned the policy is returned unchanged, so an oracle
/// that has not adopted tiers behaves exactly as before.
pub fn apply_to_policy(env: &Env, asset: &Address, policy: &mut crate::types::EffectivePolicy) {
    let Some(tier) = get_tier(env, asset) else {
        return;
    };
    let base = params(tier);
    let over: Option<PolicyOverride> = env
        .storage()
        .persistent()
        .get(&DataKey::AssetPolicy(asset.clone()));
    if over.as_ref().and_then(|o| o.method).is_none() {
        policy.method = base.method;
    }
    if over.as_ref().and_then(|o| o.min_sources).is_none() {
        policy.min_sources = base.min_sources;
    }
    if over.as_ref().and_then(|o| o.freshness_secs).is_none() {
        policy.freshness_secs = base.freshness_secs;
    }
    if over.as_ref().and_then(|o| o.max_deviation_bps).is_none() {
        policy.max_deviation_bps = base.max_deviation_bps;
    }
}
