#![cfg(test)]
//! #508 — Multisig signer-set rotation safety tests.
//!
//! Rotating signers is the operation most able to brick a multisig, and a
//! rotation bug is unrecoverable without a separate recovery path. These tests
//! are adversarial rather than happy-path: each one tries to use a rotation to
//! reach a state the multisig could never get out of.
//!
//! Invariants under test (each maps to an acceptance criterion):
//!
//! 1. No rotation can make the threshold unreachable.
//! 2. Approvals from removed signers do not count.
//! 3. A threshold change cannot pass a pending operation more cheaply.
//! 4. The recovery path remains reachable after any rotation.
//! 5. Every rotation is evented with the old and the new set.

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger, LedgerInfo, MockAuth, MockAuthInvoke},
    Address, Bytes, Env, IntoVal, String as SorobanString, Vec,
};

use crate::test_helpers::*;
use crate::PriceOracleContractClient;

/// Moves the test ledger forward so timelock windows can elapse.
fn advance_to(e: &Env, seq: u32, timestamp: u64) {
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
/// snake_case, so `MsGovernorsRotatedEvent` is `ms_governors_rotated_event`.
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

/// Deploys and initializes a contract with `n` governors at threshold
/// `required`, returning the client, the admin and the signer set.
fn with_governors(
    e: &Env,
    n: usize,
    required: u32,
) -> (PriceOracleContractClient<'_>, Address, Vec<Address>) {
    let (c, admin) = setup_contract_named(e, "rotation");
    let mut gov: Vec<Address> = Vec::new(e);
    for _ in 0..n {
        gov.push_back(Address::generate(e));
    }
    c.ms_set_governors(&gov, &required);
    (c, admin, gov)
}

fn setup_contract_named<'a>(e: &Env, name: &str) -> (PriceOracleContractClient<'a>, Address) {
    e.mock_all_auths();
    let contract_id = e.register(crate::PriceOracleContract, ());
    let c = PriceOracleContractClient::new(e, &contract_id);
    let admin = Address::generate(e);
    c.initialize(
        &admin,
        &1u32,
        &10u32,
        &18u32,
        &SorobanString::from_str(e, name),
    );
    (c, admin)
}

/// The approvals currently recorded on operation `op_id`.
fn approvals_of(c: &PriceOracleContractClient<'_>, op_id: u32) -> Vec<Address> {
    c.ms_get_operation(&op_id).approvals
}

fn empty_op(e: &Env) -> Bytes {
    Bytes::new(e)
}

// ---------------------------------------------------------------------------
// Invariant 1: the threshold is always reachable
// ---------------------------------------------------------------------------

#[test]
fn rotation_cannot_set_an_unreachable_threshold() {
    let e = Env::default();
    let (c, _admin, _gov) = with_governors(&e, 3, 2);

    // 3-of-5 asks for more signatures than the set can ever produce.
    let mut too_many: Vec<Address> = Vec::new(&e);
    for _ in 0..3 {
        too_many.push_back(Address::generate(&e));
    }
    assert!(c.try_ms_set_governors(&too_many, &4u32).is_err());

    // A zero threshold is a configuration error, not "no multisig".
    assert!(c.try_ms_set_governors(&too_many, &0u32).is_err());

    // The last valid configuration is untouched by the rejected attempts.
    assert_eq!(c.ms_get_required_approvals(), 2);
    assert_eq!(c.ms_get_governors().len(), 3);
}

/// Removing every signer would leave the threshold permanently unreachable, so
/// it must be rejected for the same reason an over-large threshold is.
#[test]
fn rotation_cannot_empty_the_signer_set() {
    let e = Env::default();
    let (c, _admin, _gov) = with_governors(&e, 3, 2);

    let empty: Vec<Address> = Vec::new(&e);
    // There is no legal `required` for an empty set: every value is either 0
    // or greater than the set size.
    for required in [0u32, 1, 2] {
        assert!(
            c.try_ms_set_governors(&empty, &required).is_err(),
            "emptying the signer set must be rejected (required={required})"
        );
    }
    assert_eq!(c.ms_get_governors().len(), 3);
}

/// After any accepted rotation, a quorum can still be assembled: the threshold
/// never exceeds the number of signers.
#[test]
fn quorum_stays_assemblable_after_each_rotation() {
    let e = Env::default();
    let (c, _admin, _signers) = with_governors(&e, 5, 3);
    let original = c.ms_get_governors();

    // A sequence of shrinking rotations, each keeping the threshold feasible.
    for size in [4usize, 3, 2] {
        let mut next: Vec<Address> = Vec::new(&e);
        for i in 0..size as u32 {
            next.push_back(original.get_unchecked(i));
        }
        c.ms_set_governors(&next, &1);

        let required = c.ms_get_required_approvals();
        assert!(
            required >= 1 && required <= c.ms_get_governors().len(),
            "threshold {required} unreachable with {} signers",
            c.ms_get_governors().len()
        );
    }
}

// ---------------------------------------------------------------------------
// Invariant 2: a departing signer's approvals do not count
// ---------------------------------------------------------------------------

#[test]
fn removed_signer_approvals_are_invalidated() {
    let e = Env::default();
    let (c, _admin, gov) = with_governors(&e, 3, 2);

    let op = c.ms_propose_operation(&gov.get_unchecked(0), &0u32, &empty_op(&e));
    c.ms_approve_operation(&gov.get_unchecked(0), &op);
    c.ms_approve_operation(&gov.get_unchecked(1), &op);
    assert_eq!(approvals_of(&c, op).len(), 2);

    // Governor 1 retires. Their approval must not keep counting.
    let mut remaining: Vec<Address> = Vec::new(&e);
    remaining.push_back(gov.get_unchecked(0));
    remaining.push_back(gov.get_unchecked(2));
    c.ms_set_governors(&remaining, &2);

    let after = approvals_of(&c, op);
    assert_eq!(after.len(), 1, "the departed signer's vote must be dropped");
    assert!(
        !after.contains(gov.get_unchecked(1)),
        "a removed signer must not remain among the approvers"
    );
    assert!(
        after.contains(gov.get_unchecked(0)),
        "a retained signer's vote must survive"
    );
}

#[test]
fn rotation_that_drops_quorum_resets_the_timelock() {
    let e = Env::default();
    let (c, _admin, gov) = with_governors(&e, 3, 2);

    // `timelock_start_ledger == 0` is the "not started" sentinel, and the
    // default test ledger sequence is 0, so advance first to make the sentinel
    // distinguishable from a started clock.
    advance_to(&e, 100, 1_000);

    let op = c.ms_propose_operation(&gov.get_unchecked(0), &0u32, &empty_op(&e));
    c.ms_approve_operation(&gov.get_unchecked(0), &op);
    c.ms_approve_operation(&gov.get_unchecked(1), &op);
    // Quorum reached, so the timelock clock has started.
    assert_eq!(c.ms_get_operation(&op).timelock_start_ledger, 100);

    // Remove governor 1, dropping the operation below its 2-signature quorum.
    // The timelock must be reset: an operation that lost quorum cannot sit on
    // a running clock and execute later on the departed signer's authority.
    let mut remaining: Vec<Address> = Vec::new(&e);
    remaining.push_back(gov.get_unchecked(0));
    remaining.push_back(gov.get_unchecked(2));
    c.ms_set_governors(&remaining, &2);

    assert_eq!(
        c.ms_get_operation(&op).timelock_start_ledger,
        0,
        "losing quorum must reset the timelock"
    );
}

#[test]
fn operation_that_lost_quorum_cannot_execute() {
    let e = Env::default();
    let (c, _admin, gov) = with_governors(&e, 3, 2);

    let op = c.ms_propose_operation(&gov.get_unchecked(0), &0u32, &empty_op(&e));
    c.ms_approve_operation(&gov.get_unchecked(0), &op);
    c.ms_approve_operation(&gov.get_unchecked(1), &op);

    // One of the two approvers is removed, dropping the operation below quorum.
    let mut remaining: Vec<Address> = Vec::new(&e);
    remaining.push_back(gov.get_unchecked(0));
    remaining.push_back(gov.get_unchecked(2));
    c.ms_set_governors(&remaining, &2);

    // Advance well past the timelock window. Even so, the operation cannot be
    // executed: quorum is judged on the *current* approvers, not on the clock.
    advance_to(&e, 1_000, 10_000);
    assert!(
        c.try_ms_execute_operation(&gov.get_unchecked(0), &op)
            .is_err(),
        "an operation that lost quorum must not execute on the old timelock"
    );
}

#[test]
fn a_removed_signer_cannot_approve_propose_execute_or_cancel() {
    let e = Env::default();
    let (c, _admin, gov) = with_governors(&e, 3, 2);

    let op = c.ms_propose_operation(&gov.get_unchecked(0), &0u32, &empty_op(&e));
    c.ms_approve_operation(&gov.get_unchecked(0), &op);

    let mut remaining: Vec<Address> = Vec::new(&e);
    remaining.push_back(gov.get_unchecked(0));
    remaining.push_back(gov.get_unchecked(2));
    c.ms_set_governors(&remaining, &2);

    // The departed signer retains no authority over the multisig.
    assert!(c
        .try_ms_approve_operation(&gov.get_unchecked(1), &op)
        .is_err());
    assert!(c
        .try_ms_execute_operation(&gov.get_unchecked(1), &op)
        .is_err());
    assert!(c
        .try_ms_propose_operation(&gov.get_unchecked(1), &0u32, &empty_op(&e))
        .is_err());
    assert!(c
        .try_ms_cancel_operation(&gov.get_unchecked(1), &op)
        .is_err());
}

// ---------------------------------------------------------------------------
// Invariant 3: a threshold change cannot pass a pending operation more cheaply
// ---------------------------------------------------------------------------

#[test]
fn lowering_the_threshold_does_not_weaken_a_pending_operation() {
    let e = Env::default();
    let (c, _admin, gov) = with_governors(&e, 3, 3);

    // Proposed under a 3-of-3 threshold: all three signatures are required.
    let op = c.ms_propose_operation(&gov.get_unchecked(0), &0u32, &empty_op(&e));
    c.ms_approve_operation(&gov.get_unchecked(0), &op);
    c.ms_approve_operation(&gov.get_unchecked(1), &op);

    // Now weaken the global threshold to 1-of-3.
    c.ms_set_governors(&gov, &1);

    // The pending operation still carries its own 3-signature requirement.
    assert_eq!(
        c.ms_get_operation(&op).required_approvals,
        3,
        "a pending operation must keep the threshold it was proposed under"
    );
    assert!(
        c.try_ms_execute_operation(&gov.get_unchecked(0), &op)
            .is_err(),
        "lowering the global threshold must not let a pending operation through \
         with fewer signatures than its own quorum"
    );

    // The third signature still completes it.
    c.ms_approve_operation(&gov.get_unchecked(2), &op);
    assert_eq!(approvals_of(&c, op).len(), 3);
}

#[test]
fn raising_the_threshold_does_not_strand_a_reached_operation() {
    let e = Env::default();
    let (c, _admin, gov) = with_governors(&e, 3, 1);

    let op = c.ms_propose_operation(&gov.get_unchecked(0), &0u32, &empty_op(&e));
    c.ms_approve_operation(&gov.get_unchecked(0), &op);

    // Raising the threshold afterwards must not retroactively demand more
    // signatures of an operation that already reached its own quorum.
    c.ms_set_governors(&gov, &3);
    assert_eq!(c.ms_get_operation(&op).required_approvals, 1);
}

// ---------------------------------------------------------------------------
// Invariant 4: the recovery path stays reachable
// ---------------------------------------------------------------------------

#[test]
fn recovery_path_is_reachable_after_any_rotation() {
    let e = Env::default();
    let (c, _admin) = setup_contract_named(&e, "recovery");

    // Guardians are an authority independent of the governor set.
    let g1 = Address::generate(&e);
    let g2 = Address::generate(&e);
    let mut guardians = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    c.recovery_set_guardians(&guardians, &2);

    // Rotate the signer set hard: down to a single signer.
    let sole = Address::generate(&e);
    let mut gov = Vec::new(&e);
    gov.push_back(sole.clone());
    c.ms_set_governors(&gov, &1);

    // `ready_ledger == 0` is the "quorum not yet reached" sentinel, so the
    // guardian quorum must be reached at a non-zero sequence. Advance first:
    // the point of this test is that recovery survives a signer rotation, not
    // anything about the default delay, so a short window is set explicitly.
    let delay = 5u32;
    c.recovery_set_delay(&delay);
    advance_to(&e, 50, 1_000);

    // Recovery still works: guardians replace the admin key even though the
    // governor set they do not control was just rewritten.
    let new_admin = Address::generate(&e);
    c.recovery_approve(&g1, &new_admin);
    c.recovery_approve(&g2, &new_admin);

    // The cancellation window is denominated in *ledgers* and starts when the
    // guardian quorum is reached, so advance the sequence past it.
    advance_to(&e, 50 + delay + 1, 2_000);
    c.recovery_execute();

    #[test]
    fn rotation_cannot_lock_out_the_admin() {
        let e = Env::default();
        let (c, admin) = setup_contract_named(&e, "lockout");

        // The admin is not a signer and does not need to be: it can still rotate
        // and still execute as an authority.
        let sole = Address::generate(&e);
        let mut gov = Vec::new(&e);
        gov.push_back(sole.clone());
        c.ms_set_governors(&gov, &1);

        assert_eq!(c.get_admin_address(), admin);
        let op = c.ms_propose_operation(&sole, &0u32, &empty_op(&e));
        c.ms_approve_operation(&sole, &op);
        advance_to(&e, 100, 1_000);
        assert!(c.try_ms_execute_operation(&admin, &op).is_ok());
    }

    // ---------------------------------------------------------------------------
    // Invariant 5: rotations are evented with both sets
    // ---------------------------------------------------------------------------

    #[test]
    fn rotation_is_evented() {
        let e = Env::default();
        let (_c, _admin, _gov) = with_governors(&e, 3, 2);
        assert!(
            has_event(&e, "ms_governors_updated_event"),
            "a rotation must be evented"
        );
    }

    #[test]
    fn rotation_emits_an_event_carrying_the_old_and_new_sets() {
        let e = Env::default();
        let (c, _admin, _gov) = with_governors(&e, 3, 2);

        // Capture the event count before the rotation so only new events are
        // inspected.
        let before = e.events().all().events().len();
        let kept = c.ms_get_governors();
        let mut next = Vec::new(&e);
        next.push_back(kept.get_unchecked(0));
        next.push_back(kept.get_unchecked(2));
        c.ms_set_governors(&next, &1);

        let all = e.events().all().events().to_vec();
        assert!(
            all.len() > before,
            "the rotation must emit at least one event"
        );

        let saw_rotation = all
            .iter()
            .skip(before)
            .any(|ev| is_event(ev, "ms_governors_rotated_event"));
        assert!(
            saw_rotation,
            "the signer-set rotation must emit ms_governors_rotated_event, which \
         carries the old and the new signer sets so the signer history is \
         reconstructible from the event stream alone"
        );
    }

    // ---------------------------------------------------------------------------
    // The rotation entrypoint itself is admin-gated
    // ---------------------------------------------------------------------------

    #[test]
    fn rotation_requires_admin_authority() {
        let e = Env::default();
        let (c, _admin, _gov) = with_governors(&e, 3, 2);

        // Only the admin's authorization satisfies the rotation guard. Mocking
        // auth for somebody else must leave the call unauthorized.
        let impostor = Address::generate(&e);
        let mut next = Vec::new(&e);
        next.push_back(impostor.clone());

        e.mock_auths(&[MockAuth {
            address: &impostor,
            invoke: &MockAuthInvoke {
                contract: &c.address,
                fn_name: "ms_set_governors",
                args: (next.clone(), 1u32).into_val(&e),
                sub_invokes: &[],
            },
        }]);

        assert!(
            c.try_ms_set_governors(&next, &1).is_err(),
            "a non-admin must not be able to rewrite the signer set"
        );
        assert_eq!(c.ms_get_governors().len(), 3);
    }

    assert_eq!(c.get_admin_address(), new_admin);
    assert_eq!(
        c.ms_get_governors().len(),
        1,
        "recovery replaces the admin, not the signer set"
    );
}
