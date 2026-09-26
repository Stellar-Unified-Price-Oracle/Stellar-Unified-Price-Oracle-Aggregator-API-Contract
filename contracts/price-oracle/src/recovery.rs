//! # Admin Key Social Recovery (#245)
//!
//! N-of-M guardian addresses, registered by the current admin, can collectively
//! recover control of the contract if the admin key is lost. A guardian names a
//! candidate replacement admin; once enough guardians (the configured threshold)
//! have approved the same candidate, a cancellation-window delay begins. The
//! current admin may cancel at any point before execution. Once the delay has
//! elapsed, anyone may trigger execution, which installs the new admin.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{
    GuardiansSetEvent, RecoveryApprovedEvent, RecoveryCancelledEvent, RecoveryExecutedEvent,
    RecoveryReadyEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, GuardianRecovery};

/// Default cancellation-window delay: ~1 day, assuming ~5s ledgers.
const DEFAULT_RECOVERY_DELAY: u32 = 17_280;
/// Upper bound on the cancellation-window delay (~10 days at 5s ledgers), so a
/// compromised admin cannot stall recovery indefinitely with a huge delay (#454).
const MAX_RECOVERY_DELAY: u32 = 172_800;

/// Rejects guardian-set / delay changes while a recovery is in flight or while
/// a veto cooldown is active, so a compromised admin cannot swap out the
/// guardians (or stretch the delay) to block its own revocation (#454).
fn check_recovery_config_unlocked(env: &Env) {
    if env.storage().persistent().has(&DataKey::PendingRecovery) {
        panic_with_error!(env, ErrorCode::RecoveryAlreadyPending);
    }
    if let Some((_, until)) = env
        .storage()
        .persistent()
        .get::<DataKey, (Address, u32)>(&DataKey::RecoveryVeto)
    {
        if env.ledger().sequence() < until {
            panic_with_error!(env, ErrorCode::RecoveryAlreadyPending);
        }
    }
}

fn read_guardians(env: &Env) -> Vec<Address> {
    let key = DataKey::RecoveryGuardians;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env))
}

fn read_threshold(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::RecoveryThreshold)
        .unwrap_or(0)
}

/// Registers the guardian set and the number of approvals required to initiate
/// recovery. Admin-only. Replaces any previously configured guardian set.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidGuardianConfig`] — threshold is `0` or exceeds the guardian count.
/// * [`ErrorCode::RecoveryAlreadyPending`] — a recovery is pending or a veto cooldown is active.
pub fn set_guardians(env: &Env, guardians: Vec<Address>, threshold: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    check_recovery_config_unlocked(env);

    if threshold == 0 || threshold > guardians.len() {
        panic_with_error!(env, ErrorCode::InvalidGuardianConfig);
    }

    let key = DataKey::RecoveryGuardians;
    env.storage().persistent().set(&key, &guardians);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    env.storage()
        .persistent()
        .set(&DataKey::RecoveryThreshold, &threshold);

    GuardiansSetEvent {
        admin: admin.clone(),
        guardian_count: guardians.len(),
        threshold,
    }
    .publish(env);
    crate::events::emit_admin_action(
        env,
        soroban_sdk::symbol_short!("set_grd"),
        admin,
        soroban_sdk::Bytes::new(env),
    );
}

/// Returns the currently registered guardian addresses.
pub fn get_guardians(env: &Env) -> Vec<Address> {
    read_guardians(env)
}

/// Returns the number of guardian approvals required to reach recovery threshold.
pub fn get_recovery_threshold(env: &Env) -> u32 {
    read_threshold(env)
}

/// Sets the cancellation-window delay (in ledgers) between reaching guardian
/// threshold and a recovery becoming eligible for auto-execution. Admin-only.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidConfiguration`] — `delay_ledgers` is `0` or exceeds `MAX_RECOVERY_DELAY`.
/// * [`ErrorCode::RecoveryAlreadyPending`] — a recovery is pending or a veto cooldown is active.
pub fn set_recovery_delay(env: &Env, delay_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    check_recovery_config_unlocked(env);
    if delay_ledgers == 0 || delay_ledgers > MAX_RECOVERY_DELAY {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::RecoveryDelay, &delay_ledgers);
}

/// Returns the configured cancellation-window delay in ledgers. Default: ~1 day.
pub fn get_recovery_delay(env: &Env) -> u32 {
    let key = DataKey::RecoveryDelay;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(DEFAULT_RECOVERY_DELAY)
}

/// A guardian approves recovery, naming `new_admin` as the candidate replacement admin.
///
/// The first guardian to call this for a given candidate initiates the recovery.
/// Subsequent guardians must name the same `new_admin` to add their approval. Once
/// the required threshold of distinct guardian approvals is reached, the
/// cancellation-window delay starts.
///
/// # Errors
/// * [`ErrorCode::NotGuardian`] — caller is not a registered guardian.
/// * [`ErrorCode::RecoveryAlreadyPending`] — a recovery is already pending for a
///   different candidate; it must be cancelled by the admin first.
/// * [`ErrorCode::RecoveryAlreadyApproved`] — this guardian already approved.
pub fn approve_recovery(env: &Env, guardian: Address, new_admin: Address) {
    guardian.require_auth();

    let guardians = read_guardians(env);
    if !guardians.contains(&guardian) {
        panic_with_error!(env, ErrorCode::NotGuardian);
    }

    let key = DataKey::PendingRecovery;
    let mut recovery: GuardianRecovery = match env.storage().persistent().get(&key) {
        Some(existing) => existing,
        None => GuardianRecovery {
            new_admin: new_admin.clone(),
            approvals: Vec::new(env),
            initiated_ledger: env.ledger().sequence(),
            ready_ledger: 0,
        },
    };

    if recovery.new_admin != new_admin {
        panic_with_error!(env, ErrorCode::RecoveryAlreadyPending);
    }

    if recovery.approvals.contains(&guardian) {
        panic_with_error!(env, ErrorCode::RecoveryAlreadyApproved);
    }
    recovery.approvals.push_back(guardian.clone());

    let threshold = read_threshold(env);
    if recovery.ready_ledger == 0 && recovery.approvals.len() >= threshold {
        let ready_ledger = env.ledger().sequence();
        recovery.ready_ledger = ready_ledger;

        RecoveryReadyEvent {
            new_admin: new_admin.clone(),
            ready_ledger,
            execute_after_ledger: ready_ledger + get_recovery_delay(env),
        }
        .publish(env);
    }

    let approval_count = recovery.approvals.len();
    env.storage().persistent().set(&key, &recovery);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    RecoveryApprovedEvent {
        guardian,
        new_admin,
        approval_count,
        threshold,
    }
    .publish(env);
}

/// Cancels the pending recovery. Admin-only — this is the cancellation window that
/// lets a still-in-control admin stop a recovery before it executes.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin, or the pending
///   recovery re-reached quorum for a candidate the admin already vetoed.
/// * [`ErrorCode::RecoveryNotPending`] — no recovery is currently pending.
pub fn cancel_recovery(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::PendingRecovery;
    let recovery: GuardianRecovery = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::RecoveryNotPending));

    // The admin gets one veto per candidate once guardians reach quorum. If
    // the guardians re-approve the same candidate, the admin can no longer
    // cancel it — guardian quorum outranks the admin key (#454).
    if recovery.ready_ledger != 0 {
        let veto: Option<(Address, u32)> = env.storage().persistent().get(&DataKey::RecoveryVeto);
        if let Some((vetoed, _)) = veto {
            if vetoed == recovery.new_admin {
                panic_with_error!(env, ErrorCode::NotAuthorized);
            }
        }
        let until = env
            .ledger()
            .sequence()
            .saturating_add(get_recovery_delay(env));
        env.storage()
            .persistent()
            .set(&DataKey::RecoveryVeto, &(recovery.new_admin.clone(), until));
        env.storage().persistent().extend_ttl(
            &DataKey::RecoveryVeto,
            LEDGER_THRESHOLD,
            LEDGER_BUMP,
        );
    }

    env.storage().persistent().remove(&key);

    RecoveryCancelledEvent {
        admin,
        new_admin: recovery.new_admin,
    }
    .publish(env);
}

/// Executes a ready recovery, installing its candidate as the new contract admin.
///
/// Callable by anyone once guardian threshold has been reached and the
/// cancellation-window delay has elapsed — no special privilege is required to
/// trigger execution once those conditions hold, which is what makes it "automatic".
///
/// # Errors
/// * [`ErrorCode::RecoveryNotPending`] — no recovery is currently pending.
/// * [`ErrorCode::RecoveryDelayNotElapsed`] — threshold not yet reached, or the
///   cancellation-window delay has not yet elapsed.
pub fn execute_recovery(env: &Env) {
    let key = DataKey::PendingRecovery;
    let recovery: GuardianRecovery = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::RecoveryNotPending));

    if recovery.ready_ledger == 0 {
        panic_with_error!(env, ErrorCode::RecoveryDelayNotElapsed);
    }

    let delay = get_recovery_delay(env);
    if env.ledger().sequence() < recovery.ready_ledger.saturating_add(delay) {
        panic_with_error!(env, ErrorCode::RecoveryDelayNotElapsed);
    }

    let old_admin = get_admin(env);
    env.storage()
        .persistent()
        .set(&DataKey::Admin, &recovery.new_admin);
    env.storage().persistent().remove(&key);
    env.storage().persistent().remove(&DataKey::RecoveryVeto);

    RecoveryExecutedEvent {
        old_admin,
        new_admin: recovery.new_admin,
    }
    .publish(env);
}

/// Returns the currently pending recovery, if any.
pub fn get_pending_recovery(env: &Env) -> Option<GuardianRecovery> {
    env.storage().persistent().get(&DataKey::PendingRecovery)
}
