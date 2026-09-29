//! # #454 — Admin key compromise: blast radius and recovery drill
//!
//! Proves a compromised admin key cannot block its own revocation through
//! the guardian recovery path, and scripts the recovery drill end to end.
//! The capability list and residual risk are documented in
//! `docs/security/admin-compromise-drill.md`.

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String, Vec,
};

use crate::test_helpers::*;
use crate::types::ErrorCode;
use crate::PriceOracleContractClient;

const DELAY: u32 = 100;

struct Drill<'a> {
    client: PriceOracleContractClient<'a>,
    attacker_admin: Address,
    g1: Address,
    g2: Address,
    rescue: Address,
}

fn advance(e: &Env, ledgers: u32) {
    let seq = e.ledger().sequence();
    e.ledger().with_mut(|l| l.sequence_number = seq + ledgers);
}

/// Guardians 2-of-2 and a delay are configured while the admin is honest; the
/// admin key is then considered compromised.
fn setup(e: &Env) -> Drill<'_> {
    e.mock_all_auths();
    let (client, admin) = setup_contract(e);
    // Sequence 0 is the "not ready" sentinel in `GuardianRecovery`.
    e.ledger().with_mut(|l| l.sequence_number = 1_000);
    let g1 = Address::generate(e);
    let g2 = Address::generate(e);
    let mut guardians: Vec<Address> = Vec::new(e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);
    client.recovery_set_delay(&DELAY);
    Drill {
        client,
        attacker_admin: admin,
        g1,
        g2,
        rescue: Address::generate(e),
    }
}

fn reach_quorum(d: &Drill) {
    d.client.recovery_approve(&d.g1, &d.rescue);
    d.client.recovery_approve(&d.g2, &d.rescue);
}

#[test]
fn compromised_admin_cannot_swap_guardians_mid_recovery() {
    let e = Env::default();
    let d = setup(&e);
    d.client.recovery_approve(&d.g1, &d.rescue);
    let mut puppets: Vec<Address> = Vec::new(&e);
    puppets.push_back(Address::generate(&e));
    let r = d.client.try_recovery_set_guardians(&puppets, &1u32);
    assert_eq!(r, Err(Ok(code(ErrorCode::RecoveryAlreadyPending))));
}

#[test]
fn compromised_admin_cannot_stretch_delay_mid_recovery() {
    let e = Env::default();
    let d = setup(&e);
    reach_quorum(&d);
    let r = d.client.try_recovery_set_delay(&50_000u32);
    assert_eq!(r, Err(Ok(code(ErrorCode::RecoveryAlreadyPending))));
}

#[test]
fn recovery_delay_is_bounded() {
    let e = Env::default();
    let d = setup(&e);
    let r = d.client.try_recovery_set_delay(&u32::MAX);
    assert_eq!(r, Err(Ok(code(ErrorCode::InvalidConfiguration))));
}

#[test]
fn compromised_admin_gets_only_one_veto_per_candidate() {
    let e = Env::default();
    let d = setup(&e);
    reach_quorum(&d);
    d.client.recovery_cancel();

    // Guardians insist on the same candidate; a second veto is refused.
    reach_quorum(&d);
    let r = d.client.try_recovery_cancel();
    assert_eq!(r, Err(Ok(code(ErrorCode::NotAuthorized))));
}

#[test]
fn veto_cooldown_blocks_guardian_swap() {
    let e = Env::default();
    let d = setup(&e);
    reach_quorum(&d);
    d.client.recovery_cancel();

    let mut puppets: Vec<Address> = Vec::new(&e);
    puppets.push_back(Address::generate(&e));
    let r = d.client.try_recovery_set_guardians(&puppets, &1u32);
    assert_eq!(r, Err(Ok(code(ErrorCode::RecoveryAlreadyPending))));

    // After the cooldown an honest admin may rotate guardians again.
    advance(&e, DELAY);
    d.client.recovery_set_guardians(&puppets, &1u32);
}

#[test]
fn admin_rotation_does_not_evade_recovery() {
    let e = Env::default();
    let d = setup(&e);
    reach_quorum(&d);
    // Attacker hands admin to a second key it controls.
    let attacker_2 = Address::generate(&e);
    d.client.set_admin(&attacker_2);
    advance(&e, DELAY);
    d.client.recovery_execute();
    assert_eq!(d.client.get_admin(), d.rescue);
}

/// Scripted drill: detection → revocation → rotation → state verification →
/// reinstatement, with the attacker vetoing once. Ledger counts are the
/// time-to-contain figures reported in the drill document.
#[test]
fn recovery_drill_end_to_end() {
    let e = Env::default();
    let d = setup(&e);
    let start = e.ledger().sequence();

    // 1. Compromise: attacker admits a rogue source (detected via events).
    let rogue = register_test_source(&e, &d.client, "Rogue");

    // 2. Revocation: guardians reach quorum; attacker vetoes once.
    reach_quorum(&d);
    d.client.recovery_cancel();
    reach_quorum(&d);
    assert!(d.client.try_recovery_cancel().is_err());

    // 3. Rotation after the cancellation window.
    advance(&e, DELAY - 1);
    assert!(d.client.try_recovery_execute().is_err());
    advance(&e, 1);
    d.client.recovery_execute();
    assert_eq!(d.client.get_admin(), d.rescue);
    assert_ne!(d.client.get_admin(), d.attacker_admin);
    let contained_after = e.ledger().sequence() - start;
    assert_eq!(contained_after, DELAY);

    // 4. State verification + reinstatement by the new admin.
    d.client.remove_source(&rogue);
    assert!(!d.client.is_source(&rogue));
    let honest = Address::generate(&e);
    d.client
        .add_source(&honest, &String::from_str(&e, "Honest"));
    assert!(d.client.is_source(&honest));
    assert!(d.client.recovery_get_pending().is_none());
}

fn code(c: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(c as u32)
}
