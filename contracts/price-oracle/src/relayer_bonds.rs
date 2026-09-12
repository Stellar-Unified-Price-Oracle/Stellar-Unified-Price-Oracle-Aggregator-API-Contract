//! # Relayer Performance Bonds (#265)
//!
//! Relayers post a configurable performance bond before being economically accountable
//! for the prices they relay (mirrors the source liveness bond in `sources.rs`).
//! Admin-attested misbehavior (unauthorized prices, excessive submission failures)
//! makes a relayer slash-eligible; a portion of its bond is then forfeited to the
//! shared treasury (the same [`DataKey::TreasuryBalance`] used by source slashing,
//! see [`crate::reputation::get_treasury_balance`]). Well-behaved relayers additionally
//! accrue an accuracy-weighted reward on top of the flat per-submission fee tracked in
//! `relayer.rs`.
//!
//! ## Why failures are admin-reported rather than auto-detected
//!
//! A Soroban contract invocation is atomic: any panic (an invalid price, an
//! unauthorized source, etc.) rolls back *every* storage write made during that
//! call, including any failure counter we might try to bump right before panicking.
//! So instead of trying to self-report failures from within the reverted call path,
//! operators/admins attest to failure incidents out-of-band (via `record_relayer_failure`)
//! after observing misbehavior — the same admin-driven pattern `reputation::slash_source`
//! already uses for sources (a `force` override, rather than fully automatic slashing).

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{
    emit_relayer_bond_config_changed, RelayerBondDepositedEvent, RelayerBondWithdrawnEvent,
    RelayerFailureRecordedEvent, RelayerSlashedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, RelayerFailureReason};

/// Default percentage of bond slashed per slash event (e.g. 20 = 20%).
pub const DEFAULT_RELAYER_SLASH_PERCENT: u32 = 20;
/// Default failure-count threshold at/above which a relayer becomes slash-eligible.
pub const DEFAULT_RELAYER_FAILURE_THRESHOLD: u32 = 3;

// ---------------------------------------------------------------------------
// Bond configuration
// ---------------------------------------------------------------------------

/// Sets the required relayer bond amount (in stroops). Admin-only.
pub fn set_relayer_bond_amount(env: &Env, amount: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::RelayerBondAmount, &amount);
    emit_relayer_bond_config_changed(env, admin, amount);
}

/// Returns the currently configured required relayer bond amount. Defaults to `0`
/// (no bond required).
pub fn get_relayer_bond_amount(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerBondAmount)
        .unwrap_or(0i128)
}

// ---------------------------------------------------------------------------
// Bond deposit / withdrawal
// ---------------------------------------------------------------------------

/// Deposits (tops up to) the required bond amount for `relayer`.
///
/// The relayer must authorize this call. Transfers only the shortfall between the
/// currently deposited amount and the configured requirement, using the shared
/// staking token configured via [`crate::reputation::set_stake_token_contract`].
/// A no-op if no bond is required or the relayer is already fully bonded.
///
/// # Errors
///
/// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
/// * [`ErrorCode::StakeTokenNotConfigured`] — no staking token has been configured.
pub fn deposit_relayer_bond(env: &Env, relayer: Address) {
    relayer.require_auth();

    if !crate::relayer::is_relayer(env, relayer.clone()) {
        panic_with_error!(env, ErrorCode::RelayerNotAuthorized);
    }

    let required = get_relayer_bond_amount(env);
    if required <= 0 {
        return;
    }

    let current = get_relayer_bond_balance(env, relayer.clone());
    if current >= required {
        return;
    }

    let deposit_amount = required - current;
    let token_contract = crate::reputation::get_stake_token_contract(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::StakeTokenNotConfigured));

    let client = soroban_sdk::token::Client::new(env, &token_contract);
    client.transfer(&relayer, env.current_contract_address(), &deposit_amount);

    let key = DataKey::RelayerBond(relayer.clone());
    env.storage().persistent().set(&key, &required);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    RelayerBondDepositedEvent {
        relayer,
        amount: deposit_amount,
        total_deposited: required,
    }
    .publish(env);
}

/// Returns the currently deposited bond balance (in stroops) for `relayer`.
pub fn get_relayer_bond_balance(env: &Env, relayer: Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerBond(relayer))
        .unwrap_or(0i128)
}

/// Withdraws the entire deposited bond back to `relayer`.
///
/// The relayer must authorize this call. A no-op if nothing is deposited.
///
/// # Errors
///
/// * [`ErrorCode::StakeTokenNotConfigured`] — no staking token has been configured
///   (only possible if a bond was previously deposited under a token that was later
///   unset).
pub fn withdraw_relayer_bond(env: &Env, relayer: Address) {
    relayer.require_auth();

    let key = DataKey::RelayerBond(relayer.clone());
    let deposited: i128 = env.storage().persistent().get(&key).unwrap_or(0i128);
    if deposited <= 0 {
        return;
    }

    let token_contract = crate::reputation::get_stake_token_contract(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::StakeTokenNotConfigured));
    let client = soroban_sdk::token::Client::new(env, &token_contract);
    client.transfer(&env.current_contract_address(), &relayer, &deposited);

    env.storage().persistent().set(&key, &0i128);

    RelayerBondWithdrawnEvent {
        relayer,
        amount: deposited,
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Failure recording & slashing
// ---------------------------------------------------------------------------

/// Records a failure incident against `relayer`, making it eligible for slashing
/// once the configured failure threshold is reached. Admin-only.
///
/// See the module-level documentation for why this is admin-reported rather than
/// auto-detected from a reverted submission.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
/// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
pub fn record_relayer_failure(env: &Env, relayer: Address, reason: RelayerFailureReason) {
    let admin = get_admin(env);
    admin.require_auth();

    if !crate::relayer::is_relayer(env, relayer.clone()) {
        panic_with_error!(env, ErrorCode::RelayerNotAuthorized);
    }

    let key = DataKey::RelayerFailureCount(relayer.clone());
    let count: u32 = env.storage().persistent().get(&key).unwrap_or(0u32);
    let new_count = count.saturating_add(1);
    env.storage().persistent().set(&key, &new_count);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    RelayerFailureRecordedEvent {
        relayer,
        reason: reason as u32,
        failure_count: new_count,
    }
    .publish(env);
}

/// Returns the number of reported failure incidents for `relayer`.
pub fn get_relayer_failure_count(env: &Env, relayer: Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerFailureCount(relayer))
        .unwrap_or(0u32)
}

/// Slashes a configurable percentage of `relayer`'s deposited bond. Admin-only.
///
/// Unless `force` is `true`, the relayer's reported failure count must be at or
/// above the configured [`get_relayer_failure_threshold`]. Slashed funds move to the
/// shared treasury balance and the failure counter resets.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
/// * [`ErrorCode::RelayerFailureThresholdNotReached`] — not forced, and the relayer's
///   failure count is below the slash-eligibility threshold.
pub fn slash_relayer(env: &Env, relayer: Address, force: bool) {
    let admin = get_admin(env);
    admin.require_auth();

    if !force {
        let threshold = get_relayer_failure_threshold(env);
        let count = get_relayer_failure_count(env, relayer.clone());
        if count < threshold {
            panic_with_error!(env, ErrorCode::RelayerFailureThresholdNotReached);
        }
    }

    let key = DataKey::RelayerBond(relayer.clone());
    let deposited: i128 = env.storage().persistent().get(&key).unwrap_or(0i128);
    if deposited <= 0 {
        return;
    }

    let slash_pct = get_relayer_slash_percent(env) as i128;
    let slash_amount = (deposited * slash_pct / 100).max(1).min(deposited);
    let remaining = deposited - slash_amount;

    env.storage().persistent().set(&key, &remaining);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let treasury_key = DataKey::TreasuryBalance;
    let treasury: i128 = env
        .storage()
        .persistent()
        .get(&treasury_key)
        .unwrap_or(0i128);
    env.storage()
        .persistent()
        .set(&treasury_key, &treasury.saturating_add(slash_amount));
    env.storage()
        .persistent()
        .extend_ttl(&treasury_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Reset the failure counter so repeated slashes require fresh evidence.
    env.storage()
        .persistent()
        .set(&DataKey::RelayerFailureCount(relayer.clone()), &0u32);

    RelayerSlashedEvent {
        relayer,
        slash_amount,
        remaining_bond: remaining,
        slash_percent: slash_pct as u32,
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Sets the slash percentage (0-100) applied to a relayer's bond. Admin-only.
pub fn set_relayer_slash_percent(env: &Env, percent: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if percent > 100 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::RelayerSlashPercent, &percent);
}

/// Returns the current relayer slash percentage. Defaults to
/// [`DEFAULT_RELAYER_SLASH_PERCENT`].
pub fn get_relayer_slash_percent(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerSlashPercent)
        .unwrap_or(DEFAULT_RELAYER_SLASH_PERCENT)
}

/// Sets the failure-count threshold at/above which a relayer becomes slash-eligible.
/// Admin-only.
pub fn set_relayer_failure_threshold(env: &Env, threshold: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::RelayerFailureThreshold, &threshold);
}

/// Returns the current relayer failure threshold. Defaults to
/// [`DEFAULT_RELAYER_FAILURE_THRESHOLD`].
pub fn get_relayer_failure_threshold(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerFailureThreshold)
        .unwrap_or(DEFAULT_RELAYER_FAILURE_THRESHOLD)
}

/// Sets the reward rate (in stroops) credited per accuracy-weighted relayed
/// submission. Admin-only. `0` disables reward accrual.
pub fn set_relayer_reward_rate(env: &Env, rate: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::RelayerRewardRate, &rate);
}

/// Returns the current relayer reward rate in stroops. Defaults to `0`.
pub fn get_relayer_reward_rate(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerRewardRate)
        .unwrap_or(0i128)
}

/// Returns the total accumulated reward balance (in stroops) owed to `relayer`.
pub fn get_relayer_reward_balance(env: &Env, relayer: Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerRewardBalance(relayer))
        .unwrap_or(0i128)
}

/// Accrues an accuracy-weighted reward for `relayer` after a successful submission.
///
/// `accuracy_bps = 10_000 * successful / (successful + failed)`, so a relayer with
/// zero reported failures earns the full configured rate per submission, while one
/// with reported failures earns proportionally less. Called internally by
/// `relayer.rs` after each successful relayed submission; a no-op if no reward rate
/// is configured.
pub fn accrue_relayer_reward(env: &Env, relayer: &Address, total_submissions: u64) {
    let rate = get_relayer_reward_rate(env);
    if rate <= 0 {
        return;
    }

    let failures = get_relayer_failure_count(env, relayer.clone()) as u64;
    let denom = total_submissions.saturating_add(failures).max(1);
    let accuracy_bps = total_submissions.saturating_mul(10_000) / denom;
    let reward = rate.saturating_mul(accuracy_bps as i128) / 10_000;

    if reward > 0 {
        let key = DataKey::RelayerRewardBalance(relayer.clone());
        let balance: i128 = env.storage().persistent().get(&key).unwrap_or(0i128);
        env.storage()
            .persistent()
            .set(&key, &balance.saturating_add(reward));
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}
