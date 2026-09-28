//! # Subscription auto-renewal (#289)
//!
//! See `docs/auto-renewal.md`.
//!
//! ## Period model
//!
//! A *period* is identified by its due timestamp. `AutoRenewRecord
//! .next_renewal_timestamp` is the timestamp at which the next period becomes
//! renewable, and `period_id == record.next_renewal_timestamp`. A renewal is
//! possible only once `env.ledger().timestamp() >= period_id`, and a successful
//! renewal advances `next_renewal_timestamp` by `plan_duration`.
//!
//! ## The bound this buys
//!
//! Because a period id is a strictly increasing timestamp and each successful
//! renewal advances it by exactly `plan_duration`, at most **one** renewal can
//! ever succeed per period id. Each of those renewals moves at most
//! `min(plan_amount, max_amount_per_period) <= max_amount_per_period`, and at
//! most `periods_authorized` of them may ever succeed. Therefore
//!
//! ```text
//! max_tokens_moved = periods_authorized * max_amount_per_period
//! ```
//!
//! and the contract can never move more than the consumer's own standing SAC
//! allowance either, because the live allowance is re-read on every attempt
//! (see `docs/auto-renewal.md` for the full proof).

use soroban_sdk::{panic_with_error, token, Address, Env};

use crate::events::{AutoRenewalAttemptEvent, AutoRenewalAuthorizationEvent};
use crate::reentrancy;
use crate::storage::{
    get_plan_amount, read_subscription_expiry, write_subscription_expiry, LEDGER_BUMP,
    LEDGER_THRESHOLD,
};
use crate::types::{AutoRenewRecord, DataKey, ErrorCode, RenewalAttempt, RenewalAuthorization};

/// Upper bound on `plan_duration` accepted by [`enable_auto_renewal`].
///
/// One year. A longer plan would let a single standing authorization stay live
/// long enough to outlive the consumer's own intent; one year is far beyond any
/// sane renewal horizon and keeps the "bounded standing right" property legible.
pub const MAX_PLAN_DURATION: u32 = 31_536_000;

/// Upper bound on `periods_authorized` accepted by [`enable_auto_renewal`].
///
/// 1 000 periods. Combined with `max_amount_per_period` this is the ceiling the
/// consumer themselves agreed to; the bound only keeps a single record's
/// head-room from being unbounded.
pub const MAX_PERIODS_AUTHORIZED: u32 = 1_000;

/// `AutoRenewalAuthorizationEvent.action` — authorization granted.
pub const ACTION_GRANTED: u32 = 0;
/// `AutoRenewalAuthorizationEvent.action` — authorization revoked.
pub const ACTION_REVOKED: u32 = 1;
/// `AutoRenewalAuthorizationEvent.action` — voided by a subscription cancel.
pub const ACTION_CANCELLED: u32 = 2;

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

fn record_key(consumer: &Address) -> DataKey {
    DataKey::SubscriptionAutoRenew(consumer.clone())
}

fn auth_key(consumer: &Address, period_id: u64) -> DataKey {
    DataKey::SubscriptionRenewalAuthorization(consumer.clone(), period_id)
}

fn spent_key(consumer: &Address, period_id: u64) -> DataKey {
    DataKey::SubscriptionRenewalSpent(consumer.clone(), period_id)
}

fn read_record(env: &Env, consumer: &Address) -> Option<AutoRenewRecord> {
    let key = record_key(consumer);
    let rec: Option<AutoRenewRecord> = env.storage().persistent().get(&key);
    if rec.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    rec
}

fn write_record(env: &Env, record: &AutoRenewRecord) {
    let key = record_key(&record.consumer);
    env.storage().persistent().set(&key, record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_auth(env: &Env, consumer: &Address, period_id: u64) -> Option<RenewalAuthorization> {
    let key = auth_key(consumer, period_id);
    let a: Option<RenewalAuthorization> = env.storage().persistent().get(&key);
    if a.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    a
}

fn write_auth(env: &Env, consumer: &Address, auth: &RenewalAuthorization) {
    let key = auth_key(consumer, auth.period_id);
    env.storage().persistent().set(&key, auth);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn is_spent(env: &Env, consumer: &Address, period_id: u64) -> bool {
    env.storage()
        .persistent()
        .has(&spent_key(consumer, period_id))
}

fn mark_spent(env: &Env, consumer: &Address, period_id: u64) {
    let key = spent_key(consumer, period_id);
    env.storage().persistent().set(&key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn clear_spent(env: &Env, consumer: &Address, period_id: u64) {
    let key = spent_key(consumer, period_id);
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }
}

/// The amount a renewal is permitted to move: `min(plan_amount, max_amount_per_period)`.
///
/// A duration with no registered plan prices at 0, so no authorization for it
/// can move tokens.
fn authorized_amount(env: &Env, record: &AutoRenewRecord) -> i128 {
    get_plan_amount(env, record.plan_duration)
        .unwrap_or(0)
        .min(record.max_amount_per_period)
}

/// Emits a failed attempt and returns it. Never panics.
fn fail(env: &Env, consumer: &Address, reason: u32, period_id: u64, expiry: u64) -> RenewalAttempt {
    AutoRenewalAttemptEvent {
        consumer: consumer.clone(),
        success: false,
        amount: 0,
        reason,
        period_id,
        expiry,
    }
    .publish(env);
    RenewalAttempt {
        consumer: consumer.clone(),
        renewed: false,
        amount: 0,
        reason,
        period_id,
        expiry,
    }
}

/// Grants a bounded standing auto-renewal authorization. Consumer-signed.
///
/// # Errors
///
/// * [`ErrorCode::InvalidConfiguration`] — `plan_duration`,
///   `max_amount_per_period` or `periods_authorized` is zero, or exceeds the
///   documented bound ([`MAX_PLAN_DURATION`] / [`MAX_PERIODS_AUTHORIZED`]).
///
/// Re-granting **supersedes** any previous record and resets
/// `authorization_nonce` to 0. Since the next authorization the consumer issues
/// must be strictly greater than 0, and a stored authorization must additionally
/// match `record.authorization_nonce`, an authorization captured under the *old*
/// grant can never be replayed against the new one.
pub fn enable_auto_renewal(
    env: &Env,
    consumer: Address,
    token: Address,
    plan_duration: u32,
    max_amount_per_period: i128,
    periods_authorized: u32,
) {
    consumer.require_auth();

    if plan_duration == 0
        || plan_duration > MAX_PLAN_DURATION
        || max_amount_per_period <= 0
        || periods_authorized == 0
        || periods_authorized > MAX_PERIODS_AUTHORIZED
    {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let record = AutoRenewRecord {
        consumer: consumer.clone(),
        token,
        plan_duration,
        max_amount_per_period,
        periods_authorized,
        periods_used: 0,
        authorization_nonce: 0,
        // The first period is immediately renewable.
        next_renewal_timestamp: env.ledger().timestamp(),
        active: true,
        cancelled: false,
    };
    write_record(env, &record);

    AutoRenewalAuthorizationEvent {
        consumer,
        action: ACTION_GRANTED,
        max_amount_per_period,
        plan_duration,
    }
    .publish(env);
}

/// Revokes a standing auto-renewal authorization. Consumer-signed.
///
/// Idempotent-safe: revoking a record that is already inactive does not panic,
/// and revoking when no record exists is a no-op.
pub fn disable_auto_renewal(env: &Env, consumer: Address) {
    consumer.require_auth();

    let Some(mut record) = read_record(env, &consumer) else {
        return;
    };

    record.active = false;
    write_record(env, &record);

    AutoRenewalAuthorizationEvent {
        consumer,
        action: ACTION_REVOKED,
        max_amount_per_period: record.max_amount_per_period,
        plan_duration: record.plan_duration,
    }
    .publish(env);
}

/// Issues a single-use authorization for the period currently due.
///
/// # Errors
///
/// * [`ErrorCode::AutoRenewalNotEnabled`] — no record, or it is not active.
/// * [`ErrorCode::AutoRenewalCancelled`]   — the subscription was cancelled.
/// * [`ErrorCode::RenewalAuthorizationMissing`] — `period_id` is not the period
///   currently due. A consumer may **not** pre-authorize a future period, so a
///   captured future authorization is useless to an attacker.
/// * [`ErrorCode::RenewalAuthorizationReplay`] — `nonce <= record.authorization_nonce`.
pub fn authorize_renewal(env: &Env, consumer: Address, period_id: u64, nonce: u64) {
    consumer.require_auth();

    let Some(mut record) = read_record(env, &consumer) else {
        panic_with_error!(env, ErrorCode::AutoRenewalNotEnabled);
    };

    if record.cancelled {
        panic_with_error!(env, ErrorCode::AutoRenewalCancelled);
    }
    if !record.active {
        panic_with_error!(env, ErrorCode::AutoRenewalNotEnabled);
    }
    if period_id != record.next_renewal_timestamp {
        panic_with_error!(env, ErrorCode::RenewalAuthorizationMissing);
    }
    if nonce <= record.authorization_nonce {
        panic_with_error!(env, ErrorCode::RenewalAuthorizationReplay);
    }

    let auth = RenewalAuthorization {
        period_id,
        nonce,
        amount: authorized_amount(env, &record),
        consumed: false,
    };
    write_auth(env, &consumer, &auth);

    record.authorization_nonce = nonce;
    write_record(env, &record);

    // A spent period is rolled forward to a strictly later period id, so the
    // spent marker for *this* id can never legitimately reappear; clearing it
    // keeps the invariant explicit and costs one storage touch.
    clear_spent(env, &consumer, period_id);
}

/// Attempts one renewal. Callable by any keeper; never panics.
///
/// A failure is a returned [`RenewalAttempt`], never a panic, so a keeper can
/// never lock a consumer out of their query path and never burns unbounded gas:
/// the work is a fixed number of storage reads and at most one token transfer.
///
/// ## Ordering (strict CEI)
///
/// All state writes — marking the authorization consumed, setting the spent
/// marker, advancing `next_renewal_timestamp`, bumping the nonce and extending
/// the subscription expiry — happen **before** the token transfer, so there is
/// no window in which a re-entrant caller observes a half-applied renewal. The
/// [`reentrancy`] guard is entered immediately before the transfer and exited
/// immediately after; a callback that re-enters `try_auto_renew` hits
/// `ErrorCode::Reentrant` and fails.
///
/// The amount moved is read from the stored authorization, never recomputed from
/// ambient state, so a change to plan pricing after authorization cannot enlarge
/// what a single-use authorization permits. If the transfer itself fails the
/// whole invocation reverts, so a failed transfer can never leave the effects
/// committed.
pub fn try_auto_renew(env: &Env, consumer: Address) -> RenewalAttempt {
    // ── Checks ──────────────────────────────────────────────────────────────
    // Every one of these returns a value; none of them panics.
    let Some(record) = read_record(env, &consumer) else {
        return fail(
            env,
            &consumer,
            ErrorCode::AutoRenewalNotEnabled as u32,
            0,
            0,
        );
    };
    let period_id = record.next_renewal_timestamp;

    if record.cancelled {
        return fail(
            env,
            &consumer,
            ErrorCode::AutoRenewalCancelled as u32,
            period_id,
            0,
        );
    }
    if !record.active {
        return fail(
            env,
            &consumer,
            ErrorCode::AutoRenewalNotEnabled as u32,
            period_id,
            0,
        );
    }
    if record.periods_used >= record.periods_authorized {
        return fail(
            env,
            &consumer,
            ErrorCode::AutoRenewalAllowanceExceeded as u32,
            period_id,
            0,
        );
    }
    if is_spent(env, &consumer, period_id) {
        return fail(
            env,
            &consumer,
            ErrorCode::RenewalAuthorizationReplay as u32,
            period_id,
            0,
        );
    }
    // Not yet due: a period is identified by its due timestamp and only becomes
    // renewable once the ledger reaches it. This is reported as `NoData` (8) —
    // there is nothing to renew *yet* — and is distinct from every other reason
    // so a keeper can tell "too early" from "denied".
    if env.ledger().timestamp() < period_id {
        // If the *preceding* period is already spent then this period's
        // authorization was consumed and rolled forward into it, so a second
        // attempt in the same cycle is a replay of a spent authorization
        // rather than a merely premature one. Reporting the replay keeps the
        // "one renewal per period" invariant legible to a keeper (and to the
        // monitor), instead of reporting the same "too early" code for what is
        // really a second drain attempt.
        let prior = period_id.saturating_sub(record.plan_duration as u64);
        let reason = if prior != period_id && is_spent(env, &consumer, prior) {
            ErrorCode::RenewalAuthorizationReplay as u32
        } else {
            ErrorCode::NoData as u32
        };
        return fail(env, &consumer, reason, period_id, 0);
    }
    let Some(auth) = read_auth(env, &consumer, period_id) else {
        return fail(
            env,
            &consumer,
            ErrorCode::RenewalAuthorizationMissing as u32,
            period_id,
            0,
        );
    };
    if auth.consumed || auth.nonce != record.authorization_nonce {
        return fail(
            env,
            &consumer,
            ErrorCode::RenewalAuthorizationReplay as u32,
            period_id,
            0,
        );
    }
    let amount = auth.amount;
    if amount <= 0 {
        return fail(env, &consumer, ErrorCode::NoData as u32, period_id, 0);
    }

    // A renewal is never performed on behalf of a consumer who never approved
    // or who has since revoked: the live allowance is read here, every time.
    let allowance = token::Client::new(env, &record.token)
        .allowance(&consumer, &env.current_contract_address());
    if allowance < amount {
        return fail(
            env,
            &consumer,
            ErrorCode::AutoRenewalAllowanceExceeded as u32,
            period_id,
            0,
        );
    }

    let Some(expiry) = read_subscription_expiry(env, &consumer) else {
        return fail(
            env,
            &consumer,
            ErrorCode::NoActiveSubscription as u32,
            period_id,
            0,
        );
    };
    if expiry < env.ledger().timestamp() {
        return fail(
            env,
            &consumer,
            ErrorCode::SubscriptionExpired as u32,
            period_id,
            expiry,
        );
    }

    // ── Effects (all state writes before the external call) ─────────────────
    let mut updated = record.clone();
    // The authorization is now dead, permanently.
    write_auth(
        env,
        &consumer,
        &RenewalAuthorization {
            period_id,
            nonce: auth.nonce,
            amount,
            consumed: true,
        },
    );
    mark_spent(env, &consumer, period_id);
    updated.periods_used = record.periods_used.saturating_add(1);
    updated.next_renewal_timestamp = period_id.saturating_add(record.plan_duration as u64);
    // Bumping past the spent nonce means the authorization just consumed can
    // never satisfy `auth.nonce == record.authorization_nonce` again.
    updated.authorization_nonce = auth.nonce.saturating_add(1);
    write_record(env, &updated);

    let new_expiry = expiry.saturating_add(record.plan_duration as u64);
    write_subscription_expiry(env, &consumer, new_expiry);

    // ── Interaction (the only external call) ────────────────────────────────
    // The pull uses `transfer_from` so it is settled **out of the consumer's
    // standing SAC allowance** rather than requiring `consumer.require_auth()`
    // in this frame. That is what makes the keeper's call permissionless: the
    // authority the contract spends is the pre-approval the consumer granted
    // (and that the check above just verified is still live), not a fresh
    // signature. `transfer` — which moves the consumer's own balance directly —
    // would require the consumer's auth and could never be performed by a
    // keeper, so the whole allowance model would be unreachable.
    reentrancy::enter(env);
    let token_client = token::Client::new(env, &record.token);
    let contract_addr = env.current_contract_address();
    token_client.transfer_from(&contract_addr, &consumer, &contract_addr, &amount);
    reentrancy::exit(env);

    AutoRenewalAttemptEvent {
        consumer: consumer.clone(),
        success: true,
        amount,
        reason: 0,
        period_id,
        expiry: new_expiry,
    }
    .publish(env);

    RenewalAttempt {
        consumer,
        renewed: true,
        amount,
        reason: 0,
        period_id,
        expiry: new_expiry,
    }
}

/// Returns a consumer's standing authorization, if any.
///
/// Read-only; extends the record's TTL when present.
pub fn get_auto_renewal_record(env: &Env, consumer: &Address) -> Option<AutoRenewRecord> {
    read_record(env, consumer)
}

/// Returns the single-use authorization for a period, if any.
///
/// Read-only; extends the authorization's TTL when present.
pub fn get_renewal_authorization(
    env: &Env,
    consumer: &Address,
    period_id: u64,
) -> Option<RenewalAuthorization> {
    read_auth(env, consumer, period_id)
}

/// Consumes a consumer's single-use authorization for the period currently due.
///
/// Bookkeeping only: marks the stored [`RenewalAuthorization`] for
/// `record.next_renewal_timestamp` as `consumed = true` and sets the
/// `DataKey::SubscriptionRenewalSpent(consumer, period_id)` marker so that
/// period id can never be authorized or renewed again. It does **not** touch the
/// `cancelled`/`active` flags and emits no event — that is
/// [`revoke_on_cancel`]'s job, expressed in terms of this function so the two
/// cannot drift.
///
/// No-op when the consumer has no record or has no outstanding authorization.
pub fn consume_current_authorization(env: &Env, consumer: &Address) {
    let Some(record) = read_record(env, consumer) else {
        return;
    };
    let period_id = record.next_renewal_timestamp;

    if let Some(auth) = read_auth(env, consumer, period_id) {
        if !auth.consumed {
            write_auth(
                env,
                consumer,
                &RenewalAuthorization {
                    period_id,
                    nonce: auth.nonce,
                    amount: auth.amount,
                    consumed: true,
                },
            );
        }
    }
    mark_spent(env, consumer, period_id);
}

/// Atomically voids every auto-renewal right for a cancelled subscription.
///
/// Called by the integrator from `subscription::cancel_subscription` so that
/// cancellation and revocation of renewal rights are one atomic unit: there is
/// no window in which a cancelled subscription can renew. In a single
/// invocation this sets `cancelled = true` **and** `active = false`, consumes the
/// currently-outstanding authorization and marks its period id spent, and emits
/// `AutoRenewalAuthorizationEvent { action: 2 }`.
///
/// The outstanding period is found without an unbounded scan: the only period
/// that can ever be authorized is `record.next_renewal_timestamp`, and spent
/// periods are recorded under the sibling `DataKey::SubscriptionRenewalSpent`
/// key so each period id is individually, permanently dead.
///
/// A total no-op — no panic, no event — when the consumer has no
/// `AutoRenewRecord`, because `cancel_subscription` calls it unconditionally.
pub fn revoke_on_cancel(env: &Env, consumer: &Address) {
    let Some(mut record) = read_record(env, consumer) else {
        return;
    };

    record.cancelled = true;
    record.active = false;
    write_record(env, &record);

    consume_current_authorization(env, consumer);

    AutoRenewalAuthorizationEvent {
        consumer: consumer.clone(),
        action: ACTION_CANCELLED,
        max_amount_per_period: record.max_amount_per_period,
        plan_duration: record.plan_duration,
    }
    .publish(env);
}
