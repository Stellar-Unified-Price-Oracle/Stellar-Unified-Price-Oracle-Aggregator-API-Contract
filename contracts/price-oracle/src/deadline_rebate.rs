//! # Deadline-Aware Price Submission with Rebate (#202)
//!
//! Sources can submit prices with a deadline. If used before deadline, they get
//! a gas rebate. If deadline passes, no penalty. Incentivizes timely submissions.

use soroban_sdk::{Address, Env};

use crate::events::{PriceSubmittedWithDeadlineEvent, RebateDistributedEvent};
use crate::storage::LEDGER_BUMP;
use crate::types::DataKey;

/// Records a price submission with deadline and eligibility for rebate.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source making the submission.
/// * `asset` - Asset being priced.
/// * `deadline_ledger` - Ledger by which the price must be used for rebate.
/// * `rebate_amount` - Gas rebate in stroops if used before deadline.
pub fn record_deadline_submission(
    env: &Env,
    source: Address,
    asset: Address,
    deadline_ledger: u32,
    rebate_amount: i128,
) {
    let current_ledger = env.ledger().sequence();

    // Store deadline for this submission
    let key = DataKey::SubmissionDeadline(source.clone(), asset.clone());
    env.storage().persistent().set(&key, &deadline_ledger);
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);

    // Store rebate amount
    let rebate_key = DataKey::SubmissionRebate(source.clone(), asset.clone());
    env.storage().persistent().set(&rebate_key, &rebate_amount);
    env.storage()
        .persistent()
        .extend_ttl(&rebate_key, 300000, LEDGER_BUMP);

    PriceSubmittedWithDeadlineEvent {
        source,
        asset,
        deadline_ledger,
        current_ledger,
        rebate_amount,
    }
    .publish(env);
}

/// Checks if a submission is within its deadline for rebate eligibility.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source that made the submission.
/// * `asset` - Asset that was priced.
///
/// # Returns
/// `true` if deadline has not passed, `false` if deadline expired.
pub fn is_within_deadline(env: &Env, source: &Address, asset: &Address) -> bool {
    let key = DataKey::SubmissionDeadline(source.clone(), asset.clone());
    if let Some(deadline) = env.storage().persistent().get::<_, u32>(&key) {
        env.ledger().sequence() <= deadline
    } else {
        true // No deadline = always within deadline
    }
}

/// Claims the rebate for a deadline submission if within the deadline window.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source claiming the rebate.
/// * `asset` - Asset that was submitted.
///
/// # Returns
/// Rebate amount if claim is valid, 0 otherwise.
pub fn claim_rebate(env: &Env, source: Address, asset: Address) -> i128 {
    if !is_within_deadline(env, &source, &asset) {
        return 0; // Deadline expired, no rebate
    }

    let rebate_key = DataKey::SubmissionRebate(source.clone(), asset.clone());
    let rebate: i128 = env.storage().persistent().get(&rebate_key).unwrap_or(0);

    if rebate > 0 {
        // Clear the rebate so it can't be claimed twice
        env.storage().persistent().remove(&rebate_key);

        RebateDistributedEvent {
            source,
            asset,
            rebate_amount: rebate,
        }
        .publish(env);
    }

    rebate
}

/// Returns total accumulated rebates for a source across all assets.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source address.
///
/// # Returns
/// Total rebate balance in stroops.
pub fn get_rebate_balance(env: &Env, source: &Address) -> i128 {
    let key = DataKey::RebateBalance(source.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

/// Adds rebate to a source's accumulated balance.
///
/// Called internally when rebate is earned.
pub fn add_rebate_balance(env: &Env, source: &Address, amount: i128) {
    let key = DataKey::RebateBalance(source.clone());
    let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);

    env.storage()
        .persistent()
        .set(&key, &current.saturating_add(amount));
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);
}
