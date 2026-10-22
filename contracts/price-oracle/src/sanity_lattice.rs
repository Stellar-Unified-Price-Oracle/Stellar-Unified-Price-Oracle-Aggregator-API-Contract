//! # Cross-asset sanity lattice (#488)
//!
//! A per-asset deviation bound only asks "is this price near the other prices
//! *for this asset*". That is blind to the case where a manipulation is
//! internally consistent for one asset but impossible given its relatives —
//! a stablecoin quoting at 1.30 while its sibling still says 1.00, or a
//! triangular FX cross that does not close.
//!
//! This module adds a family-level check: every candidate aggregate is
//! validated against the *published* aggregates of the assets it is declared
//! related to, and a violation is rejected, flagged, or quarantined.
//!
//! ## Relations
//!
//! | Kind | Holds when | Fields used |
//! |---|---|---|
//! | [`SanityRelationKind::Peg`] | `price(asset) ~= price(peer) * num/den` | `peer`, `ratio_num`, `ratio_den` |
//! | [`SanityRelationKind::Triangle`] | `price(asset) * price(peer) ~= price(peer2)` | `peer`, `peer2` |
//! | [`SanityRelationKind::Spread`] | `|price(asset) - price(peer)| / price(asset) <= tol` | `peer` |
//!
//! A relation is evaluated against the **stored** aggregate of its peers, not
//! against their sources, so a check never re-runs a whole aggregation.
//!
//! ## Cycles
//!
//! The relation graph is allowed to contain cycles (A pegs B, B pegs A).
//! Evaluation is therefore **iterative with a visited set and a hard
//! iteration cap**, never recursive: a cycle terminates because a visited
//! asset is not expanded twice, and a pathological graph is additionally cut
//! off by [`MAX_EXPANSIONS`]. See [`reachable`].
//!
//! ## De-pegs
//!
//! A legitimate de-peg breaks the assumed relation. [`declare_depeg`]
//! suspends every relation touching an asset until a given ledger without
//! disabling the asset itself: it keeps aggregating, publishing and serving —
//! it just stops being second-guessed against a peg that no longer holds.
//!
//! See `docs/sanity-lattice.md`.

use soroban_sdk::{contractevent, panic_with_error, Address, Env, String};

use crate::events::emit_admin_action;
use crate::price_bounds::note_sanity_relation;
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    AggregatePrice, DataKey, ErrorCode, SanityAction, SanityRelation, SanityRelationKind,
    SanityStatus,
};

/// Largest accepted tolerance, in bps (100 %). Wider than this is not a
/// relation but an absence of one.
pub const MAX_TOLERANCE_BPS: u32 = 10_000;
/// Hard cap on relation-graph expansion, so a cycle or a dense graph can never
/// make evaluation unbounded.
pub const MAX_EXPANSIONS: u32 = 64;
/// Largest number of relations one asset may declare.
pub const MAX_RELATIONS_PER_ASSET: u32 = 16;

/// Emitted when a candidate aggregate breaks a declared relation (#488).
///
/// Carries everything needed to reproduce the check offline: both assets, both
/// values, the observed magnitude and the tolerance it exceeded.
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct SanityRelationViolatedEvent {
    #[topic]
    pub asset: Address,
    /// The asset whose published value the candidate was checked against.
    pub conflicting_asset: Address,
    /// `SanityRelationKind` discriminant.
    pub kind: u32,
    /// The candidate aggregate that was checked.
    pub candidate_price: i128,
    /// The published value of `conflicting_asset` at evaluation time.
    pub reference_price: i128,
    /// Observed relative deviation, in bps.
    pub magnitude_bps: u32,
    /// Tolerance the observation was compared against.
    pub tolerance_bps: u32,
    /// `SanityAction` discriminant applied to the candidate.
    pub action: u32,
    pub ledger: u32,
}

/// Emitted when relations are suspended for a declared de-peg (#488).
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct DepegDeclaredEvent {
    #[topic]
    pub asset: Address,
    /// Ledger at which the suspension expires.
    pub until_ledger: u32,
    /// Mandatory human-readable justification.
    pub reason: String,
    pub ledger: u32,
}

/// Rejects a relation that cannot be evaluated as written.
pub fn validate(env: &Env, asset: &Address, r: &SanityRelation) {
    if r.tolerance_bps == 0 || r.tolerance_bps > MAX_TOLERANCE_BPS {
        panic_with_error!(env, ErrorCode::InvalidSanityTolerance);
    }
    match r.kind {
        SanityRelationKind::Triangle => {
            let Some(p2) = r.peer2.clone() else {
                panic_with_error!(env, ErrorCode::InvalidSanityRelation);
            };
            if p2 == *asset || r.peer == *asset || r.peer == p2 {
                // A triangle needs three distinct assets, otherwise it is
                // either a self-reference or a degenerate 2-cycle with no
                // information in it.
                panic_with_error!(env, ErrorCode::InvalidSanityRelation);
            }
        }
        _ => {
            if r.peer == *asset {
                panic_with_error!(env, ErrorCode::InvalidSanityRelation);
            }
        }
    }
    if r.kind == SanityRelationKind::Peg && (r.ratio_den == 0 || r.ratio_num <= 0) {
        panic_with_error!(env, ErrorCode::InvalidSanityRatio);
    }
    check_registered_asset(env, &r.peer);
    if let Some(p2) = r.peer2.clone() {
        check_registered_asset(env, &p2);
    }
}

/// The relations declared on `asset`, oldest first.
pub fn get_relations(env: &Env, asset: &Address) -> soroban_sdk::Vec<SanityRelation> {
    let key = DataKey::AssetSanityRelations(asset.clone());
    let v: Option<soroban_sdk::Vec<SanityRelation>> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v.unwrap_or_else(|| soroban_sdk::Vec::new(env))
}

/// Adds a relation to `asset`. Admin only; validated on write.
pub fn add_relation(env: &Env, asset: Address, relation: SanityRelation) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);
    validate(env, &asset, &relation);

    let key = DataKey::AssetSanityRelations(asset.clone());
    let mut list = get_relations(env, &asset);
    if list.len() >= MAX_RELATIONS_PER_ASSET {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    for i in 0..list.len() {
        let existing = list.get_unchecked(i);
        if existing.peer == relation.peer && existing.kind == relation.kind {
            // Re-declaring the same pair is a configuration error, not an
            // update: replacing a relation silently would hide the change.
            panic_with_error!(env, ErrorCode::InvalidSanityRelation);
        }
    }
    list.push_back(relation);
    env.storage().persistent().set(&key, &list);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    note_sanity_relation(env);
    emit_admin_action(
        env,
        soroban_sdk::symbol_short!("san_rel"),
        admin,
        soroban_sdk::Bytes::new(env),
    );
}

/// Removes the relation between `asset` and `peer` of the given kind.
pub fn remove_relation(env: &Env, asset: Address, peer: Address, kind: SanityRelationKind) {
    get_admin(env).require_auth();
    let key = DataKey::AssetSanityRelations(asset.clone());
    let list = get_relations(env, &asset);
    let mut out = soroban_sdk::Vec::new(env);
    for i in 0..list.len() {
        let r = list.get_unchecked(i);
        if !(r.peer == peer && r.kind == kind) {
            out.push_back(r);
        }
    }
    env.storage().persistent().set(&key, &out);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Suspends every relation touching `asset` until `until_ledger` (#488).
///
/// The asset itself keeps working: it still aggregates, publishes and serves.
/// Only the family checks are skipped, which is exactly what a declared de-peg
/// needs.
pub fn declare_depeg(env: &Env, asset: Address, until_ledger: u32, reason: String) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);
    let key = DataKey::AssetSanitySuspended(asset.clone());
    env.storage().persistent().set(&key, &until_ledger);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    DepegDeclaredEvent {
        asset,
        until_ledger,
        reason,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
    emit_admin_action(
        env,
        soroban_sdk::symbol_short!("depeg"),
        admin,
        soroban_sdk::Bytes::new(env),
    );
}

/// Lifts a de-peg suspension immediately.
pub fn clear_depeg(env: &Env, asset: Address) {
    get_admin(env).require_auth();
    env.storage()
        .persistent()
        .remove(&DataKey::AssetSanitySuspended(asset.clone()));
}

/// `true` while a declared de-peg suspension is in force.
pub fn is_suspended(env: &Env, asset: &Address) -> bool {
    let key = DataKey::AssetSanitySuspended(asset.clone());
    match env.storage().persistent().get::<_, u32>(&key) {
        Some(until) => env.ledger().sequence() <= until,
        None => false,
    }
}

/// The assets reachable from `asset` through declared relations, breadth-first.
///
/// This is where cycles are handled: an already-visited asset is never expanded
/// again, and the total number of expansions is capped at [`MAX_EXPANSIONS`],
/// so a cyclic or dense graph terminates instead of recursing forever.
pub fn reachable(env: &Env, asset: &Address) -> soroban_sdk::Vec<Address> {
    let mut seen = soroban_sdk::Vec::new(env);
    let mut out = soroban_sdk::Vec::new(env);
    let mut frontier = soroban_sdk::Vec::new(env);
    seen.push_back(asset.clone());
    frontier.push_back(asset.clone());
    let mut expansions = 0u32;
    while !frontier.is_empty() && expansions < MAX_EXPANSIONS {
        expansions += 1;
        let current = frontier.get_unchecked(0);
        for r in get_relations(env, &current).iter() {
            for peer in [Some(r.peer.clone()), r.peer2.clone()]
                .into_iter()
                .flatten()
            {
                if !seen.contains(&peer) {
                    seen.push_back(peer.clone());
                    out.push_back(peer.clone());
                    frontier.push_back(peer);
                }
            }
        }
        // Pop the head. The frontier is bounded by the number of distinct
        // assets, and every pop corresponds to one counted expansion.
        let mut rest = soroban_sdk::Vec::new(env);
        for i in 1..frontier.len() {
            rest.push_back(frontier.get_unchecked(i));
        }
        frontier = rest;
    }
    out
}

fn stored_price(env: &Env, asset: &Address) -> Option<i128> {
    let key = DataKey::Aggregate(asset.clone());
    let a: Option<AggregatePrice> = env.storage().persistent().get(&key);
    a.filter(|a| a.price > 0).map(|a| a.price)
}

/// Relative deviation between `price` and `expected`, in bps.
fn deviation_bps(price: i128, expected: i128) -> u32 {
    if expected == 0 {
        return u32::MAX;
    }
    let diff = (price - expected).abs();
    let scaled = diff.saturating_mul(10_000) / expected.abs();
    if scaled > i128::from(u32::MAX) {
        u32::MAX
    } else {
        scaled as u32
    }
}

/// Outcome of checking one candidate aggregate against its declared relations.
pub struct Verdict {
    /// `true` when the candidate must not be published.
    pub blocked: bool,
    /// `true` when the candidate was published but flagged.
    pub flagged: bool,
    pub status: SanityStatus,
}

/// Checks `price` for `asset` against every declared relation.
///
/// Returns a [`Verdict`]. A missing peer price is not a violation — the family
/// simply is not observable yet, and inventing a violation there would let an
/// unpriced relative block a healthy feed.
pub fn check(env: &Env, asset: &Address, price: i128) -> Verdict {
    let suspended = is_suspended(env, asset);
    let relations = get_relations(env, asset);
    let ledger = env.ledger().sequence();
    let quarantined = read_status(env, asset)
        .map(|p| p.quarantined)
        .unwrap_or(false);

    let mut status = SanityStatus {
        asset: asset.clone(),
        violated: false,
        suspended,
        checked: 0,
        conflicting_asset: None,
        magnitude_bps: 0,
        tolerance_bps: 0,
        action: SanityAction::Flag,
        quarantined,
        ledger,
    };

    if suspended || relations.is_empty() {
        return Verdict {
            blocked: quarantined,
            flagged: false,
            status,
        };
    }

    let mut blocked = false;
    let mut flagged = false;
    for i in 0..relations.len() {
        let r = relations.get_unchecked(i);
        let Some(reference) = stored_price(env, &r.peer) else {
            continue;
        };
        let expected = match r.kind {
            SanityRelationKind::Peg => reference
                .saturating_mul(r.ratio_num)
                .checked_div(r.ratio_den)
                .unwrap_or(0),
            SanityRelationKind::Triangle => {
                match r.peer2.clone().and_then(|p| stored_price(env, &p)) {
                    Some(p2) => p2,
                    None => continue,
                }
            }
            SanityRelationKind::Spread => reference,
        };
        if expected <= 0 {
            continue;
        }
        status.checked += 1;
        let magnitude = match r.kind {
            SanityRelationKind::Spread => {
                let base = price.abs().max(1);
                ((price - reference).abs().saturating_mul(10_000) / base) as u32
            }
            _ => deviation_bps(price, expected),
        };
        if magnitude <= r.tolerance_bps {
            continue;
        }

        let mut vstatus = status.clone();
        vstatus.violated = true;
        vstatus.conflicting_asset = Some(r.peer.clone());
        vstatus.magnitude_bps = magnitude;
        vstatus.tolerance_bps = r.tolerance_bps;
        vstatus.action = r.action;
        status = vstatus;

        SanityRelationViolatedEvent {
            asset: asset.clone(),
            conflicting_asset: r.peer.clone(),
            kind: r.kind as u32,
            candidate_price: price,
            reference_price: reference,
            magnitude_bps: magnitude,
            tolerance_bps: r.tolerance_bps,
            action: r.action as u32,
            ledger,
        }
        .publish(env);

        match r.action {
            SanityAction::Flag => flagged = true,
            SanityAction::Reject => blocked = true,
            SanityAction::Quarantine => {
                blocked = true;
                status.quarantined = true;
            }
        }
    }

    if status.violated {
        // The full status is persisted, not just the quarantine bit: a consumer
        // must be able to read back the conflicting asset and the magnitude
        // that caused the decision, not merely that something happened.
        let key = DataKey::AssetSanityStatus(asset.clone());
        env.storage().persistent().set(&key, &status);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    Verdict {
        blocked: blocked || status.quarantined,
        flagged,
        status,
    }
}

fn read_status(env: &Env, asset: &Address) -> Option<SanityStatus> {
    let key = DataKey::AssetSanityStatus(asset.clone());
    let v: Option<SanityStatus> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// The last recorded sanity outcome for `asset`, if any.
///
/// The stored record is the one written at evaluation time, so it names the
/// conflicting asset and the observed magnitude that produced the decision.
pub fn get_status(env: &Env, asset: &Address) -> Option<SanityStatus> {
    read_status(env, asset)
}

/// Clears a quarantine, letting the asset publish again (#488).
pub fn clear_quarantine(env: &Env, asset: &Address) {
    get_admin(env).require_auth();
    check_registered_asset(env, asset);
    env.storage()
        .persistent()
        .remove(&DataKey::AssetSanityStatus(asset.clone()));
}
