//! Tests for the #481 hysteresis circuit breaker.
//!
//! The properties under test: no chatter around the trip threshold, re-arm only
//! after the settle condition, manual override never blocked, and a bounded open
//! duration with escalation.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Ledger, LedgerInfo};
use soroban_sdk::{Address, Env, String};

use crate::test_helpers::register_test_asset;
use crate::{BreakerPolicy, PriceOracleContractClient};

/// Moves the ledger forward by `n`, advancing the timestamp with it.
fn advance(e: &Env, n: u32) {
    let seq = e.ledger().sequence();
    let ts = e.ledger().timestamp();
    e.ledger().set(LedgerInfo {
        timestamp: ts + u64::from(n) * 5,
        sequence_number: seq + n,
        protocol_version: 26,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
}

/// Deploys a contract with one registered asset; returns client, asset and the
/// contract's own address (needed to write storage in the contract's scope).
fn setup<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address, Address) {
    e.mock_all_auths();
    let contract_id = e.register(crate::PriceOracleContract, ());
    let client = crate::PriceOracleContractClient::new(e, &contract_id);
    let admin = Address::generate(e);
    client.initialize(
        &admin,
        &2u32,
        &10u32,
        &18u32,
        &soroban_sdk::String::from_str(e, "test"),
    );
    e.ledger().with_mut(|l| l.timestamp = 1000);
    let asset = register_test_asset(e, &client);
    (client, asset, contract_id)
}

fn policy(trip: u32, clear: u32, settle: u32, max_open: u32) -> BreakerPolicy {
    BreakerPolicy {
        trip_bps: trip,
        clear_bps: clear,
        settle_ledgers: settle,
        max_open_ledgers: max_open,
        auto_rearm: true,
    }
}

/// Opens the breaker the way the aggregation path does: the asset-level trip
/// records the open state, and the breaker module records the armed ledger and
/// resets the settle streak.
fn trip(e: &Env, contract: &Address, asset: &Address, deviation: u32) {
    // Both writes happen inside the contract's own storage scope, which is what
    // the aggregation path has and a bare test call does not.
    e.as_contract(contract, || {
        crate::assets::trip_circuit_breaker(
            e,
            asset.clone(),
            100,
            200,
            i128::from(deviation),
            1_000,
        );
        crate::breaker::record_trip(e, asset, deviation);
    });
}

/// A deadband with no gap is rejected: it would reproduce the chattering breaker.
#[test]
fn deadband_must_be_strict() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);

    // clear == trip: no deadband at all.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_breaker_policy(&asset, &policy(1_000, 1_000, 5, 100))
    }));
    assert!(result.is_err(), "equal thresholds leave no deadband");

    // clear > trip: inverted.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_breaker_policy(&asset, &policy(1_000, 2_000, 5, 100))
    }));
    assert!(
        result.is_err(),
        "the clear threshold must sit below the trip"
    );
}

/// A zero settle window or escalation bound is rejected as vacuous.
#[test]
fn zero_settle_and_open_bounds_are_rejected() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_breaker_policy(&asset, &policy(1_000, 500, 0, 100))
    }));
    assert!(result.is_err(), "a zero settle window is vacuous");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_breaker_policy(&asset, &policy(1_000, 500, 5, 0))
    }));
    assert!(result.is_err(), "a zero escalation bound is vacuous");
}

/// A valid policy is stored and readable.
#[test]
fn policy_round_trips() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    let p = policy(2_000, 1_000, 10, 500);
    client.set_breaker_policy(&asset, &p);
    assert_eq!(client.get_breaker_policy(&asset), p);
}

/// The core anti-chatter property: a deviation oscillating across the trip
/// threshold never re-arms and never flaps.
#[test]
fn oscillation_around_the_trip_threshold_does_not_chatter() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 5, 10_000));

    trip(&e, &contract, &asset, 2_500);
    assert!(client.get_breaker_status(&asset).is_open);

    // Alternate below and above the *clear* threshold for many ledgers. The
    // breaker must stay open the whole time: the settle streak resets every
    // time the deviation climbs back, so it never reaches the required run.
    for i in 0..40u32 {
        advance(&e, 1);
        let deviation = if i % 2 == 0 { 400 } else { 1_800 };
        let rearmed = client.evaluate_breaker_rearm(&asset, &deviation);
        assert!(!rearmed, "the breaker chattered on iteration {}", i);
        assert!(
            client.get_breaker_status(&asset).is_open,
            "the breaker closed on iteration {}",
            i
        );
    }
}

/// A deviation inside the deadband does not re-arm either.
#[test]
fn deviation_inside_the_deadband_does_not_rearm() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 3, 10_000));
    trip(&e, &contract, &asset, 2_500);

    // 1 500 bps is below the trip threshold but above the clear threshold: the
    // market has neither recovered nor stayed broken.
    for _ in 0..20 {
        advance(&e, 1);
        assert!(!client.evaluate_breaker_rearm(&asset, &1_500));
    }
    assert!(client.get_breaker_status(&asset).is_open);
}

/// Re-arm happens only after the deviation stays below the clear threshold for
/// the full settle window.
#[test]
fn rearm_requires_the_whole_settle_window() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 5, 10_000));
    trip(&e, &contract, &asset, 2_500);

    // Four settled ledgers is one short of the required five.
    for i in 0..4u32 {
        advance(&e, 1);
        assert!(
            !client.evaluate_breaker_rearm(&asset, &100),
            "re-armed early on settled ledger {}",
            i
        );
        assert!(client.get_breaker_status(&asset).is_open);
    }

    // The fifth completes the window.
    advance(&e, 1);
    assert!(client.evaluate_breaker_rearm(&asset, &100));
    assert!(
        !client.get_breaker_status(&asset).is_open,
        "the breaker stayed open"
    );
}

/// A single calm observation followed by a spike does not re-arm: the streak
/// must be consecutive.
#[test]
fn a_single_calm_observation_does_not_rearm() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 5, 10_000));
    trip(&e, &contract, &asset, 2_500);

    // Build a long, nearly-complete streak.
    for _ in 0..4 {
        advance(&e, 1);
        assert!(!client.evaluate_breaker_rearm(&asset, &100));
    }
    // One spike resets it.
    advance(&e, 1);
    assert!(!client.evaluate_breaker_rearm(&asset, &1_900));
    assert_eq!(client.get_breaker_status(&asset).settle_ledgers, 0);

    // Restarting from zero, four more ledgers are still not enough.
    for _ in 0..4 {
        advance(&e, 1);
        assert!(!client.evaluate_breaker_rearm(&asset, &100));
    }
    assert!(client.get_breaker_status(&asset).is_open);
}

/// Manual override works regardless of the settle streak, and regardless of the
/// automatic path being disabled entirely.
#[test]
fn manual_override_is_never_blocked() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);

    // Automatic re-arm switched off.
    let mut p = policy(2_000, 1_000, 5, 10_000);
    p.auto_rearm = false;
    client.set_breaker_policy(&asset, &p);
    trip(&e, &contract, &asset, 2_500);

    for _ in 0..20 {
        advance(&e, 1);
        assert!(!client.evaluate_breaker_rearm(&asset, &0));
    }
    assert!(client.get_breaker_status(&asset).is_open);

    // The operator can still clear it.
    client.clear_breaker(&asset);
    assert!(!client.get_breaker_status(&asset).is_open);
}

/// Manual override works even after escalation, which only disables automation.
#[test]
fn manual_override_works_after_escalation() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 5, 10));
    trip(&e, &contract, &asset, 2_500);

    // Past the bound the automatic path escalates and refuses.
    advance(&e, 20);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.evaluate_breaker_rearm(&asset, &0)
    }));
    assert!(result.is_err(), "the open duration must be bounded");
    assert!(client.get_breaker_status(&asset).is_open);

    // The operator is not blocked by the escalation.
    client.clear_breaker(&asset);
    assert!(!client.get_breaker_status(&asset).is_open);
}

/// The open duration is bounded, and escalation is what happens at the bound.
#[test]
fn open_duration_is_bounded_with_escalation() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 5, 10));
    trip(&e, &contract, &asset, 2_500);

    // Within the bound, evaluation proceeds normally.
    advance(&e, 5);
    assert!(!client.evaluate_breaker_rearm(&asset, &1_500));
    assert!(!client.get_breaker_status(&asset).escalated);

    // Past it, the contract escalates rather than re-arming.
    advance(&e, 20);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.evaluate_breaker_rearm(&asset, &0)
    }));
    assert!(result.is_err());
    let status = client.get_breaker_status(&asset);
    assert!(status.escalated, "the bound must escalate");
    assert!(status.open_ledgers > 10);
}

/// A re-armed breaker accepts submissions again.
#[test]
fn rearmed_breaker_accepts_submissions_again() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_min_sources_required(&1u32);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 3, 10_000));

    let source = Address::generate(&e);
    client.add_source(&source, &String::from_str(&e, "Source"));
    client.submit_price(&source, &asset, &100i128, &1000u64);

    trip(&e, &contract, &asset, 2_500);
    // The breaker refuses submissions while open.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.submit_price(&source, &asset, &200i128, &1000u64)
    }));
    assert!(result.is_err(), "an open breaker must refuse submissions");

    for _ in 0..3 {
        advance(&e, 1);
        client.evaluate_breaker_rearm(&asset, &0);
    }
    assert!(!client.get_breaker_status(&asset).is_open);

    client.submit_price(&source, &asset, &200i128, &e.ledger().timestamp());
}

/// An armed breaker's re-arm evaluation is a no-op.
#[test]
fn evaluation_on_an_armed_breaker_is_a_noop() {
    let e = Env::default();
    let (client, asset, contract) = setup(&e);
    client.set_breaker_policy(&asset, &policy(2_000, 1_000, 5, 10_000));
    assert!(!client.evaluate_breaker_rearm(&asset, &0));
    assert!(!client.get_breaker_status(&asset).is_open);
}
