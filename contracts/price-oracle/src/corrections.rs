//! # Auditable price corrections (#486)
//!
//! Errors happen: a typo in a source adapter, a bad value discovered hours
//! later. Rather than leaving wrong data published or mutating history
//! opaquely, an authorized operator may file a *correction* that carries a
//! mandatory reason and appends to an immutable revision chain.
//!
//! ## Guarantees
//!
//! * **Evidence is never destroyed.** Revision `0` is the original publication
//!   and is copied to [`DataKey::PriceOriginal`]; corrections only ever append.
//!   `get_original_price` and `get_price_revisions` stay queryable forever.
//! * **Authority is scoped, not general.** `correct_price` requires the admin
//!   (or a `PriceUpdater` delegate) and is additionally bounded on three axes:
//!   * *asset scope* — [`set_correction_scope`] limits which assets a
//!     correction may target (`None` = all assets),
//!   * *time scope* — the published aggregate must be younger than
//!     `window_ledgers` ([`MAX_CORRECTION_WINDOW_LEDGERS`]),
//!   * *count* — at most `max_corrections` corrections per asset
//!     ([`MAX_CORRECTIONS_PER_ASSET`]).
//! * **Every correction is evented** with actor, reason, old and new value in
//!   [`PriceCorrectedEvent`], and appended to the admin audit trail.
//!
//! A correction is a *republication*, not a history rewrite: already-published
//! history entries are untouched, and consumers that cached the old value can
//! detect the change through the aggregate `version`.

use soroban_sdk::{contractevent, contracttype, panic_with_error, Address, Env, String, Vec};

use crate::events::emit_admin_action;
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregatePrice, DataKey, ErrorCode, PriceRevision};

/// Longest correction window, in ledgers (≈ 1 day at 5 s blocks).
pub const MAX_CORRECTION_WINDOW_LEDGERS: u32 = 17_280;
/// Upper bound on corrections per asset.
pub const MAX_CORRECTIONS_PER_ASSET: u32 = 32;
/// Maximum accepted correction-reason length.
pub const MAX_REASON_LEN: u32 = 256;

/// Emitted for every applied correction (#486).
///
/// Topics: `asset`, `actor`
#[contractevent]
#[derive(Clone)]
pub struct PriceCorrectedEvent {
    #[topic]
    pub asset: Address,
    /// Address that filed the correction.
    #[topic]
    pub actor: Address,
    /// Mandatory human-readable reason.
    pub reason: String,
    /// Value published before the correction.
    pub old_price: i128,
    /// Value published by the correction.
    pub new_price: i128,
    /// Index of the new revision in the chain.
    pub revision_index: u32,
    /// Ledger the correction was applied at.
    pub ledger: u32,
    /// True when the corrected value had already been consumed downstream
    /// (i.e. it was live for more than one ledger before being corrected).
    pub affects_downstream: bool,
}

/// Per-asset correction limits.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CorrectionScope {
    /// Assets a correction may target. `None` means "any registered asset".
    pub assets: Option<Vec<Address>>,
    /// The published aggregate must be younger than this many ledgers.
    pub window_ledgers: u32,
    /// Maximum number of corrections the asset may accumulate.
    pub max_corrections: u32,
}

/// The correction limits currently in force.
pub fn get_correction_scope(env: &Env) -> CorrectionScope {
    env.storage()
        .persistent()
        .get(&DataKey::CfgCorrectionScope)
        .unwrap_or_else(default_scope)
}

fn default_scope() -> CorrectionScope {
    CorrectionScope {
        assets: None,
        window_ledgers: MAX_CORRECTION_WINDOW_LEDGERS,
        max_corrections: MAX_CORRECTIONS_PER_ASSET,
    }
}

/// Tightens or widens the correction limits. Admin only; values are clamped to
/// the documented hard maxima so the scope can never become unbounded.
pub fn set_correction_scope(env: &Env, scope: CorrectionScope) {
    get_admin(env).require_auth();
    if scope.window_ledgers == 0 || scope.max_corrections == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let bounded = CorrectionScope {
        assets: scope.assets.clone(),
        window_ledgers: scope.window_ledgers.min(MAX_CORRECTION_WINDOW_LEDGERS),
        max_corrections: scope.max_corrections.min(MAX_CORRECTIONS_PER_ASSET),
    };
    for a in bounded.assets.clone().unwrap_or(Vec::new(env)).iter() {
        check_registered_asset(env, &a);
    }
    let key = DataKey::CfgCorrectionScope;
    env.storage().persistent().set(&key, &bounded);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    crate::price_bounds::note_correction_scope_configured(env);
}

/// The full revision chain for `asset`, oldest first.
pub fn get_revisions(env: &Env, asset: &Address) -> Vec<PriceRevision> {
    let key = DataKey::PriceRevisions(asset.clone());
    let v: Vec<PriceRevision> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    if !v.is_empty() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Revision at `index`, or `None` when the chain is shorter.
pub fn get_revision(env: &Env, asset: &Address, index: u32) -> Option<PriceRevision> {
    let chain = get_revisions(env, asset);
    if index >= chain.len() {
        return None;
    }
    Some(chain.get_unchecked(index))
}

/// The first-ever published value. Never overwritten by a correction.
pub fn get_original_price(env: &Env, asset: &Address) -> Option<i128> {
    let key = DataKey::PriceOriginal(asset.clone());
    let v: Option<i128> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Number of corrections applied to `asset` so far.
pub fn correction_count(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::PriceCorrectionCount(asset.clone()))
        .unwrap_or(0)
}

/// Records the *original* publication as revision 0, once.
///
/// Called from the aggregation path; subsequent calls are no-ops, so the
/// original value is established at first publication and preserved forever.
pub fn record_original(env: &Env, asset: &Address, aggregate: &AggregatePrice) {
    let key = DataKey::PriceRevisions(asset.clone());
    if env
        .storage()
        .persistent()
        .get::<DataKey, Vec<PriceRevision>>(&key)
        .is_some()
    {
        return;
    }
    let original = DataKey::PriceOriginal(asset.clone());
    env.storage().persistent().set(&original, &aggregate.price);
    env.storage()
        .persistent()
        .extend_ttl(&original, LEDGER_THRESHOLD, LEDGER_BUMP);
    append_revision(
        env,
        asset,
        PriceRevision {
            index: 0,
            price: aggregate.price,
            timestamp: aggregate.timestamp,
            ledger: env.ledger().sequence(),
            actor: env.current_contract_address(),
            reason: String::from_str(env, "original"),
            corrected: false,
        },
    );
}

fn append_revision(env: &Env, asset: &Address, revision: PriceRevision) {
    let key = DataKey::PriceRevisions(asset.clone());
    let mut chain: Vec<PriceRevision> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    chain.push_back(revision);
    env.storage().persistent().set(&key, &chain);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn history_ledger(env: &Env, asset: &Address) -> u32 {
    let key = DataKey::PriceHistoryLedgers(asset.clone());
    let list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    if list.is_empty() {
        return 0;
    }
    list.get_unchecked(list.len() - 1)
}

/// Corrects the published aggregate for `asset` to `new_price` and returns the
/// index of the new revision.
///
/// Requires admin authority (or a `PriceUpdater` delegate) plus a non-empty
/// `reason` of at most [`MAX_REASON_LEN`] characters. The correction is
/// rejected when the asset is out of correction scope, the published aggregate
/// is older than the correction window, the per-asset correction cap is
/// reached, or `new_price` violates the asset's hard bounds (#484) — a
/// correction is not a way around the hard tier.
///
/// On success the aggregate is republished with a bumped `version` and
/// `is_override = true`, a revision is appended to the chain, and
/// [`PriceCorrectedEvent`] plus an admin audit entry are emitted. The
/// original publication is untouched.
pub fn correct_price(env: &Env, asset: Address, new_price: i128, reason: String) -> u32 {
    let admin = get_admin(env);
    if !crate::rbac::has_role(env, &admin, crate::types::Role::PriceUpdater) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
    admin.require_auth();
    check_registered_asset(env, &asset);

    if new_price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }
    if reason.is_empty() || reason.len() > MAX_REASON_LEN {
        panic_with_error!(env, ErrorCode::InvalidCorrectionReason);
    }

    if let Some(tier) = crate::price_bounds::get_bounds(env, &asset) {
        if new_price < tier.hard_min || new_price > tier.hard_max {
            panic_with_error!(env, ErrorCode::AggregateRejectedByBounds);
        }
    }

    let scope = get_correction_scope(env);
    if let Some(assets) = scope.assets.clone() {
        if !assets.contains(&asset) {
            panic_with_error!(env, ErrorCode::NotAuthorized);
        }
    }
    if correction_count(env, &asset) >= scope.max_corrections {
        panic_with_error!(env, ErrorCode::CorrectionLimitReached);
    }

    let agg_key = DataKey::Aggregate(asset.clone());
    let previous: AggregatePrice = match env.storage().persistent().get(&agg_key) {
        Some(a) => a,
        None => panic_with_error!(env, ErrorCode::NoData),
    };

    let ledger = env.ledger().sequence();
    let published_ledger = history_ledger(env, &asset);
    if ledger.saturating_sub(published_ledger) > scope.window_ledgers {
        panic_with_error!(env, ErrorCode::CorrectionWindowExpired);
    }

    // If no correction scope was ever configured, the aggregation path did not
    // record revision 0, so seed it here from the value that is live *now* —
    // i.e. the erroneous value the correction is about to replace. From this
    // point on the chain is immutable and every later value is appended.
    if get_revisions(env, &asset).is_empty() {
        record_original(env, &asset, &previous);
    }

    let index = get_revisions(env, &asset).len();
    let corrected = AggregatePrice {
        price: new_price,
        timestamp: env.ledger().timestamp(),
        num_sources: previous.num_sources,
        decimals: previous.decimals,
        is_override: true,
        version: previous.version.saturating_add(1),
    };
    env.storage().persistent().set(&agg_key, &corrected);
    env.storage()
        .persistent()
        .extend_ttl(&agg_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let count_key = DataKey::PriceCorrectionCount(asset.clone());
    let count = correction_count(env, &asset).saturating_add(1);
    env.storage().persistent().set(&count_key, &count);
    env.storage()
        .persistent()
        .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    append_revision(
        env,
        &asset,
        PriceRevision {
            index,
            price: new_price,
            timestamp: corrected.timestamp,
            ledger,
            actor: admin.clone(),
            reason: reason.clone(),
            corrected: true,
        },
    );

    PriceCorrectedEvent {
        asset: asset.clone(),
        actor: admin.clone(),
        reason: reason.clone(),
        old_price: previous.price,
        new_price,
        revision_index: index,
        ledger,
        // A value that was live for more than a single ledger may already have
        // been consumed by downstream contracts; flag it explicitly.
        affects_downstream: ledger > published_ledger.saturating_add(1),
    }
    .publish(env);
    emit_admin_action(
        env,
        soroban_sdk::symbol_short!("corr_px"),
        admin,
        soroban_sdk::Bytes::from_slice(env, &new_price.to_be_bytes()),
    );

    index
}
