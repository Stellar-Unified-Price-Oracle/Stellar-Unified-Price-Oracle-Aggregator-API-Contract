#![cfg(test)]

//! # Auto-renewal test suite (#289)
//!
//! See `docs/auto-renewal.md`. Every acceptance criterion for the
//! token-approval-driven auto-renewal path is covered here: the happy path, the
//! allowance-drain bound, the reentrancy guard, cancellation/supersession,
//! authorization replay, non-panicking failures and the event schema.

use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Events as _},
    token,
    xdr::{ContractEvent, ContractEventBody, ScVal},
    Address, Env, IntoVal, MuxedAddress, Symbol, Val, Vec as SVec,
};

use std::string::ToString;

use crate::auto_renewal;
use crate::events::{AutoRenewalAttemptEvent, AutoRenewalAuthorizationEvent};
use crate::test_helpers::*;
use crate::types::{ErrorCode as EC, RenewalAttempt};
use crate::PriceOracleContractClient;

/// One plan period, in seconds (30 days).
const DURATION: u32 = 2_592_000;
/// Registered plan price for [`DURATION`]. A renewal moves
/// `min(PLAN_AMOUNT, MAX_PER_PERIOD)`.
const PLAN_AMOUNT: i128 = 1_000;
/// Per-period ceiling the consumer grants.
const MAX_PER_PERIOD: i128 = 1_000;
/// Generous standing allowance, so most tests measure the *renewal* bound and
/// not the SAC allowance.
const STANDING_ALLOWANCE: i128 = 1_000_000_000;
/// Starting mint.
const START_BALANCE: i128 = 1_000_000;
/// Ledger timestamp every fixture starts at.
const T0: u64 = 1_000_000;
/// A far-future SAC approval expiry.
const APPROVAL_LEDGER: u32 = 2_000_000;

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Deploys a real SAC, registers a plan, subscribes `consumer` and grants it a
/// standing allowance to this contract.
fn setup_with_allowance<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address, Address) {
    ledger_default(e, 1_000, T0);
    let (client, _admin) = setup_contract(e);

    let tok = deploy_token(e);
    client.set_subscription_token(&tok);
    client.set_subscription_price(&DURATION, &PLAN_AMOUNT);

    let consumer = Address::generate(e);
    mint_token(e, &tok, &consumer, START_BALANCE);
    client.subscribe(&consumer, &DURATION);

    // The consumer pre-approves this contract to move up to
    // `STANDING_ALLOWANCE` on their behalf.
    token::StellarAssetClient::new(e, &tok).approve(
        &consumer,
        &client.address,
        &STANDING_ALLOWANCE,
        &APPROVAL_LEDGER,
    );

    (client, consumer, tok)
}

/// Like [`setup_with_allowance`] but with **no** SAC approval at all, so the
/// allowance check is the only thing that can deny the first renewal.
fn setup_without_allowance<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address, Address) {
    ledger_default(e, 1_000, T0);
    let (client, _admin) = setup_contract(e);

    let tok = deploy_token(e);
    client.set_subscription_token(&tok);
    client.set_subscription_price(&DURATION, &PLAN_AMOUNT);

    let consumer = Address::generate(e);
    mint_token(e, &tok, &consumer, START_BALANCE);
    client.subscribe(&consumer, &DURATION);

    (client, consumer, tok)
}

fn balance(e: &Env, tok: &Address, who: &Address) -> i128 {
    token::StellarAssetClient::new(e, tok).balance(who)
}

fn allowance(e: &Env, tok: &Address, from: &Address, spender: &Address) -> i128 {
    token::StellarAssetClient::new(e, tok).allowance(from, spender)
}

fn grant_renewal(
    client: &PriceOracleContractClient<'_>,
    consumer: &Address,
    tok: &Address,
    max_per_period: i128,
    periods: u32,
) {
    client.enable_auto_renewal(consumer, tok, &DURATION, &max_per_period, &periods);
}

/// Authorizes the period that is currently due and returns its id.
fn authorize_current(
    client: &PriceOracleContractClient<'_>,
    consumer: &Address,
    nonce: u64,
) -> u64 {
    let period_id = client
        .get_auto_renewal_record(consumer)
        .expect("record")
        .next_renewal_timestamp;
    client.authorize_renewal(consumer, &period_id, &nonce);
    period_id
}

fn contract_events(e: &Env, c: &PriceOracleContractClient<'_>) -> std::vec::Vec<ContractEvent> {
    e.events()
        .all()
        .filter_by_contract(&c.address)
        .events()
        .to_vec()
}

fn topics(ev: &ContractEvent) -> std::vec::Vec<ScVal> {
    match &ev.body {
        ContractEventBody::V0(v0) => v0.topics.to_vec(),
    }
}

/// `true` if the event is `name`.
///
/// A `#[contractevent]` puts its own snake_case name in the leading topic.
fn is_event(ev: &ContractEvent, name: &str) -> bool {
    match topics(ev).first() {
        Some(ScVal::Symbol(sym)) => sym.0.to_string() == name,
        _ => false,
    }
}

/// Reads a field out of an event's `Map` data payload.
fn field(ev: &ContractEvent, key: &str) -> Option<ScVal> {
    let ContractEventBody::V0(v0) = &ev.body;
    let ScVal::Map(Some(map)) = &v0.data else {
        return None;
    };
    map.0
        .iter()
        .find(|entry| match &entry.key {
            ScVal::Symbol(sym) => sym.0.to_string() == key,
            _ => false,
        })
        .map(|entry| entry.val.clone())
}

fn as_bool(v: &ScVal) -> Option<bool> {
    match v {
        ScVal::Bool(b) => Some(*b),
        _ => None,
    }
}

fn as_u32(v: &ScVal) -> Option<u32> {
    match v {
        ScVal::U32(u) => Some(*u),
        _ => None,
    }
}

fn as_i128(v: &ScVal) -> Option<i128> {
    match v {
        ScVal::I128(parts) => Some(((parts.hi as i128) << 64) | parts.lo as i128),
        _ => None,
    }
}

fn as_u64(v: &ScVal) -> Option<u64> {
    match v {
        ScVal::U64(u) => Some(*u),
        _ => None,
    }
}

/// Decodes `(success, reason)` from every `AutoRenewalAttemptEvent` in `evs`.
///
/// `reason` is the `ErrorCode` discriminant, or 0 on success.
fn attempt_reasons(evs: &[ContractEvent]) -> std::vec::Vec<(bool, u32)> {
    let mut out = std::vec::Vec::new();
    for ev in evs {
        if !is_event(ev, "auto_renewal_attempt_event") {
            continue;
        }
        let success = field(ev, "success")
            .as_ref()
            .and_then(as_bool)
            .unwrap_or(false);
        let reason = field(ev, "reason")
            .as_ref()
            .and_then(as_u32)
            .unwrap_or(u32::MAX);
        out.push((success, reason));
    }
    out
}

/// Decodes `(amount, reason, period_id, expiry)` from every attempt event.
fn attempt_details(evs: &[ContractEvent]) -> std::vec::Vec<(i128, u32, u64, u64)> {
    let mut out = std::vec::Vec::new();
    for ev in evs {
        if !is_event(ev, "auto_renewal_attempt_event") {
            continue;
        }
        out.push((
            field(ev, "amount").as_ref().and_then(as_i128).unwrap_or(-1),
            field(ev, "reason")
                .as_ref()
                .and_then(as_u32)
                .unwrap_or(u32::MAX),
            field(ev, "period_id")
                .as_ref()
                .and_then(as_u64)
                .unwrap_or(u64::MAX),
            field(ev, "expiry")
                .as_ref()
                .and_then(as_u64)
                .unwrap_or(u64::MAX),
        ));
    }
    out
}

/// Decodes the `action` of every authorization event (0 = granted, 1 = revoked,
/// 2 = cancelled with the subscription).
fn authorization_actions(evs: &[ContractEvent]) -> std::vec::Vec<u32> {
    let mut out = std::vec::Vec::new();
    for ev in evs {
        if !is_event(ev, "auto_renewal_authorization_event") {
            continue;
        }
        out.push(
            field(ev, "action")
                .as_ref()
                .and_then(as_u32)
                .unwrap_or(u32::MAX),
        );
    }
    out
}

// ---------------------------------------------------------------------------
// Re-entrant mock token (#289, criterion 3)
// ---------------------------------------------------------------------------

const MOVED: Symbol = soroban_sdk::symbol_short!("moved");
const REENTERED: Symbol = soroban_sdk::symbol_short!("reentered");
const ORACLE: Symbol = soroban_sdk::symbol_short!("oracle");
const SUBJECT: Symbol = soroban_sdk::symbol_short!("subject");

/// A hostile token whose `transfer` re-enters the oracle's `try_auto_renew`.
///
/// It implements just enough of the token interface for the oracle to use it:
/// `allowance` (always generous), `transfer` and `transfer_from` (both record
/// the amount moved, then call back into `try_auto_renew` for the subject).
#[contract]
pub struct MockReentrantToken;

#[contractimpl]
impl MockReentrantToken {
    /// Points the callback at `oracle.try_auto_renew(subject)`.
    pub fn setup(env: Env, oracle: Address, subject: Address) {
        env.storage().instance().set(&ORACLE, &oracle);
        env.storage().instance().set(&SUBJECT, &subject);
    }

    /// Always reports a generous standing allowance.
    pub fn allowance(env: Env, _from: Address, _spender: Address) -> i128 {
        let _ = env;
        1_000_000_000i128
    }

    /// Records the amount moved, then re-enters `try_auto_renew`.
    pub fn transfer(env: Env, _from: Address, to: MuxedAddress, amount: i128) {
        let _ = to;
        Self::record(&env, amount);
        Self::reenter(&env);
    }

    /// Records the amount pulled out of an allowance, then re-enters.
    pub fn transfer_from(env: Env, _spender: Address, _from: Address, _to: Address, amount: i128) {
        Self::record(&env, amount);
        Self::reenter(&env);
    }

    fn record(env: &Env, amount: i128) {
        let moved: i128 = env.storage().instance().get(&MOVED).unwrap_or(0);
        env.storage()
            .instance()
            .set(&MOVED, &moved.saturating_add(amount));
    }

    /// Accumulates the moved amount and then re-enters the oracle's renewal path.
    ///
    /// A no-op until [`MockReentrantToken::setup`] has armed a callback, so the
    /// fixture's own `subscribe` transfer is harmless.
    fn reenter(env: &Env) {
        let oracle: Option<Address> = env.storage().instance().get(&ORACLE);
        let subject: Option<Address> = env.storage().instance().get(&SUBJECT);
        let (Some(oracle), Some(subject)) = (oracle, subject) else {
            return;
        };
        let args: SVec<Val> = (subject,).into_val(env);
        let ok = matches!(
            env.try_invoke_contract::<RenewalAttempt, soroban_sdk::Error>(
                &oracle,
                &Symbol::new(env, "try_auto_renew"),
                args,
            ),
            Ok(Ok(a)) if a.renewed
        );
        env.storage().instance().set(&REENTERED, &ok);
    }

    /// Total amount this token was ever asked to move.
    pub fn moved(env: Env) -> i128 {
        env.storage().instance().get(&MOVED).unwrap_or(0)
    }

    /// `true` only if the re-entrant `try_auto_renew` reported a success.
    pub fn reentered(env: Env) -> bool {
        env.storage().instance().get(&REENTERED).unwrap_or(false)
    }
}

/// A fixture whose auto-renewal token is the hostile mock instead of a real SAC,
/// so the only token movement under test is the renewal's own transfer.
fn setup_with_mock_token<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address, Address) {
    ledger_default(e, 1_000, T0);
    let (client, _admin) = setup_contract(e);

    let mock = e.register(MockReentrantToken, ());
    let consumer = Address::generate(e);

    client.set_subscription_token(&mock);
    client.set_subscription_price(&DURATION, &PLAN_AMOUNT);
    client.subscribe(&consumer, &DURATION);
    // Arm the callback only once the subscription exists, so the only re-entry
    // under test is the one the renewal's own transfer triggers.
    MockReentrantTokenClient::new(e, &mock).setup(&client.address, &consumer);

    (client, consumer, mock)
}

// ===========================================================================
// 1. Token approval integration for auto-renewal (happy path)
// ===========================================================================

#[test]
fn auto_renewal_moves_exactly_the_authorized_amount_on_the_approved_allowance() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);

    let before = balance(&e, &tok, &consumer);
    let expiry_before = client.get_subscription_expiry(&consumer);
    assert_eq!(expiry_before, T0 + DURATION as u64);

    let period_id = authorize_current(&client, &consumer, 1);
    assert_eq!(period_id, T0, "the first period is due immediately");

    let attempt = client.try_auto_renew(&consumer);
    assert!(attempt.renewed, "renewal should succeed: {attempt:?}");
    assert_eq!(attempt.amount, PLAN_AMOUNT);
    assert_eq!(attempt.reason, 0);
    assert_eq!(attempt.period_id, period_id);
    assert_eq!(attempt.expiry, expiry_before + DURATION as u64);

    // Exactly one period's worth left the consumer — no more, no less.
    assert_eq!(before - balance(&e, &tok, &consumer), PLAN_AMOUNT);
    // The standing allowance was debited by the same amount.
    assert_eq!(
        allowance(&e, &tok, &consumer, &client.address),
        STANDING_ALLOWANCE - PLAN_AMOUNT
    );
    // The expiry really advanced.
    assert_eq!(client.get_subscription_expiry(&consumer), attempt.expiry);

    let rec = client.get_auto_renewal_record(&consumer).unwrap();
    assert_eq!(rec.periods_used, 1);
    assert_eq!(rec.next_renewal_timestamp, period_id + DURATION as u64);
    assert!(rec.active && !rec.cancelled);

    // The single-use authorization is now dead.
    let auth = client
        .get_renewal_authorization(&consumer, &period_id)
        .unwrap();
    assert!(auth.consumed);
}

// ===========================================================================
// 2. A renewal triggered on every query cannot drain beyond the per-period cap
// ===========================================================================

#[test]
fn repeated_renewal_attempts_in_one_period_move_at_most_one_period_amount() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);

    let before = balance(&e, &tok, &consumer);
    authorize_current(&client, &consumer, 1);

    // Ten keeper invocations inside the *same* period, with no ledger movement.
    let mut successes = 0;
    for _ in 0..10 {
        let a = client.try_auto_renew(&consumer);
        if a.renewed {
            successes += 1;
        } else {
            assert_eq!(a.reason, EC::RenewalAuthorizationReplay as u32);
            assert_eq!(a.amount, 0);
        }
    }
    assert_eq!(successes, 1, "only the first attempt may renew");
    assert_eq!(before - balance(&e, &tok, &consumer), MAX_PER_PERIOD);

    // Advance exactly one period, authorize and renew again.
    ledger_default(&e, 1_001, T0 + DURATION as u64);
    authorize_current(&client, &consumer, 3);
    let a = client.try_auto_renew(&consumer);
    assert!(a.renewed);
    let moved = before - balance(&e, &tok, &consumer);
    assert_eq!(moved, 2 * MAX_PER_PERIOD);
    assert!(
        moved <= 3 * MAX_PER_PERIOD,
        "running total must stay within periods_authorized * max_amount_per_period"
    );
}

#[test]
fn exhausting_every_authorized_period_blocks_further_renewals() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    let periods = 3u32;
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, periods);

    let before = balance(&e, &tok, &consumer);
    let mut nonce = 1u64;
    for i in 0..periods {
        authorize_current(&client, &consumer, nonce);
        let a = client.try_auto_renew(&consumer);
        assert!(a.renewed, "period {i} should renew: {a:?}");
        nonce += 2;
        if i + 1 < periods {
            ledger_default(&e, 1_000 + i + 1, T0 + (i as u64 + 1) * DURATION as u64);
        }
    }

    let moved = before - balance(&e, &tok, &consumer);
    assert_eq!(moved, periods as i128 * MAX_PER_PERIOD);

    // One more period: authorized, but the standing allowance is spent.
    authorize_current(&client, &consumer, nonce);
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalAllowanceExceeded as u32);
    assert_eq!(balance(&e, &tok, &consumer), before - moved, "zero moved");
}

// ===========================================================================
// 3. Reentrancy through the token callback is blocked
// ===========================================================================

#[test]
fn reentrant_token_transfer_cannot_re_enter_try_auto_renew() {
    let e = Env::default();
    let (client, consumer, mock) = setup_with_mock_token(&e);
    grant_renewal(&client, &consumer, &mock, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);

    let m = MockReentrantTokenClient::new(&e, &mock);
    let moved_before = m.moved();

    let attempt = client.try_auto_renew(&consumer);
    assert!(
        attempt.renewed,
        "the outer renewal must still succeed: {attempt:?}"
    );

    // The callback fired and did NOT get a second renewal out of the oracle.
    assert!(
        !m.reentered(),
        "re-entrant try_auto_renew must not report a renewal"
    );
    // Exactly one transfer's worth moved. If `reentrancy::enter` were removed
    // from `try_auto_renew`, the callback would run a *second* full renewal
    // frame against a record whose effects were already committed; `moved()`
    // would then be `moved_before + 2 * PLAN_AMOUNT` with `reentered() == true`.
    // Both assertions below are the ones that fail without the guard.
    assert_eq!(m.moved() - moved_before, PLAN_AMOUNT);
    assert_eq!(
        client
            .get_auto_renewal_record(&consumer)
            .unwrap()
            .periods_used,
        1,
        "one renewal, not two"
    );
}

// ===========================================================================
// 4. Cancelled or superseded subscriptions provably cannot renew
// ===========================================================================

#[test]
fn cancelled_subscription_cannot_renew_even_with_a_live_authorization() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);

    let before = balance(&e, &tok, &consumer);
    client.cancel_subscription(&consumer);

    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalCancelled as u32);
    assert_eq!(balance(&e, &tok, &consumer), before, "zero tokens moved");

    let rec = client.get_auto_renewal_record(&consumer).unwrap();
    assert!(
        rec.cancelled && !rec.active,
        "cancellation is atomic with rights"
    );
}

#[test]
fn superseded_authorization_cannot_be_replayed_after_a_regrant() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);

    // Grant, then capture (period_id, nonce) of the outstanding authorization.
    let captured_period = authorize_current(&client, &consumer, 1);
    let captured = client
        .get_renewal_authorization(&consumer, &captured_period)
        .unwrap();
    assert_eq!(captured.nonce, 1);
    assert!(!captured.consumed);

    let before = balance(&e, &tok, &consumer);

    // The consumer re-grants: the old record (and its nonce counter) is
    // superseded and `authorization_nonce` resets to 0. The stored
    // authorization still carries nonce 1, so it no longer equals the record's
    // nonce and the attempt is rejected as a replay.
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    let fresh = client.get_auto_renewal_record(&consumer).unwrap();
    assert_eq!(fresh.periods_used, 0);
    assert_eq!(fresh.authorization_nonce, 0);

    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::RenewalAuthorizationReplay as u32);
    assert_eq!(balance(&e, &tok, &consumer), before, "zero tokens moved");
}

#[test]
fn renewal_and_cancellation_race_leaves_at_most_one_winner() {
    // Interleave in the *same* ledger: authorize, then attempt the renewal and
    // the cancellation back to back. Whichever takes effect, the other must be
    // void, and no tokens may move after the cancellation.
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    // Renewal first.
    let first = client.try_auto_renew(&consumer);
    assert!(first.renewed);
    client.cancel_subscription(&consumer);

    // The cancellation wins from here on: nothing else may move.
    let second = client.try_auto_renew(&consumer);
    assert!(!second.renewed);
    assert_eq!(second.reason, EC::AutoRenewalCancelled as u32);
    assert_eq!(balance(&e, &tok, &consumer), before - PLAN_AMOUNT);

    // The record is terminal.
    let rec = client.get_auto_renewal_record(&consumer).unwrap();
    assert!(rec.cancelled && !rec.active);
    assert_eq!(rec.periods_used, 1);
}

#[test]
fn cancellation_before_any_attempt_wins_the_race_outright() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    // Cancellation first, at the very same ledger as the due period.
    client.cancel_subscription(&consumer);
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalCancelled as u32);
    assert_eq!(balance(&e, &tok, &consumer), before, "zero tokens moved");
}

#[test]
fn revoke_on_cancel_is_a_total_no_op_without_a_record() {
    // `subscription::cancel_subscription` calls `revoke_on_cancel`
    // unconditionally, so a consumer who never used auto-renewal must not panic.
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let (client, _admin) = setup_contract(&e);
    let consumer = Address::generate(&e);

    e.as_contract(&client.address, || {
        auto_renewal::revoke_on_cancel(&e, &consumer);
    });

    // And through the public path, too: a plain cancel must still work.
    let tok = deploy_token(&e);
    client.set_subscription_token(&tok);
    client.set_subscription_price(&DURATION, &PLAN_AMOUNT);
    mint_token(&e, &tok, &consumer, START_BALANCE);
    client.subscribe(&consumer, &DURATION);
    client.cancel_subscription(&consumer);
    assert!(client.get_auto_renewal_record(&consumer).is_none());
}

#[test]
fn revoke_on_cancel_voids_the_outstanding_authorization_and_its_period() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    let period_id = authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    e.as_contract(&client.address, || {
        auto_renewal::revoke_on_cancel(&e, &consumer);
    });

    let rec = client.get_auto_renewal_record(&consumer).unwrap();
    assert!(rec.cancelled && !rec.active);
    let auth = client
        .get_renewal_authorization(&consumer, &period_id)
        .unwrap();
    assert!(auth.consumed, "the outstanding authorization is consumed");

    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalCancelled as u32);
    assert_eq!(balance(&e, &tok, &consumer), before, "zero tokens moved");

    // The spent marker also makes the period id permanently unusable.
    let res = e.try_invoke_contract::<(), soroban_sdk::Error>(
        &client.address,
        &Symbol::new(&e, "authorize_renewal"),
        (consumer.clone(), period_id, 9u64).into_val(&e),
    );
    assert!(
        res.is_err(),
        "a cancelled record accepts no new authorization"
    );
}

// ===========================================================================
// 5. Replay of a captured renewal authorization fails
// ===========================================================================

#[test]
fn captured_authorization_cannot_be_reissued_after_it_is_spent() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);

    let period_id = authorize_current(&client, &consumer, 1);
    let captured = client
        .get_renewal_authorization(&consumer, &period_id)
        .unwrap();
    assert_eq!((captured.period_id, captured.nonce), (period_id, 1));

    assert!(client.try_auto_renew(&consumer).renewed);

    // Re-issuing the *same* (period_id, nonce) against a later period is a
    // replay: the record's nonce is now 2, so `nonce <= record.authorization_nonce`.
    ledger_default(&e, 1_001, T0 + DURATION as u64);
    let args: SVec<Val> = (consumer.clone(), T0 + DURATION as u64, 1u64).into_val(&e);
    let res = e.try_invoke_contract::<(), soroban_sdk::Error>(
        &client.address,
        &Symbol::new(&e, "authorize_renewal"),
        args,
    );
    let expected: soroban_sdk::Error =
        soroban_sdk::Error::from_contract_error(EC::RenewalAuthorizationReplay as u32);
    assert!(
        matches!(res, Err(Ok(ref err)) if *err == expected),
        "re-issuing a spent nonce must be a replay, got {res:?}"
    );

    // The new period has no authorization at all (the replay was refused), and
    // the old period id is permanently spent — neither can move tokens.
    let rec = client.get_auto_renewal_record(&consumer).unwrap();
    assert_eq!(rec.next_renewal_timestamp, T0 + DURATION as u64);
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::RenewalAuthorizationMissing as u32);
    assert_eq!(a.amount, 0);
    assert!(
        client
            .get_renewal_authorization(&consumer, &period_id)
            .unwrap()
            .consumed
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #175)")]
fn authorize_renewal_rejects_a_nonce_that_is_not_strictly_increasing() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    let period_id = authorize_current(&client, &consumer, 5);
    // Same nonce again for the same period.
    client.authorize_renewal(&consumer, &period_id, &5);
}

#[test]
#[should_panic(expected = "Error(Contract, #176)")]
fn authorize_renewal_rejects_a_future_period_id() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    // Pre-authorizing a future period is refused, so a captured future
    // authorization is useless.
    client.authorize_renewal(&consumer, &(T0 + DURATION as u64), &1);
}

#[test]
#[should_panic(expected = "Error(Contract, #178)")]
fn authorize_renewal_is_refused_after_cancellation() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    let period_id = client
        .get_auto_renewal_record(&consumer)
        .unwrap()
        .next_renewal_timestamp;
    client.cancel_subscription(&consumer);
    client.authorize_renewal(&consumer, &period_id, &1);
}

#[test]
#[should_panic(expected = "Error(Contract, #177)")]
fn authorize_renewal_is_refused_without_a_record() {
    let e = Env::default();
    let (client, consumer, _tok) = setup_with_allowance(&e);
    client.authorize_renewal(&consumer, &T0, &1);
}

// ===========================================================================
// 6. Failed renewals never lock the consumer out
// ===========================================================================

#[test]
fn a_renewal_denied_for_allowance_succeeds_immediately_after_the_approval() {
    let e = Env::default();
    let (client, consumer, tok) = setup_without_allowance(&e);

    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    // Failure is a value, not a panic, and moves nothing.
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalAllowanceExceeded as u32);
    assert_eq!(a.amount, 0);
    assert_eq!(balance(&e, &tok, &consumer), before);

    // The failure consumed nothing: the very next attempt succeeds.
    token::StellarAssetClient::new(&e, &tok).approve(
        &consumer,
        &client.address,
        &STANDING_ALLOWANCE,
        &APPROVAL_LEDGER,
    );
    let b = client.try_auto_renew(&consumer);
    assert!(
        b.renewed,
        "the approval must unblock the next attempt: {b:?}"
    );
    assert_eq!(before - balance(&e, &tok, &consumer), PLAN_AMOUNT);
}

#[test]
fn every_failure_path_returns_a_value_rather_than_panicking() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);

    // No record at all.
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalNotEnabled as u32);

    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);

    // No authorization for the due period yet.
    let b = client.try_auto_renew(&consumer);
    assert!(!b.renewed);
    assert_eq!(b.reason, EC::RenewalAuthorizationMissing as u32);

    // Authorized, but the period is not due yet.
    let period_id = authorize_current(&client, &consumer, 1);
    ledger_default(&e, 1_000, T0 - 1);
    let c = client.try_auto_renew(&consumer);
    assert!(!c.renewed);
    assert_eq!(
        c.reason,
        EC::NoData as u32,
        "not-yet-due is reported as NoData"
    );
    assert_eq!(c.period_id, period_id);

    // Back at the due timestamp it renews; the work is a fixed number of reads
    // and at most one transfer, so no path is unbounded.
    ledger_default(&e, 1_000, T0);
    assert!(client.try_auto_renew(&consumer).renewed);
}

#[test]
fn a_consumer_who_never_approved_gets_allowance_exceeded_and_moves_nothing() {
    let e = Env::default();
    let (client, consumer, tok) = setup_without_allowance(&e);

    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalAllowanceExceeded as u32);
    assert_eq!(balance(&e, &tok, &consumer), before);
}

#[test]
fn a_revoked_allowance_stops_renewals() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);

    // The consumer revokes.
    token::StellarAssetClient::new(&e, &tok).approve(
        &consumer,
        &client.address,
        &0,
        &APPROVAL_LEDGER,
    );

    let before = balance(&e, &tok, &consumer);
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalAllowanceExceeded as u32);
    assert_eq!(balance(&e, &tok, &consumer), before);
}

// ===========================================================================
// 7. Events for renewal attempts (success/failure) with reasons
// ===========================================================================

#[test]
fn attempt_events_carry_success_and_the_documented_error_discriminants() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);

    // The host scopes diagnostics to one top-level invocation, so the events of
    // each attempt are captured immediately after that attempt.

    // Failure 1: no authorization for the due period -> 176.
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    let missing = client.try_auto_renew(&consumer);
    let missing_events = contract_events(&e, &client);
    assert_eq!(missing.reason, EC::RenewalAuthorizationMissing as u32);

    // Failure 2: cancelled -> 178.
    let other = Address::generate(&e);
    mint_token(&e, &tok, &other, START_BALANCE);
    client.subscribe(&other, &DURATION);
    grant_renewal(&client, &other, &tok, MAX_PER_PERIOD, 3);
    client.cancel_subscription(&other);
    let cancelled = client.try_auto_renew(&other);
    let cancelled_events = contract_events(&e, &client);
    assert_eq!(cancelled.reason, EC::AutoRenewalCancelled as u32);

    // Success -> 0.
    let period_id = authorize_current(&client, &consumer, 1);
    let ok = client.try_auto_renew(&consumer);
    let success_events = contract_events(&e, &client);
    assert!(ok.renewed);

    // At least one success and two distinct failure reasons, each carrying the
    // documented `ErrorCode` discriminant.
    assert_eq!(attempt_reasons(&missing_events), vec![(false, 176u32)]);
    assert_eq!(attempt_reasons(&cancelled_events), vec![(false, 178u32)]);
    assert_eq!(attempt_reasons(&success_events), vec![(true, 0u32)]);

    // The success event also carries the amount, the period and the new expiry.
    assert_eq!(
        attempt_details(&success_events),
        vec![(PLAN_AMOUNT, 0u32, period_id, ok.expiry)]
    );
    // A failure event reports amount 0, the real period id and the reason.
    assert_eq!(
        attempt_details(&missing_events),
        vec![(0, 176u32, period_id, 0u64)]
    );
}

#[test]
fn grant_revoke_and_cancel_authorization_events_carry_their_action() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);

    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 4);
    let granted = authorization_actions(&contract_events(&e, &client));
    client.disable_auto_renewal(&consumer);
    let revoked = authorization_actions(&contract_events(&e, &client));
    client.cancel_subscription(&consumer);
    let cancelled = authorization_actions(&contract_events(&e, &client));

    assert_eq!(granted, vec![0u32], "action 0 = granted");
    assert_eq!(revoked, vec![1u32], "action 1 = revoked");
    assert_eq!(
        cancelled,
        vec![2u32],
        "action 2 = cancelled with the subscription"
    );
}

// ===========================================================================
// 8. disable_auto_renewal stops renewals immediately
// ===========================================================================

#[test]
fn disable_auto_renewal_stops_renewals_immediately() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    client.disable_auto_renewal(&consumer);
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::AutoRenewalNotEnabled as u32);
    assert_eq!(balance(&e, &tok, &consumer), before, "zero tokens moved");

    // Idempotent: disabling again does not panic.
    client.disable_auto_renewal(&consumer);
    assert!(!client.get_auto_renewal_record(&consumer).unwrap().active);

    // Re-granting revives it.
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    assert!(client.get_auto_renewal_record(&consumer).unwrap().active);
}

// ===========================================================================
// Configuration validation and amount clamping
// ===========================================================================

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn enable_auto_renewal_rejects_a_zero_period_cap() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    client.enable_auto_renewal(&consumer, &tok, &DURATION, &0i128, &3u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn enable_auto_renewal_rejects_a_zero_period_count() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    client.enable_auto_renewal(&consumer, &tok, &DURATION, &MAX_PER_PERIOD, &0u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn enable_auto_renewal_rejects_a_zero_duration() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    client.enable_auto_renewal(&consumer, &tok, &0u32, &MAX_PER_PERIOD, &3u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn enable_auto_renewal_rejects_more_periods_than_the_documented_bound() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    client.enable_auto_renewal(
        &consumer,
        &tok,
        &DURATION,
        &MAX_PER_PERIOD,
        &(crate::auto_renewal::MAX_PERIODS_AUTHORIZED + 1),
    );
}

#[test]
fn renewal_moves_min_of_plan_price_and_the_per_period_cap() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    // Cap below the plan price: the cap wins.
    let tight = 400i128;
    grant_renewal(&client, &consumer, &tok, tight, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);
    let a = client.try_auto_renew(&consumer);
    assert!(a.renewed);
    assert_eq!(a.amount, tight, "min(plan_amount, max_amount_per_period)");
    assert_eq!(before - balance(&e, &tok, &consumer), tight);
}

#[test]
fn the_authorization_amount_is_frozen_at_issue_time() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    let period_id = authorize_current(&client, &consumer, 1);
    let auth = client
        .get_renewal_authorization(&consumer, &period_id)
        .unwrap();
    assert_eq!(auth.amount, PLAN_AMOUNT);

    // Re-pricing the plan after the fact must not enlarge what the already
    // issued single-use authorization permits.
    client.set_subscription_price(&DURATION, &50_000);
    let before = balance(&e, &tok, &consumer);
    let a = client.try_auto_renew(&consumer);
    assert!(a.renewed);
    assert_eq!(
        a.amount, auth.amount,
        "the amount comes from the authorization"
    );
    assert_eq!(before - balance(&e, &tok, &consumer), auth.amount);
}

#[test]
fn a_renewal_without_an_active_subscription_is_refused() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let (client, _admin) = setup_contract(&e);
    let tok = deploy_token(&e);
    client.set_subscription_token(&tok);
    client.set_subscription_price(&DURATION, &PLAN_AMOUNT);
    let consumer = Address::generate(&e);
    mint_token(&e, &tok, &consumer, START_BALANCE);
    token::StellarAssetClient::new(&e, &tok).approve(
        &consumer,
        &client.address,
        &STANDING_ALLOWANCE,
        &APPROVAL_LEDGER,
    );

    // Authorized, but the consumer never subscribed.
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::NoActiveSubscription as u32);
    assert_eq!(balance(&e, &tok, &consumer), before);
}

#[test]
fn a_renewal_of_a_lapsed_subscription_is_refused() {
    let e = Env::default();
    let (client, consumer, tok) = setup_with_allowance(&e);
    grant_renewal(&client, &consumer, &tok, MAX_PER_PERIOD, 3);
    authorize_current(&client, &consumer, 1);
    let before = balance(&e, &tok, &consumer);

    // The subscription has run out.
    ledger_default(&e, 1_001, T0 + DURATION as u64 + 1);
    let a = client.try_auto_renew(&consumer);
    assert!(!a.renewed);
    assert_eq!(a.reason, EC::SubscriptionExpired as u32);
    assert_eq!(balance(&e, &tok, &consumer), before);
}
