#![cfg(test)]
//! #510 — Dead-man switch tests.
//!
//! The switch fails toward safety when the operator team's heartbeat stops.
//! These tests assert the acceptance criteria directly: the trip happens on
//! schedule, heartbeats cannot be spoofed, a warning always precedes the
//! trigger, recovery works through an authority independent of the admin, and
//! the degraded state serves no unsafely-fresh value.

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger, LedgerInfo, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal, Vec,
};

use crate::test_helpers::*;
use crate::types::Asset;
use crate::PriceOracleContractClient;

/// Deploys a contract with `min_sources = 1`, one registered source and one
/// registered asset with a published aggregate — so the read path has something
/// to serve before the switch trips.
fn serving(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address, Address) {
    let (c, admin) = setup_contract(e);
    c.set_min_sources_required(&1u32);
    let source = register_test_source(e, &c, "src");
    let asset = register_test_asset(e, &c);
    submit_test_price(&c, &source, &asset, 1_000, 1);
    advance(e, 1, 1);
    (c, admin, source, asset)
}

fn advance(e: &Env, seq: u32, timestamp: u64) {
    e.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
}

/// `true` if the event's first topic is the given symbol.
///
/// `#[contractevent]` derives its topic symbol from the struct name in
/// snake_case, so `DeadManTriggeredEvent` is `dead_man_triggered_event`.
fn is_event(ev: &soroban_sdk::xdr::ContractEvent, name: &str) -> bool {
    use soroban_sdk::xdr::{ContractEventBody, ScVal};
    use std::string::ToString;
    match &ev.body {
        ContractEventBody::V0(v0) => match v0.topics.first() {
            Some(ScVal::Symbol(sym)) => sym.0.to_string() == name,
            _ => false,
        },
    }
}

fn has_event(e: &Env, name: &str) -> bool {
    e.events()
        .all()
        .events()
        .iter()
        .any(|ev| is_event(ev, name))
}

fn count_events(e: &Env, name: &str) -> usize {
    e.events()
        .all()
        .events()
        .iter()
        .filter(|ev| is_event(ev, name))
        .count()
}

/// Arms the switch with the given trigger/warning seconds and `operator` as the
/// only registered operator.
fn arm(e: &Env, c: &PriceOracleContractClient<'_>, operator: &Address, trigger: u64, warn: u64) {
    let mut ops = Vec::new(e);
    ops.push_back(operator.clone());
    c.dead_man_configure(&trigger, &warn, &ops);
}

// ---------------------------------------------------------------------------
// The switch is off until configured
// ---------------------------------------------------------------------------

#[test]
fn switch_is_disabled_by_default() {
    let e = Env::default();
    let (c, _admin, _s, _a) = serving(&e);

    assert!(!c.dead_man_is_degraded());
    // With no configuration, evaluation is a no-op however far time advances.
    advance(&e, 1_000, 10_000_000);
    assert_eq!(c.dead_man_evaluate(), 0);
    assert!(!c.dead_man_is_degraded());
    assert_eq!(c.dead_man_get_config().trigger_after, 0);
}

#[test]
fn heartbeat_is_rejected_while_the_switch_is_disabled() {
    let e = Env::default();
    let (c, _admin, _s, _a) = serving(&e);
    let op = Address::generate(&e);
    assert!(c.try_dead_man_heartbeat(&op).is_err());
}

// ---------------------------------------------------------------------------
// Configuration is validated
// ---------------------------------------------------------------------------

#[test]
fn configuration_requires_a_warning_before_the_trigger() {
    let e = Env::default();
    let (c, _admin, _s, _a) = serving(&e);
    let op = Address::generate(&e);
    let mut ops = Vec::new(&e);
    ops.push_back(op.clone());

    // A warning that fires at or after the trigger would never be seen before
    // the trip, so it is rejected.
    assert!(c.try_dead_man_configure(&1000u64, &1000u64, &ops).is_err());
    assert!(c.try_dead_man_configure(&1000u64, &2000u64, &ops).is_err());
    assert!(c.try_dead_man_configure(&1000u64, &999u64, &ops).is_ok());
}

#[test]
fn arming_requires_at_least_one_operator() {
    let e = Env::default();
    let (c, _admin, _s, _a) = serving(&e);
    let empty = Vec::new(&e);
    // An armed switch nobody can heartbeat would trip immediately and could
    // never be kept alive, so it is refused.
    assert!(c.try_dead_man_configure(&1000u64, &500u64, &empty).is_err());
}

#[test]
fn configuration_requires_admin_authority() {
    let e = Env::default();
    let (c, _admin, _s, _a) = serving(&e);
    let op = Address::generate(&e);
    let mut ops = Vec::new(&e);
    ops.push_back(op.clone());

    // ---------------------------------------------------------------------------
    // Acceptance: missing heartbeats beyond the interval enters the degraded state
    // ---------------------------------------------------------------------------

    #[test]
    fn missing_heartbeat_beyond_the_interval_degrades_the_contract() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let op = Address::generate(&e);
        arm(&e, &c, &op, 1_000, 500);
        c.dead_man_heartbeat(&op);

        // One second short of the trigger: still serving.
        advance(&e, 10, 999);
        assert_eq!(c.dead_man_evaluate(), 0);
        assert!(!c.dead_man_is_degraded());

        // One second past it: degraded.
        advance(&e, 11, 1_000);
        assert_eq!(c.dead_man_evaluate(), 1);
        assert!(c.dead_man_is_degraded());
    }

    #[test]
    fn a_fresh_heartbeat_keeps_the_contract_operational() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let op = Address::generate(&e);
        arm(&e, &c, &op, 1_000, 500);

        advance(&e, 10, 900);
        c.dead_man_heartbeat(&op);

        // Long after arming, the heartbeat restarted the clock.
        advance(&e, 20, 1_800);
        assert_eq!(c.dead_man_evaluate(), 0);
        assert!(!c.dead_man_is_degraded());
    }

    #[test]
    fn evaluation_is_idempotent_and_emits_one_trigger() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let op = Address::generate(&e);
        arm(&e, &c, &op, 100, 50);

        advance(&e, 10, 1_000);
        assert_eq!(c.dead_man_evaluate(), 1);
        let first = count_events(&e, "dead_man_triggered_event");

        // Repeated evaluation by any keeper must not re-trip or re-announce.
        for seq in 11..15 {
            advance(&e, seq, 2_000);
            assert_eq!(c.dead_man_evaluate(), 1);
        }
        assert_eq!(
            first,
            count_events(&e, "dead_man_triggered_event"),
            "re-evaluating an already-degraded contract must not re-emit the trigger"
        );
    }

    // ---------------------------------------------------------------------------
    // Acceptance: heartbeats cannot be spoofed
    // ---------------------------------------------------------------------------

    #[test]
    fn a_non_operator_cannot_heartbeat() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let operator = Address::generate(&e);
        let stranger = Address::generate(&e);
        arm(&e, &c, &operator, 1_000, 500);

        // The stranger authorizes the call themselves, but is not in the operator
        // set: membership and authorization are both required.
        assert!(c.try_dead_man_heartbeat(&stranger).is_err());
    }

    #[test]
    fn an_operator_cannot_heartbeat_on_another_operators_behalf() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let operator = Address::generate(&e);
        let other = Address::generate(&e);
        arm(&e, &c, &operator, 1_000, 500);

        advance(&e, 5, 100);
        // `operator` authorizes a call that names `other` as the sender. The guard
        // requires the *named* address to authorize, so this must fail: a stolen
        // operator key cannot keep the contract alive indefinitely by heartbeating
        // as a peer.
        e.mock_auths(&[MockAuth {
            address: &operator,
            invoke: &MockAuthInvoke {
                contract: &c.address,
                fn_name: "dead_man_heartbeat",
                args: (other.clone(),).into_val(&e),
                sub_invokes: &[],
            },
        }]);
        assert!(c.try_dead_man_heartbeat(&other).is_err());
    }

    // ---------------------------------------------------------------------------
    // Acceptance: warnings are emitted before triggering
    // ---------------------------------------------------------------------------

    #[test]
    fn a_warning_precedes_the_trigger() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let op = Address::generate(&e);
        arm(&e, &c, &op, 1_000, 500);
        c.dead_man_heartbeat(&op);

        // Inside the warning window: nothing is announced yet.
        advance(&e, 5, 400);
        assert_eq!(c.dead_man_evaluate(), 0);
        assert!(!has_event(&e, "dead_man_warning_event"));

        // Warning window reached: the warning is emitted, but the switch has not
        // tripped, so operators still have a window in which to heartbeat.
        advance(&e, 6, 500);
        assert_eq!(c.dead_man_evaluate(), 0);
        assert!(has_event(&e, "dead_man_warning_event"));
        assert!(
            !has_event(&e, "dead_man_triggered_event"),
            "the warning must arrive before the trigger"
        );
        assert!(!c.dead_man_is_degraded());

        // A heartbeat inside the warning window averts the trip entirely.
        c.dead_man_heartbeat(&op);
        advance(&e, 7, 900);
        assert_eq!(c.dead_man_evaluate(), 0);
        assert!(!c.dead_man_is_degraded());
    }

    #[test]
    fn the_warning_is_emitted_once_per_lapse() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let op = Address::generate(&e);
        arm(&e, &c, &op, 10_000, 500);
        c.dead_man_heartbeat(&op);

        for seq in 5..12 {
            advance(&e, seq, 500 + (seq - 5) as u64 * 100);
            assert_eq!(c.dead_man_evaluate(), 0);
        }
        assert_eq!(
            count_events(&e, "dead_man_warning_event"),
            1,
            "the warning is a latch, not a per-ledger event"
        );
    }

    #[test]
    fn a_trip_emits_both_the_warning_and_the_trigger() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let op = Address::generate(&e);
        arm(&e, &c, &op, 1_000, 500);

        // ---------------------------------------------------------------------------
        // Acceptance: the degraded state serves no unsafely-fresh value
        // ---------------------------------------------------------------------------

        #[test]
        fn the_degraded_state_serves_no_price() {
            let e = Env::default();
            let (c, _admin, _s, asset) = serving(&e);
            let op = Address::generate(&e);
            arm(&e, &c, &op, 100, 50);

            // Before the trip the read path serves the published aggregate.
            advance(&e, 5, 10);
            assert!(
                c.prices(&Asset::Stellar(asset.clone()), &1u32).is_some(),
                "the read path should serve a price before the switch trips"
            );

            // After the trip it serves nothing at all: a stored value handed out now
            // would look live while nobody is attesting to its freshness.
            advance(&e, 10, 1_000);
            assert_eq!(c.dead_man_evaluate(), 1);
            assert!(
                c.prices(&Asset::Stellar(asset), &1u32).is_none(),
                "the degraded state must serve no price at all"
            );
        }

        #[test]
        fn the_degraded_state_rejects_submissions() {
            let e = Env::default();
            let (c, _admin, source, asset) = serving(&e);
            let op = Address::generate(&e);
            arm(&e, &c, &op, 100, 50);

            advance(&e, 10, 1_000);
            assert_eq!(c.dead_man_evaluate(), 1);

            assert!(
                c.try_submit_price(&source, &asset, &2_000i128, &2u64)
                    .is_err(),
                "no new aggregate may be produced while nobody is watching"
            );
        }

        #[test]
        fn non_price_reads_still_work_while_degraded() {
            let e = Env::default();
            let (c, _admin, _s, _a) = serving(&e);
            let op = Address::generate(&e);
            arm(&e, &c, &op, 100, 50);

            advance(&e, 10, 1_000);
            assert_eq!(c.dead_man_evaluate(), 1);

            // The degraded state must stay *observable* and reversible: configuration
            // and the switch's own state remain readable.
            assert_eq!(c.dead_man_get_config().trigger_after, 100);
            assert!(c.dead_man_is_degraded());
            assert_eq!(c.get_min_sources_required(), 1);
        }

        // ---------------------------------------------------------------------------
        // Acceptance: recovery is possible via an independent authority
        // ---------------------------------------------------------------------------

        /// Trips the switch and returns a contract in the degraded state, with
        /// `guardian` registered as a recovery guardian.
        fn degraded_with_guardian(e: &Env) -> (PriceOracleContractClient<'_>, Address) {
            let (c, admin) = setup_contract(e);
            c.set_min_sources_required(&1u32);

            let guardian = Address::generate(e);
            let mut guardians = Vec::new(e);
            guardians.push_back(guardian.clone());
            c.recovery_set_guardians(&guardians, &1);

            let op = Address::generate(e);
            arm(&e, &c, &op, 100, 50);

            advance(e, 10, 1_000);
            assert_eq!(c.dead_man_evaluate(), 1);
            assert!(c.dead_man_is_degraded());
            let _ = admin;
            (c, guardian)
        }

        #[test]
        fn a_recovery_guardian_can_clear_the_degraded_state() {
            let e = Env::default();
            let (c, guardian) = degraded_with_guardian(&e);

            // The guardian is not the admin and holds no operator key: recovery does
            // not depend on the authority whose loss the switch is detecting.
            c.dead_man_recover(&guardian);

            assert!(!c.dead_man_is_degraded());
            assert!(has_event(&e, "dead_man_recovered_event"));
        }

        #[test]
        fn a_stranger_cannot_clear_the_degraded_state() {
            let e = Env::default();
            let (c, _guardian) = degraded_with_guardian(&e);
            let stranger = Address::generate(&e);

            assert!(c.try_dead_man_recover(&stranger).is_err());
            assert!(
                c.dead_man_is_degraded(),
                "an unauthorized caller must not clear the degraded state"
            );
        }

        #[test]
        fn recovery_resumes_serving() {
            let e = Env::default();
            let (c, _admin, _source, asset) = serving(&e);
            let guardian = Address::generate(&e);
            let mut guardians = Vec::new(&e);
            guardians.push_back(guardian.clone());
            c.recovery_set_guardians(&guardians, &1);

            let op = Address::generate(&e);
            arm(&e, &c, &op, 100, 50);
            advance(&e, 10, 1_000);
            assert_eq!(c.dead_man_evaluate(), 1);
            assert!(c.prices(&Asset::Stellar(asset.clone()), &1u32).is_none());

            c.dead_man_recover(&guardian);

            // Reads serve again, and so do submissions.
            advance(&e, 11, 1_100);
            assert!(c.prices(&Asset::Stellar(asset), &1u32).is_some());
        }

        #[test]
        fn recovery_when_not_degraded_is_rejected() {
            let e = Env::default();
            let (c, admin, _s, _a) = serving(&e);
            let guardian = Address::generate(&e);
            let mut guardians = Vec::new(&e);
            guardians.push_back(guardian.clone());
            c.recovery_set_guardians(&guardians, &1);

            // Nothing to recover from: a no-op recovery would be a free state reset.
            assert!(c.try_dead_man_recover(&guardian).is_err());
            assert!(c.try_dead_man_recover(&admin).is_err());
        }

        #[test]
        fn a_heartbeat_does_not_clear_the_degraded_state() {
            let e = Env::default();
            let (c, _admin, _s, _a) = serving(&e);
            let op = Address::generate(&e);
            arm(&e, &c, &op, 100, 50);

            advance(&e, 10, 1_000);
            assert_eq!(c.dead_man_evaluate(), 1);

            // Beating must not silently undo the trip: that would make the switch
            // reversible by whoever still holds an operator key, which is exactly the
            // scenario it exists to catch.
            c.dead_man_heartbeat(&op);
            advance(&e, 11, 1_010);
            assert_eq!(c.dead_man_evaluate(), 1);
            assert!(c.dead_man_is_degraded());
        }

        // Nobody ever heartbeats, so evaluation goes straight past the warning
        // window into the trip. Both events must be present: monitoring that only
        // watched for the trigger would have had no warning to act on.
        advance(&e, 5, 5_000);
        assert_eq!(c.dead_man_evaluate(), 1);
        assert!(has_event(&e, "dead_man_warning_event"));
        assert!(has_event(&e, "dead_man_triggered_event"));
    }

    #[test]
    fn a_spoofed_heartbeat_does_not_postpone_the_trip() {
        let e = Env::default();
        let (c, _admin, _s, _a) = serving(&e);
        let operator = Address::generate(&e);
        let stranger = Address::generate(&e);
        arm(&e, &c, &operator, 1_000, 500);

        // The stranger tries and fails to heartbeat throughout.
        for seq in 1..5 {
            advance(&e, seq, seq as u64 * 400);
            assert!(c.try_dead_man_heartbeat(&stranger).is_err());
            assert_eq!(c.dead_man_evaluate(), 0);
        }

        // The real operator never beats. The switch trips on schedule.
        advance(&e, 10, 1_000);
        assert_eq!(c.dead_man_evaluate(), 1);
    }

    // Only the admin may arm the switch.
    e.mock_auths(&[MockAuth {
        address: &op,
        invoke: &MockAuthInvoke {
            contract: &c.address,
            fn_name: "dead_man_configure",
            args: (1000u64, 500u64, ops.clone()).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    assert!(c.try_dead_man_configure(&1000u64, &500u64, &ops).is_err());
    assert_eq!(c.dead_man_get_config().trigger_after, 0);
}
