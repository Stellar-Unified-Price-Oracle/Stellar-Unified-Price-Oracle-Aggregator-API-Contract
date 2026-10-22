//! # Aggregate recomputation on source-set change (#483)
//!
//! Removing, de-registering or disqualifying a source must not leave its last
//! value sitting inside the published median. Every source-set mutation calls
//! [`recompute_source_change`], which synchronously re-derives the aggregate
//! for each affected asset from the *surviving* source set and emits the
//! resulting aggregate in the same transaction as the removal.
//!
//! ## Why this is atomic and convergent
//!
//! * **No partial state.** The aggregate, its history entry and the
//!   `AggregateRecomputedEvent` are written in the same transaction as the
//!   source-set mutation, so an observer never sees a removal without the
//!   matching recomputation.
//! * **No double counting.** The source is removed from the registry *before*
//!   recomputation runs, so its prior submission is skipped by the aggregation
//!   loop exactly like a source that had never submitted.
//! * **Batch == sequential.** Recomputation is a pure function of the surviving
//!   registry, so removing `A` then `B` and removing `{A, B}` in one call
//!   converge on the same aggregate.
//!
//! Recomputation costs one aggregation pass per asset that already has an
//! aggregate, capped by [`MAX_RECOMPUTE_ASSETS`]; it never walks history, so it
//! is cheap enough to run inside the removal transaction.

use soroban_sdk::{contractevent, contracttype, Address, Env, Vec};

use crate::prices::recompute_asset;
use crate::storage::{is_source_inactive, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregatePrice, DataKey};

/// Upper bound on how many assets a single source-set change re-derives.
pub const MAX_RECOMPUTE_ASSETS: u32 = 64;

/// Why a recomputation was triggered.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum RecomputeReason {
    /// The source was de-registered entirely.
    Removed = 0,
    /// The source lost its claim on a single asset.
    AssetClaimRemoved = 1,
    /// The source was disqualified by the demerit system.
    Disqualified = 2,
    /// The source was marked inactive (suspended).
    Suspended = 3,
}

/// Emitted with the aggregate produced by a forced recomputation (#483).
///
/// Topics: `asset`, `reason`
#[contractevent]
#[derive(Clone)]
pub struct AggregateRecomputedEvent {
    /// Asset whose aggregate was re-derived.
    #[topic]
    pub asset: Address,
    /// Why the recomputation ran.
    #[topic]
    pub reason: RecomputeReason,
    /// Recomputed aggregate price (`0` when the asset lost quorum).
    pub price: i128,
    /// Sources contributing to the recomputed aggregate.
    pub num_sources: u32,
    /// Contributing-source count before the change.
    pub previous_num_sources: u32,
    /// Ledger at which the recomputation ran.
    pub ledger: u32,
}

/// Whether `source` is ineligible to contribute because it is marked inactive
/// or disqualified by the demerit system.
pub fn is_excluded(env: &Env, source: &Address) -> bool {
    is_source_inactive(env, source) || crate::sources::is_source_suspended(env, source.clone())
}

/// Records that a source has become ineligible to contribute.
///
/// The flag is sticky: sources are reinstated far less often than they are
/// excluded, and over-reporting only means the (more expensive) exact check
/// runs during aggregation. The aggregation loop reads the guard flags once
/// and skips the per-source checks entirely when no source has ever been
/// excluded, so the common case costs no per-source storage reads at all.
pub fn note_excluded_source(env: &Env) {
    crate::price_bounds::note_source_excluded(env);
}

/// Assets a source could have contributed to, bounded by
/// [`MAX_RECOMPUTE_ASSETS`] so a removal transaction cannot be unbounded.
pub fn affected_assets(env: &Env, source: &Address) -> Vec<Address> {
    let mut out: Vec<Address> = Vec::new(env);

    if let Some(list) = env
        .storage()
        .persistent()
        .get::<DataKey, Vec<Address>>(&DataKey::SourceAssets(source.clone()))
    {
        for a in list.iter() {
            if !out.contains(&a) {
                out.push_back(a);
            }
        }
    }

    for a in crate::storage::read_registered_assets(env).iter() {
        if out.len() >= MAX_RECOMPUTE_ASSETS {
            break;
        }
        if out.contains(&a) {
            continue;
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::Submission(a.clone(), source.clone()))
        {
            out.push_back(a);
        }
    }
    out
}

fn stamp(env: &Env) {
    let key = DataKey::LastForcedRecompute;
    let ledger = env.ledger().sequence();
    env.storage().persistent().set(&key, &ledger);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn recompute_one(env: &Env, asset: &Address, reason: RecomputeReason, ledger: u32) {
    let key = DataKey::Aggregate(asset.clone());
    // Nothing was ever published: there is nothing to recompute.
    let Some(previous) = env
        .storage()
        .persistent()
        .get::<DataKey, AggregatePrice>(&key)
    else {
        return;
    };
    let (price, num_sources) = recompute_asset(env, asset);
    AggregateRecomputedEvent {
        asset: asset.clone(),
        reason,
        price,
        num_sources,
        previous_num_sources: previous.num_sources,
        ledger,
    }
    .publish(env);
}

/// Re-derives the aggregate of every asset `source` could affect and emits the
/// result. Called by the source-set mutation paths *after* the registry was
/// updated, so the removed source is already gone.
pub fn recompute_source_change(env: &Env, source: &Address, reason: RecomputeReason) {
    let ledger = env.ledger().sequence();
    stamp(env);
    for asset in affected_assets(env, source).iter() {
        recompute_one(env, &asset, reason, ledger);
    }
}

/// Re-derives the aggregate of an explicit asset list. Used when the caller
/// already knows which asset changed — e.g. `remove_source_asset`, where the
/// source's submission for that asset is already gone and can no longer be
/// discovered by scanning.
pub fn recompute_assets(env: &Env, assets: &Vec<Address>, reason: RecomputeReason) {
    let ledger = env.ledger().sequence();
    stamp(env);
    for (i, asset) in assets.iter().enumerate() {
        if i as u32 >= MAX_RECOMPUTE_ASSETS {
            break;
        }
        recompute_one(env, &asset, reason, ledger);
    }
}

/// Recomputes every registered asset, not just those a single source touched.
///
/// Used by batch removal so a multi-source change converges on the same
/// aggregate as the equivalent sequence of single removals.
pub fn recompute_all(env: &Env, reason: RecomputeReason) {
    let ledger = env.ledger().sequence();
    stamp(env);
    let list: Vec<Address> = crate::storage::read_registered_assets(env);
    for (i, asset) in list.iter().enumerate() {
        if i as u32 >= MAX_RECOMPUTE_ASSETS {
            break;
        }
        recompute_one(env, &asset, reason, ledger);
    }
}
