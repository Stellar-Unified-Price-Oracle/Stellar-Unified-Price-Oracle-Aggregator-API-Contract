//! # Deferred (quorum-within-window) aggregation (#485)
//!
//! For illiquid assets, publishing after a single report is more dangerous
//! than waiting. An asset may opt into a [`DeferralPolicy`]: publication is
//! withheld until `quorum` sources submit inside `window_secs`, and if that
//! never happens the asset escalates to [`PublicationState::Stale`] after
//! `max_defer_secs` rather than being starved silently forever.
//!
//! ## States
//!
//! ```text
//! Absent ──first submission──> Deferred ──quorum reached──> Published
//!                                  │                            │
//!                                  └──max_defer_secs elapsed───> Stale
//! ```
//!
//! All four states are distinguishable purely from the [`PublicationStatus`]
//! query, so a consumer never has to infer "waiting" from an old timestamp.
//! The `missing` field says exactly how many more submissions are needed.
//!
//! Assets without a policy keep the pre-#485 trigger behaviour (publish as
//! soon as the global `min_sources` quorum is met) — this module only changes
//! the publication policy for assets that explicitly opt in.
//!
//! Bounds (validated on write): `1 <= quorum <= 64`,
//! `1 <= window_secs <= 86_400`, `window_secs <= max_defer_secs <= 604_800`.

use soroban_sdk::{contractevent, panic_with_error, Address, Env};

use crate::price_bounds::note_deferral_configured;
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    DataKey, DeferralPolicy, ErrorCode, PriceEntry, PublicationGuards, PublicationState,
    PublicationStatus,
};

/// Largest accepted quorum.
pub const MAX_QUORUM: u32 = 64;
/// Largest accepted submission window (1 day).
pub const MAX_WINDOW_SECS: u64 = 86_400;
/// Largest accepted deferral bound (1 week).
pub const MAX_DEFER_SECS: u64 = 604_800;

/// Emitted whenever a deferrable asset changes publication state (#485).
///
/// Topics: `asset`, `state`
#[contractevent]
#[derive(Clone)]
pub struct PublicationStateChangedEvent {
    #[topic]
    pub asset: Address,
    /// The state entered.
    #[topic]
    pub state: PublicationState,
    /// The state left.
    pub previous_state: PublicationState,
    /// Submissions counted inside the window.
    pub received: u32,
    /// Submissions still required.
    pub missing: u32,
    /// Configured quorum.
    pub quorum: u32,
    /// Ledger of the transition.
    pub ledger: u32,
}

/// `1 <= quorum <= 64` and `window_secs <= max_defer_secs <= 604_800`.
pub fn validate(env: &Env, p: &DeferralPolicy) {
    if p.quorum == 0
        || p.quorum > MAX_QUORUM
        || p.window_secs == 0
        || p.window_secs > MAX_WINDOW_SECS
        || p.max_defer_secs < p.window_secs
        || p.max_defer_secs > MAX_DEFER_SECS
    {
        panic_with_error!(env, ErrorCode::InvalidDeferralPolicy);
    }
}

/// Sets the deferral policy for `asset`. Admin only; bounds are validated.
pub fn set_policy(env: &Env, asset: Address, policy: DeferralPolicy) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    validate(env, &policy);
    let key = DataKey::AssetDeferral(asset.clone());
    env.storage().persistent().set(&key, &policy);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    note_deferral_configured(env);
}

/// Returns the deferral policy for `asset`, if it opted in.
pub fn get_policy(env: &Env, asset: &Address) -> Option<DeferralPolicy> {
    let key = DataKey::AssetDeferral(asset.clone());
    let v: Option<DeferralPolicy> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

/// Removes the deferral policy, restoring the default publication trigger.
pub fn clear_policy(env: &Env, asset: Address) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    env.storage()
        .persistent()
        .remove(&DataKey::AssetDeferral(asset));
}

/// Whether the asset ever published an aggregate.
pub fn has_published(env: &Env, asset: &Address) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::Aggregate(asset.clone()))
}

/// Distinct sources with a submission inside the current window.
pub fn count_window_submissions(env: &Env, asset: &Address, policy: &DeferralPolicy) -> u32 {
    let now = env.ledger().timestamp();
    let sources: crate::types::OracleSources = crate::storage::read_oracle_sources(env);
    let mut n: u32 = 0;
    for s in sources.sources.iter() {
        let key = DataKey::Submission(asset.clone(), s);
        let Some(entry) = env.storage().persistent().get::<DataKey, PriceEntry>(&key) else {
            continue;
        };
        if now.saturating_sub(entry.ledger_timestamp) <= policy.window_secs {
            n += 1;
        }
    }
    n
}

fn read_status(env: &Env, asset: &Address) -> Option<PublicationStatus> {
    let key = DataKey::AssetPublicationState(asset.clone());
    let v: Option<PublicationStatus> = env.storage().persistent().get(&key);
    if v.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    v
}

fn write_status(env: &Env, asset: &Address, status: &PublicationStatus) {
    let key = DataKey::AssetPublicationState(asset.clone());
    env.storage().persistent().set(&key, status);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn initial_state(env: &Env, asset: &Address) -> PublicationState {
    if read_status(env, asset).is_some() || has_published(env, asset) {
        PublicationState::Published
    } else {
        PublicationState::Absent
    }
}

/// Resolves the lifecycle state from the window submission count.
///
/// Staleness wins over publication: once `max_defer_secs` has elapsed since
/// the deferral began the asset is `Stale` even if a late submission has since
/// completed quorum, so an operator sees the starvation rather than a
/// quietly-recovered feed.
fn resolve(
    env: &Env,
    asset: &Address,
    policy: &DeferralPolicy,
    received: u32,
) -> PublicationStatus {
    let now = env.ledger().timestamp();
    let previous = read_status(env, asset);
    let previous_state = previous
        .as_ref()
        .map(|p| p.state)
        .unwrap_or_else(|| initial_state(env, asset));
    let prev_deferred = previous.as_ref().map(|p| p.deferred_since).unwrap_or(0);

    let deferral_bound_hit = previous_state == PublicationState::Deferred
        && prev_deferred != 0
        && now.saturating_sub(prev_deferred) >= policy.max_defer_secs;

    // `Stale` is sticky: once a deferral has been escalated it stays escalated
    // until quorum is met again, so a starved feed cannot silently revert to
    // `Deferred` just because the clock moved on.
    let (state, deferred_since) = if received >= policy.quorum {
        (PublicationState::Published, 0)
    } else if deferral_bound_hit || previous_state == PublicationState::Stale {
        (
            PublicationState::Stale,
            if prev_deferred == 0 {
                now
            } else {
                prev_deferred
            },
        )
    } else if previous_state == PublicationState::Deferred {
        (PublicationState::Deferred, prev_deferred)
    } else {
        (PublicationState::Deferred, now)
    };

    PublicationStatus {
        state,
        received,
        missing: policy.quorum.saturating_sub(received),
        quorum: policy.quorum,
        window_secs: policy.window_secs,
        deferred_since,
        max_defer_secs: policy.max_defer_secs,
        ledger: env.ledger().sequence(),
    }
}

/// Current publication status for `asset`, or `None` when it never opted into
/// deferral. Resolving is pure — use [`note_state`] to persist and event it.
pub fn get_status(env: &Env, asset: &Address) -> Option<PublicationStatus> {
    let policy = get_policy(env, asset)?;
    let received = count_window_submissions(env, asset, &policy);
    Some(resolve(env, asset, &policy, received))
}

/// Whether the aggregation path may publish for `asset` right now.
///
/// Assets without a policy always publish (pre-#485 behaviour). A deferrable
/// asset publishes only once quorum is reached inside the window; a `Stale`
/// asset never publishes, so a starved feed cannot be revived by a single
/// late report.
pub fn should_publish(env: &Env, asset: &Address, guards: PublicationGuards) -> bool {
    // `guards` is the single publication-path read shared with #483/#484, so an
    // oracle that never opted into deferral skips the whole path for free.
    if !guards.any_deferral_configured {
        return true;
    }
    let Some(policy) = get_policy(env, asset) else {
        return true;
    };
    let received = count_window_submissions(env, asset, &policy);
    let status = resolve(env, asset, &policy, received);
    let previous_state = read_status(env, asset)
        .map(|p| p.state)
        .unwrap_or_else(|| initial_state(env, asset));
    if previous_state != status.state {
        PublicationStateChangedEvent {
            asset: asset.clone(),
            state: status.state,
            previous_state,
            received: status.received,
            missing: status.missing,
            quorum: status.quorum,
            ledger: status.ledger,
        }
        .publish(env);
    }
    write_status(env, asset, &status);
    status.state == PublicationState::Published
}

/// Recomputes and persists the state, emitting a transition event when it
/// changed. Safe to call on every submission.
pub fn note_state(env: &Env, asset: &Address) -> Option<PublicationStatus> {
    let policy = get_policy(env, asset)?;
    let previous = read_status(env, asset);
    let previous_state = previous
        .as_ref()
        .map(|p| p.state)
        .unwrap_or_else(|| initial_state(env, asset));
    let received = count_window_submissions(env, asset, &policy);
    let status = resolve(env, asset, &policy, received);
    if previous_state != status.state {
        PublicationStateChangedEvent {
            asset: asset.clone(),
            state: status.state,
            previous_state,
            received: status.received,
            missing: status.missing,
            quorum: status.quorum,
            ledger: status.ledger,
        }
        .publish(env);
    }
    write_status(env, asset, &status);
    Some(status)
}

/// Records that an aggregate was published for `asset`.
pub fn mark_published(env: &Env, asset: &Address) {
    let Some(policy) = get_policy(env, asset) else {
        return;
    };
    let mut status = resolve(env, asset, &policy, policy.quorum);
    status.state = PublicationState::Published;
    status.received = policy.quorum;
    status.missing = 0;
    status.deferred_since = 0;
    write_status(env, asset, &status);
}
