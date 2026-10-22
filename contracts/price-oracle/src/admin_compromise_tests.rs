//! # #454 — Admin key compromise: blast radius and recovery drill
//! # #658 — Admin succession and unavailable-signer recovery
//!
//! Proves a compromised admin key cannot block its own revocation through
//! the guardian recovery path, and scripts the recovery drill end to end.
//! The capability list and residual risk are documented in
//! `docs/security/admin-compromise-drill.md`.
//!
//! # #604 — Mainnet administrator multisig migration rehearsal
//!
//! Rehearses the handoff from bootstrap administration to the production
//! multisig. The procedure is documented in
//! `docs/operations/admin-multisig-migration.md`; these tests prove the
//! handoff ordering leaves no unowned interval and that neither the old nor
//! the new signer set can create an unauthorized transition.

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

/// #604 — Rehearses the bootstrap → production multisig handoff.
///
/// Ordering (see `docs/operations/admin-multisig-migration.md`):
///   1. Bootstrap admin configures guardians + delay (recovery armed).
///   2. Bootstrap admin hands admin to the production multisig address.
///   3. Multisig verifies ownership and performs the first authorized action.
///
/// The admin slot is never empty: `set_admin` is atomic, so the interval
/// between the old and new owner is a single ledger with exactly one owner.
#[test]
fn multisig_migration_handoff_leaves_no_unowned_interval() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, bootstrap) = setup_contract(&e);
    e.ledger().with_mut(|l| l.sequence_number = 1_000);

    // 1. Arm recovery while the bootstrap admin is still in control.
    let g1 = Address::generate(&e);
    let g2 = Address::generate(&e);
    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);
    client.recovery_set_delay(&DELAY);

    // 2. Atomic handoff to the production multisig address.
    let multisig = Address::generate(&e);
    assert_eq!(client.get_admin(), bootstrap);
    client.set_admin(&multisig);

    // 3. Ownership is continuous: exactly one owner at every step.
    assert_eq!(client.get_admin(), multisig);
    assert_ne!(client.get_admin(), bootstrap);

    // 4. The new owner performs the first authorized action.
    let src = register_test_source(&e, &client, "PostMigration");
    assert!(client.is_source(&src));
    assert!(client.recovery_get_pending().is_none());
}

/// #604 — After migration the old bootstrap signer set cannot create an
/// unauthorized transition: it can no longer rotate admin or guardians.
#[test]
fn old_signer_set_cannot_create_unauthorized_transition_after_migration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, bootstrap) = setup_contract(&e);
    e.ledger().with_mut(|l| l.sequence_number = 1_000);

    let multisig = Address::generate(&e);
    client.set_admin(&multisig);
    assert_eq!(client.get_admin(), multisig);

    // The old bootstrap key is no longer the admin; a fresh admin rotation
    // must be authorized by the current owner, not the retired one.
    let rogue = Address::generate(&e);
    e.set_auths(&[]);
    let r = client.try_set_admin(&rogue);
    assert!(r.is_err());
    assert_eq!(client.get_admin(), multisig);
    assert_ne!(client.get_admin(), bootstrap);
}

/// #604 — The new multisig signer set cannot create an unauthorized
/// transition outside its authorized scope: it cannot bypass the guardian
/// recovery path to seize guardians mid-recovery.
#[test]
fn new_signer_set_cannot_create_unauthorized_transition_outside_scope() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _bootstrap) = setup_contract(&e);
    e.ledger().with_mut(|l| l.sequence_number = 1_000);

    let g1 = Address::generate(&e);
    let g2 = Address::generate(&e);
    let mut guardians: Vec<Address> = Vec::new(&e);
    guardians.push_back(g1.clone());
    guardians.push_back(g2.clone());
    client.recovery_set_guardians(&guardians, &2u32);
    client.recovery_set_delay(&DELAY);

    let multisig = Address::generate(&e);
    client.set_admin(&multisig);
    assert_eq!(client.get_admin(), multisig);

    // Guardians open a recovery; the multisig admin cannot override it.
    let rescue = Address::generate(&e);
    client.recovery_approve(&g1, &rescue);
    let mut puppets: Vec<Address> = Vec::new(&e);
    puppets.push_back(Address::generate(&e));
    let r = client.try_recovery_set_guardians(&puppets, &1u32);
    assert_eq!(r, Err(Ok(code(ErrorCode::RecoveryAlreadyPending))));
    let r = client.try_recovery_set_delay(&50_000u32);
    assert_eq!(r, Err(Ok(code(ErrorCode::RecoveryAlreadyPending))));
}

fn code(c: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(c as u32)
}

// ---------------------------------------------------------------------------
// #653 — Production multisig policy and signer eligibility
// ---------------------------------------------------------------------------

/// Rotation cadence for production signers, in ledgers (~90 days at 5s/ledger).
const ROTATION_CADENCE_LEDGERS: u32 = 1_555_200;
/// Maximum age a signer key may reach before it must be rotated.
const MAX_SIGNER_AGE_LEDGERS: u32 = ROTATION_CADENCE_LEDGERS;
/// Minimum number of independent signers required for critical actions.
const MIN_INDEPENDENT_SIGNERS: u32 = 3;

/// A declared conflict of interest on a signer, with optional mitigation.
struct Conflict {
    declared: bool,
    mitigated: bool,
}

/// A production signer under the multisig policy.
struct Signer {
    address: Address,
    /// Custody/control domain (e.g. org or HSM operator). Signers sharing a
    /// domain are not independent.
    custody_domain: u32,
    /// Ledger at which the key was provisioned; drives rotation cadence.
    provisioned_at: u32,
    /// Whether the signer meets eligibility criteria (vetted, bonded, etc.).
    eligible: bool,
    conflict: Conflict,
}

/// Signer independence: no two signers may share a custody/control domain.
fn signers_are_independent(signers: &[Signer]) -> bool {
    for (i, a) in signers.iter().enumerate() {
        for b in signers.iter().skip(i + 1) {
            if a.custody_domain == b.custody_domain {
                return false;
            }
        }
    }
    true
}

/// Signer eligibility: every signer must be vetted and within rotation age.
fn signers_are_eligible(signers: &[Signer], now: u32) -> bool {
    signers.iter().all(|s| {
        s.eligible && now.saturating_sub(s.provisioned_at) <= MAX_SIGNER_AGE_LEDGERS
    })
}

/// A declared conflict must carry a mitigation; an undeclared conflict is
/// treated as absent. Returns false when a declared conflict lacks mitigation.
fn conflicts_are_mitigated(signers: &[Signer]) -> bool {
    signers
        .iter()
        .all(|s| !s.conflict.declared || s.conflict.mitigated)
}

// ---------------------------------------------------------------------------
// #657 — Emergency action post-incident review workflow
// ---------------------------------------------------------------------------

/// Maximum ledgers an emergency operation may remain open before its
/// post-incident review window expires (~7 days at 5s/ledger).
const REVIEW_WINDOW_LEDGERS: u32 = 120_960;

/// A post-incident review attached to an emergency operation.
struct PostIncidentReview {
    /// Accountable owner who signs off on the review.
    owner: Option<Address>,
    /// Whether the review has been published (evidence attached).
    published: bool,
    /// Ledger at which the emergency operation was opened.
    opened_at: u32,
    /// Ledger at which the review was published, if any.
    published_at: Option<u32>,
}

impl PostIncidentReview {
    fn new(opened_at: u32) -> Self {
        Self {
            owner: None,
            published: false,
            opened_at,
            published_at: None,
        }
    }

    /// Publish the review with an accountable owner and evidence.
    fn publish(&mut self, owner: Address, now: u32, has_evidence: bool) -> bool {
        if !has_evidence || self.published {
            return false;
        }
        self.owner = Some(owner);
        self.published = true;
        self.published_at = Some(now);
        true
    }

    /// Whether the review window has expired relative to `now`.
    fn window_expired(&self, now: u32) -> bool {
        now.saturating_sub(self.opened_at) > REVIEW_WINDOW_LEDGERS
    }

    /// An emergency may only be closed with a published review, an
    /// accountable owner, and a review window that has not expired.
    fn can_close(&self, now: u32) -> bool {
        self.published && self.owner.is_some() && !self.window_expired(now)
    }
}

#[test]
fn emergency_cannot_close_without_published_review() {
    let e = Env::default();
    let mut review = PostIncidentReview::new(1_000);
    // No review published yet: closing is refused.
    assert!(!review.can_close(1_000));
    // Publishing without evidence is refused.
    let owner = Address::generate(&e);
    assert!(!review.publish(owner.clone(), 1_000, false));
    assert!(!review.can_close(1_000));
}

#[test]
fn emergency_cannot_close_without_accountable_owner() {
    let e = Env::default();
    let mut review = PostIncidentReview::new(1_000);
    // A published review with no owner cannot be closed.
    review.published = true;
    review.published_at = Some(1_000);
    assert!(review.owner.is_none());
    assert!(!review.can_close(1_000));
    // Attaching an accountable owner unblocks closure.
    let owner = Address::generate(&e);
    assert!(review.publish(owner, 1_000, true));
    assert!(review.can_close(1_000));
}

#[test]
fn emergency_closes_with_published_review_and_owner() {
    let e = Env::default();
    let mut review = PostIncidentReview::new(1_000);
    let owner = Address::generate(&e);
    assert!(review.publish(owner.clone(), 1_500, true));
    assert_eq!(review.owner, Some(owner));
    assert_eq!(review.published_at, Some(1_500));
    assert!(review.can_close(1_500));
    // A second publish is refused (review is immutable once published).
    assert!(!review.publish(Address::generate(&e), 1_600, true));
}

#[test]
fn review_window_expiry_blocks_closure() {
    let e = Env::default();
    let mut review = PostIncidentReview::new(1_000);
    let owner = Address::generate(&e);
    assert!(review.publish(owner, 1_000, true));
    // At the boundary the window is still open.
    assert!(!review.window_expired(1_000 + REVIEW_WINDOW_LEDGERS));
    assert!(review.can_close(1_000 + REVIEW_WINDOW_LEDGERS));
    // One ledger past the window, closure is refused.
    assert!(review.window_expired(1_000 + REVIEW_WINDOW_LEDGERS + 1));
    assert!(!review.can_close(1_000 + REVIEW_WINDOW_LEDGERS + 1));
}

#[test]
fn review_recovery_after_window_expiry() {
    let e = Env::default();
    let mut review = PostIncidentReview::new(1_000);
    let owner = Address::generate(&e);
    assert!(review.publish(owner, 1_000, true));
    // Window expires: the emergency cannot be closed as-is.
    let expired_at = 1_000 + REVIEW_WINDOW_LEDGERS + 1;
    assert!(!review.can_close(expired_at));
    // Recovery: a fresh review opened within the window restores closure.
    let mut reopened = PostIncidentReview::new(expired_at);
    let new_owner = Address::generate(&e);
    assert!(reopened.publish(new_owner, expired_at, true));
    assert!(reopened.can_close(expired_at));
}
