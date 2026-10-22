//! # Hysteresis-based circuit breaker with automatic re-arming (#481)
//!
//! See `docs/breaker-hysteresis.md`.
//!
//! ## The problem
//!
//! The pre-existing deviation breaker had one threshold. A price oscillating
//! either side of it therefore trips and clears on alternate ledgers, which is
//! indistinguishable from a broken feed: operators cannot tell a flapping
//! indicator from a genuinely unstable market, and the manual clearing it
//! demands does not scale to a portfolio of assets.
//!
//! ## The deadband
//!
//! Two thresholds, not one:
//!
//! * **trip** at `deviation >= trip_bps` — the breaker opens.
//! * **settled** at `deviation < clear_bps`, with `clear_bps < trip_bps` —
//!   the market is calm again.
//!
//! A deviation between the two is neither: the breaker holds whatever state it
//! is in. That band is the deadband, and it is what stops chatter. A price that
//! oscillates across the trip line stays inside the band once the breaker is
//! open, so it cannot re-trip, and a price that settles just below the trip
//! threshold but above the clear threshold does not re-arm either.
//!
//! `clear_bps` must be **strictly** below `trip_bps`; equal thresholds would
//! reproduce the chattering breaker this replaces.
//!
//! ## Re-arm requires a settle condition
//!
//! A single below-clear-threshold observation is not enough. The deviation must
//! stay below `clear_bps` for `settle_ledgers` *consecutive* ledgers. A
//! one-sample dip into calmness therefore cannot re-arm the breaker, so an
//! adversary cannot flap a price down for one ledger, let the breaker re-arm,
//! and push it back up.
//!
//! The settle streak resets to zero on any observation at or above `clear_bps`.
//!
//! ## Recovery verification signals (#623)
//!
//! A settle streak alone proves the *price* is calm; it does not prove the
//! *source* has recovered. Before the automatic path re-arms, two independent
//! pieces of evidence must both be present:
//!
//! * **Source recovery** — the feed that tripped the breaker must have reported
//!   a fresh, healthy observation. [`record_source_recovery`] records this; a
//!   trip clears it, so a stale recovery cannot survive a new trip.
//! * **Stable price observations** — the settle streak must have reached
//!   `settle_ledgers` consecutive calm ledgers (see above).
//!
//! [`evaluate_rearm`] re-arms only when **all** configured conditions hold.
//! Partial satisfaction keeps the breaker tripped and is reported through
//! [`BreakerRearmAttemptEvent`] so operators can see exactly which evidence is
//! still missing. [`get_breaker_status`] exposes the same signals for
//! inspection.
//!
//! ## Manual override is never blocked
//!
//! [`clear_breaker`] is authorised by the admin and ignores the settle streak
//! and the escalation flag entirely. The automatic path is a convenience for
//! the common case; it is never a gate on the operator. Automatic re-arm being
//! disabled (`auto_rearm = false`) or having escalated does not prevent a
//! manual clear — it only means the contract stops trying on its own.
//!
//! ## Bounded open time
//!
//! A breaker that can never re-arm is just a permanent pause with extra steps,
//! and one that can always re-arm is not a control. `max_open_ledgers` bounds
//! the automatic path: past it the contract emits [`BreakerEscalatedEvent`],
//! stops attempting automatic re-arm, and leaves the decision to the operator.
//! Escalation is a *narrowing* of automation, never a widening: it can only
//! ever reduce what the contract does on its own.
//!
//! ## Reconstructibility
//!
//! Every transition is evented with the deviation that drove it:
//! [`BreakerTrippedEvent`], [`BreakerRearmAttemptEvent`] (emitted on **both**
//! outcomes of an evaluation), [`BreakerRearmedEvent`] and
//! [`BreakerEscalatedEvent`]. Because the attempt event fires whether or not
//! it re-arms, a consumer replaying the stream can distinguish "evaluated and
//! held" from "never evaluated" — silence is not ambiguous.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::assets::is_circuit_breaker_tripped;
use crate::events::{
    BreakerEscalatedEvent, BreakerRearmAttemptEvent, BreakerRearmedEvent, BreakerTrippedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{BreakerPolicy, BreakerStatus, DataKey, ErrorCode};

/// Returns an asset's breaker policy (default: [`BreakerPolicy::default_policy`]).
pub fn get_breaker_policy(env: &Env, asset: &Address) -> BreakerPolicy {
    let key = DataKey::BreakerPolicy(asset.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(BreakerPolicy::default_policy)
}

/// Configures an asset's breaker. Admin only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidBreakerDeadband`] — `clear_bps >= trip_bps`. Equal
///   thresholds would leave no deadband and reproduce the chattering breaker
///   this replaces.
/// * [`ErrorCode::InvalidBreakerPolicy`] — `settle_ledgers` or
///   `max_open_ledgers` is `0`, which would make the settle condition or the
///   escalation bound vacuous.
pub fn set_breaker_policy(env: &Env, asset: Address, policy: BreakerPolicy) {
    get_admin(env).require_auth();
    validate_policy(env, &policy);

    let key = DataKey::BreakerPolicy(asset.clone());
    env.storage().persistent().set(&key, &policy);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Shared policy validation, so the setter and any future path agree.
fn validate_policy(env: &Env, policy: &BreakerPolicy) {
    if policy.clear_bps >= policy.trip_bps {
        panic_with_error!(env, ErrorCode::InvalidBreakerDeadband);
    }
    if policy.settle_ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidBreakerPolicy);
    }
    if policy.max_open_ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidBreakerPolicy);
    }
}

/// Returns the ledger at which the current settle streak began.
fn settle_streak_start(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::BreakerSettleStreak(asset.clone()))
        .unwrap_or(0)
}

/// Returns the ledger at which the breaker last opened.
fn armed_ledger(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::BreakerArmedLedger(asset.clone()))
        .unwrap_or(0)
}

fn escalations(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::BreakerEscalations(asset.clone()))
        .unwrap_or(0)
}

/// Returns the ledger at which source recovery was last recorded, or `0` if
/// no recovery evidence is present for the current trip.
fn source_recovery_ledger(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::BreakerSourceRecovery(asset.clone()))
        .unwrap_or(0)
}

/// Returns `true` when the source has reported a fresh, healthy observation
/// since the breaker last tripped.
///
/// This is the *source recovery* evidence required before automatic re-arm.
/// A trip clears it, so recovery recorded before a trip cannot satisfy the
/// condition for the new trip.
pub fn has_source_recovery(env: &Env, asset: &Address) -> bool {
    source_recovery_ledger(env, asset) > armed_ledger(env, asset)
}

/// Records that the feed for `asset` has recovered (reported a fresh, healthy
/// observation).
///
/// Call this from the ingestion path once a source is confirmed healthy again.
/// It is the source-recovery half of the re-arm evidence; the settle streak is
/// the price-stability half. Recording recovery does not itself re-arm the
/// breaker — [`evaluate_rearm`] still requires every condition to hold.
pub fn record_source_recovery(env: &Env, asset: &Address) {
    let now = env.ledger().sequence();
    let key = DataKey::BreakerSourceRecovery(asset.clone());
    env.storage().persistent().set(&key, &now);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns an asset's full breaker state.
pub fn get_breaker_status(env: &Env, asset: &Address) -> BreakerStatus {
    let policy = get_breaker_policy(env, asset);
    let is_open = is_circuit_breaker_tripped(env, asset);
    let now = env.ledger().sequence();
    let armed = armed_ledger(env, asset);
    let open_ledgers = if is_open {
        now.saturating_sub(armed)
    } else {
        0
    };
    let max_open_ledgers = policy.max_open_ledgers;
    BreakerStatus {
        is_open,
        policy,
        last_deviation_bps: 0,
        settle_ledgers: if is_open {
            now.saturating_sub(settle_streak_start(env, asset))
        } else {
            0
        },
        open_ledgers,
        escalated: is_open && open_ledgers > max_open_ledgers,
    }
}

/// Records a trip against `asset` and resets the settle streak.
///
/// Called from the aggregation path when a candidate price crosses the trip
/// threshold. The event carries the triggering deviation so a consumer can see
/// *why* the breaker opened rather than only that it did.
///
/// A trip also clears any recorded source-recovery evidence: the source that
/// just failed cannot be considered recovered for the new trip.
pub fn record_trip(env: &Env, asset: &Address, deviation_bps: u32) {
    let policy = get_breaker_policy(env, asset);
    let now = env.ledger().sequence();

    env.storage()
        .persistent()
        .set(&DataKey::BreakerArmedLedger(asset.clone()), &now);
    // A trip invalidates any partial settle streak: the market is not calm.
    env.storage()
        .persistent()
        .set(&DataKey::BreakerSettleStreak(asset.clone()), &now);
    // A trip invalidates prior source-recovery evidence: the source just
    // failed, so it must prove recovery again before the breaker can re-arm.
    env.storage()
        .persistent()
        .set(&DataKey::BreakerSourceRecovery(asset.clone()), &0u32);

    BreakerTrippedEvent {
        asset: asset.clone(),
        deviation_bps,
        trip_bps: policy.trip_bps,
        clear_bps: policy.clear_bps,
        ledger: now,
    }
    .publish(env);
}

/// Evaluates the re-arm condition for an open breaker and re-arms it if the
/// market has settled.
///
/// Call this once per ledger while the breaker is open — for example from
/// `trigger_aggregation`. It is a no-op when the breaker is already armed.
///
/// The settle condition is `settle_ledgers` **consecutive** ledgers at a
/// deviation strictly below `clear_bps`. Any ledger at or above `clear_bps`
/// resets the streak. In addition, the source must have recorded recovery
/// (see [`record_source_recovery`]) since the breaker tripped. Automatic
/// re-arm is impossible until **all** of these conditions hold; partial
/// satisfaction keeps the breaker tripped.
///
/// Returns `true` when the breaker was re-armed by this call.
pub fn evaluate_rearm(env: &Env, asset: &Address, deviation_bps: u32) -> bool {
    if !is_circuit_breaker_tripped(env, asset) {
        return false;
    }

    let policy = get_breaker_policy(env, asset);
    let now = env.ledger().sequence();

    // Track the settle streak: a calm observation extends it, anything else
    // resets it to the current ledger.
    if deviation_bps < policy.clear_bps {
        let start = settle_streak_start(env, asset);
        if start == 0 {
            env.storage()
                .persistent()
                .set(&DataKey::BreakerSettleStreak(asset.clone()), &now);
        }
    } else {
        env.storage()
            .persistent()
            .set(&DataKey::BreakerSettleStreak(asset.clone()), &now);
    }

    let streak = now.saturating_sub(settle_streak_start(env, asset));
    let price_stable = deviation_bps < policy.clear_bps && streak >= policy.settle_ledgers;
    let source_recovered = has_source_recovery(env, asset);

    let open_ledgers = now.saturating_sub(armed_ledger(env, asset));
    let escalated = open_ledgers > policy.max_open_ledgers;

    // Automatic re-arm requires every configured condition to hold. Partial
    // satisfaction keeps the breaker tripped.
    let can_rearm = policy.auto_rearm && !escalated && price_stable && source_recovered;

    BreakerRearmAttemptEvent {
        asset: asset.clone(),
        deviation_bps,
        settle_ledgers: streak,
        source_recovered,
        rearmed: can_rearm,
        ledger: now,
    }
    .publish(env);

    if escalated && escalations(env, asset) == 0 {
        env.storage()
            .persistent()
            .set(&DataKey::BreakerEscalations(asset.clone()), &1u32);
        BreakerEscalatedEvent {
            asset: asset.clone(),
            open_ledgers,
            max_open_ledgers: policy.max_open_ledgers,
            ledger: now,
        }
        .publish(env);
    }

    if can_rearm {
        crate::assets::set_circuit_breaker(env, asset, false);
        env.storage()
            .persistent()
            .set(&DataKey::BreakerSettleStreak(asset.clone()), &0u32);
        env.storage()
            .persistent()
            .set(&DataKey::BreakerSourceRecovery(asset.clone()), &0u32);
        BreakerRearmedEvent {
            asset: asset.clone(),
            deviation_bps,
            ledger: now,
        }
        .publish(env);
        return true;
    }

    false
}

/// Manually clears an open breaker. Admin only.
///
/// Ignores the settle streak, source-recovery evidence and the escalation flag
/// entirely: the operator is never gated by the automatic path.
pub fn clear_breaker(env: &Env, asset: &Address) {
    get_admin(env).require_auth();
    crate::assets::set_circuit_breaker(env, asset, false);
    env.storage()
        .persistent()
        .set(&DataKey::BreakerSettleStreak(asset.clone()), &0u32);
    env.storage()
        .persistent()
        .set(&DataKey::BreakerSourceRecovery(asset.clone()), &0u32);
}
