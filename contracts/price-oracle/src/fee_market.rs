//! # Prioritized Submission Fee Market (#176)
//!
//! Implements a priority-fee mechanism for price submissions.  Oracle sources attach a
//! `priority_fee` to their submission; the contract maintains a priority buffer ordered
//! by `(priority_fee DESC, timestamp ASC)`.  Submissions are processed up to a
//! per-invocation instruction safety boundary, and any remainder rolls over to the
//! next ledger.  Accumulated fees are split between sources (as incentive) and the
//! treasury based on a configurable `fee_distribution_ratio`.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{Asset, DataKey, ErrorCode, FeeMarketSubmission, PendingFeeSubmissions};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Default minimum priority fee (in stroops equivalent).
pub const DEFAULT_MIN_PRIORITY_FEE: u128 = 0;

/// Default fee distribution ratio: 80 % to sources, 20 % to treasury (out of 100).
pub const DEFAULT_FEE_DISTRIBUTION_RATIO: u32 = 80;

/// Maximum number of submissions processed per invocation of `process_fee_market`.
/// Chosen conservatively to stay well within the 4 M instruction budget.
pub const MAX_PROCESS_PER_LEDGER: u32 = 20;

// ─────────────────────────────────────────────────────────────────────────────
// Storage helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_pending(env: &Env) -> PendingFeeSubmissions {
    let key = DataKey::FmPendingQueue;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(PendingFeeSubmissions {
            submissions: Vec::new(env),
        })
}

fn write_pending(env: &Env, queue: &PendingFeeSubmissions) {
    let key = DataKey::FmPendingQueue;
    env.storage().persistent().set(&key, queue);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_fee_pool(env: &Env) -> u128 {
    env.storage()
        .persistent()
        .get(&DataKey::FmFeePool)
        .unwrap_or(0u128)
}

fn write_fee_pool(env: &Env, pool: u128) {
    env.storage().persistent().set(&DataKey::FmFeePool, &pool);
}

fn read_source_fee_balance(env: &Env, source: &Address) -> u128 {
    env.storage()
        .persistent()
        .get(&DataKey::FmSourceFeeBalance(source.clone()))
        .unwrap_or(0u128)
}

fn write_source_fee_balance(env: &Env, source: &Address, balance: u128) {
    let key = DataKey::FmSourceFeeBalance(source.clone());
    env.storage().persistent().set(&key, &balance);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_treasury_balance(env: &Env) -> u128 {
    env.storage()
        .persistent()
        .get(&DataKey::FmTreasuryBalance)
        .unwrap_or(0u128)
}

fn write_treasury_balance(env: &Env, balance: u128) {
    env.storage()
        .persistent()
        .set(&DataKey::FmTreasuryBalance, &balance);
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration accessors
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_min_priority_fee(env: &Env) -> u128 {
    env.storage()
        .persistent()
        .get(&DataKey::FmMinPriorityFee)
        .unwrap_or(DEFAULT_MIN_PRIORITY_FEE)
}

pub fn set_min_priority_fee(env: &Env, min_fee: u128) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::FmMinPriorityFee, &min_fee);
    crate::events::FmMinPriorityFeeEvent { value: min_fee }.publish(env);
}

pub fn get_fee_distribution_ratio(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::FmFeeDistributionRatio)
        .unwrap_or(DEFAULT_FEE_DISTRIBUTION_RATIO)
}

pub fn set_fee_distribution_ratio(env: &Env, ratio: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if ratio > 100 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::FmFeeDistributionRatio, &ratio);
    crate::events::FmFeeDistRatioChangedEvent { value: ratio }.publish(env);
}

pub fn get_treasury_address(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&DataKey::FmTreasury)
}

pub fn set_treasury_address(env: &Env, treasury: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::FmTreasury, &treasury);
}

// ─────────────────────────────────────────────────────────────────────────────
// Core enqueue / process logic
// ─────────────────────────────────────────────────────────────────────────────

/// Enqueues a fee-market price submission.
///
/// Validates that:
/// - `source` is a registered oracle source and has authorized this call.
/// - `asset` is registered.
/// - `priority_fee` ≥ the configured minimum.
/// - `price` > 0.
///
/// The submission is inserted into the priority buffer in sorted order:
/// primary key `priority_fee DESC`, secondary key `timestamp ASC`.
pub fn enqueue_submission(
    env: &Env,
    source: Address,
    asset: Asset,
    price: u128,
    timestamp: u64,
    priority_fee: u128,
) {
    source.require_auth();

    // Validate source
    let source_key = DataKey::SrcActive(source.clone());
    let is_source: bool = env.storage().persistent().get(&source_key).unwrap_or(false);
    if !is_source {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    // Validate asset
    let asset_addr = match &asset {
        Asset::Stellar(addr) => addr.clone(),
        Asset::Other(_) => panic_with_error!(env, ErrorCode::AssetNotRegistered),
    };
    crate::storage::check_registered_asset(env, &asset_addr);

    // Validate price
    if price == 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    // Validate priority_fee >= minimum
    let min_fee = get_min_priority_fee(env);
    if priority_fee < min_fee {
        panic_with_error!(env, ErrorCode::FeeMarketBelowMinimum);
    }

    let submission = FeeMarketSubmission {
        source,
        asset,
        price,
        timestamp,
        priority_fee,
        submitted_ledger: env.ledger().sequence(),
    };

    let mut queue = read_pending(env);
    // Insert in sorted position: priority_fee DESC, timestamp ASC
    let insert_pos = find_insert_position(&queue.submissions, &submission);
    queue.submissions.insert(insert_pos, submission.clone());
    write_pending(env, &queue);

    // Accumulate fee into fee pool immediately on enqueue
    let pool = read_fee_pool(env);
    write_fee_pool(env, pool.saturating_add(priority_fee));

    crate::events::FmSubmissionEnqueuedEvent {
        source: submission.source.clone(),
        priority_fee,
        queue_depth: queue.submissions.len(),
    }
    .publish(env);
}

/// Finds the insertion position for a new submission maintaining sorted order.
/// Sort order: `priority_fee DESC` then `timestamp ASC`.
fn find_insert_position(queue: &Vec<FeeMarketSubmission>, new_sub: &FeeMarketSubmission) -> u32 {
    let len = queue.len();
    for i in 0..len {
        let existing = queue.get_unchecked(i);
        if new_sub.priority_fee > existing.priority_fee
            || (new_sub.priority_fee == existing.priority_fee
                && new_sub.timestamp < existing.timestamp)
        {
            return i;
        }
    }
    len
}

/// Processes up to `MAX_PROCESS_PER_LEDGER` submissions from the priority queue.
///
/// For each submission processed:
/// 1. Calls the existing `submit_price` logic (write `PriceEntry`, aggregate).
/// 2. Distributes the priority fee between the source and treasury based on ratio.
///
/// Returns the number of submissions actually processed.
pub fn process_fee_market(env: &Env) -> u32 {
    let mut queue = read_pending(env);
    let total = queue.submissions.len();
    if total == 0 {
        return 0;
    }

    let process_count = total.min(MAX_PROCESS_PER_LEDGER);
    let ratio = get_fee_distribution_ratio(env); // % to source
    let mut processed = 0u32;

    for _ in 0..process_count {
        if queue.submissions.is_empty() {
            break;
        }
        let sub = queue.submissions.get_unchecked(0);
        queue.submissions.remove(0);

        let asset_addr = match &sub.asset {
            Asset::Stellar(addr) => addr.clone(),
            Asset::Other(_) => continue,
        };

        // Submit price through core pricing logic (no extra auth needed — already validated at enqueue)
        crate::prices::submit_price_internal(
            env,
            sub.source.clone(),
            asset_addr.clone(),
            sub.price as i128,
            sub.timestamp,
        );

        // Distribute fee
        let fee = sub.priority_fee;
        let source_share = fee.saturating_mul(ratio as u128) / 100;
        let treasury_share = fee.saturating_sub(source_share);

        let src_bal = read_source_fee_balance(env, &sub.source);
        write_source_fee_balance(env, &sub.source, src_bal.saturating_add(source_share));

        let tsy_bal = read_treasury_balance(env);
        write_treasury_balance(env, tsy_bal.saturating_add(treasury_share));

        processed += 1;

        crate::events::FmSubmissionProcessedEvent {
            source: sub.source.clone(),
            asset: asset_addr,
            price: sub.price as i128,
            priority_fee: fee,
            source_share,
            treasury_share,
        }
        .publish(env);
    }

    write_pending(env, &queue);
    processed
}

// ─────────────────────────────────────────────────────────────────────────────
// Query helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the number of submissions currently in the priority queue.
pub fn get_pending_submissions(env: &Env) -> u32 {
    read_pending(env).submissions.len()
}

/// Returns the accumulated fee balance for a source.
pub fn get_source_fee_balance(env: &Env, source: Address) -> u128 {
    read_source_fee_balance(env, &source)
}

/// Returns the accumulated treasury fee balance.
pub fn get_treasury_fee_balance(env: &Env) -> u128 {
    read_treasury_balance(env)
}
