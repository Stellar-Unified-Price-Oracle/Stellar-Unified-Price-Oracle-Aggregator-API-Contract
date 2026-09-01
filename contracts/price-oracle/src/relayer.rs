use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, String, Vec};

use crate::admin::{get_decimals, get_timestamp_threshold};
use crate::assets::get_min_price;
use crate::events::{
    emit_relayer_fee_set, BatchPriceRelayedEvent, PriceRelayedEvent, RelayerAddedEvent,
    RelayerRemovedEvent,
};
use crate::pause::check_not_paused;
use crate::prices::do_aggregate;
use crate::signed_submission::read_submission_key;
use crate::sources::{is_source_suspended, record_invalid_submission};
use crate::storage::{
    check_registered_asset, check_source, check_source_asset, get_admin, LEDGER_BUMP,
    LEDGER_THRESHOLD,
};
use crate::types::{
    DataKey, ErrorCode, PriceEntry, RelayedSubmission, RelayerInfo, SourceRelayerDelegation,
};

/// Maximum number of legs accepted by a single `submit_prices_relayed` batch (#264).
///
/// Bounded conservatively so a batch comfortably fits within a single invocation's
/// instruction budget alongside per-leg authorization checks.
pub const MAX_BATCH_SIZE: u32 = 25;

// ---------------------------------------------------------------------------
// Relayer registration
// ---------------------------------------------------------------------------

/// Approves a new relayer address, allowing it to submit prices on behalf of sources.
///
/// Only the admin may call this. The relayer address must not already be approved.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
/// * [`ErrorCode::RelayerAlreadyExists`] — `relayer` is already approved.
pub fn add_relayer(env: &Env, relayer: Address, name: String) {
    let admin = get_admin(env);
    admin.require_auth();

    if env
        .storage()
        .persistent()
        .has(&DataKey::ApprovedRelayer(relayer.clone()))
    {
        panic_with_error!(env, ErrorCode::RelayerAlreadyExists);
    }

    let info = RelayerInfo {
        name: name.clone(),
        approved_at_ledger: env.ledger().sequence(),
    };
    env.storage()
        .persistent()
        .set(&DataKey::ApprovedRelayer(relayer.clone()), &info);
    env.storage().persistent().extend_ttl(
        &DataKey::ApprovedRelayer(relayer.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    // Track every approved relayer for cross-relayer comparisons (#267).
    let registry_key = DataKey::RelayerRegistry;
    let mut registry: Vec<Address> = env
        .storage()
        .persistent()
        .get(&registry_key)
        .unwrap_or(Vec::new(env));
    if !registry.contains(&relayer) {
        registry.push_back(relayer.clone());
        env.storage().persistent().set(&registry_key, &registry);
        env.storage()
            .persistent()
            .extend_ttl(&registry_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    RelayerAddedEvent {
        relayer,
        admin,
        name,
    }
    .publish(env);
}

/// Revokes an approved relayer's authorization.
///
/// Only the admin may call this. The relayer must currently be approved.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
/// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not a registered relayer.
pub fn remove_relayer(env: &Env, relayer: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    if !env
        .storage()
        .persistent()
        .has(&DataKey::ApprovedRelayer(relayer.clone()))
    {
        panic_with_error!(env, ErrorCode::RelayerNotAuthorized);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::ApprovedRelayer(relayer.clone()));

    RelayerRemovedEvent { relayer, admin }.publish(env);
}

/// Returns `true` if `relayer` is an approved relayer, `false` otherwise.
pub fn is_relayer(env: &Env, relayer: Address) -> bool {
    let key = DataKey::ApprovedRelayer(relayer);
    let exists = env.storage().persistent().has(&key);
    if exists {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    exists
}

/// Returns the [`RelayerInfo`] for a given relayer address, or `None` if not approved.
pub fn get_relayer_info(env: &Env, relayer: Address) -> Option<RelayerInfo> {
    let key = DataKey::ApprovedRelayer(relayer);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

fn hash_relayer_delegation_payload(
    env: &Env,
    source: &Address,
    relayer: &Address,
    nonce: u64,
    expiration_ledger: u32,
) -> BytesN<32> {
    let mut buf = Bytes::new(env);
    buf.append(&Bytes::from_slice(env, b"source_relayer_delegation_v1"));
    buf.append(&Bytes::from_slice(env, &nonce.to_le_bytes()));
    buf.append(&Bytes::from(&source.to_string()));
    buf.append(&Bytes::from(&relayer.to_string()));
    buf.append(&Bytes::from_slice(env, &expiration_ledger.to_le_bytes()));
    env.crypto().sha256(&buf).into()
}

pub fn delegate_relayer(
    env: &Env,
    source: Address,
    relayer: Address,
    nonce: u64,
    expiration_ledger: u32,
    signature: BytesN<64>,
) {
    check_source(env, &source);
    if env.ledger().sequence() > expiration_ledger {
        panic_with_error!(env, ErrorCode::SignatureExpired);
    }

    let nonce_key = DataKey::SourceRelayerDelegationNonce(source.clone());
    let last_nonce: u64 = env.storage().persistent().get(&nonce_key).unwrap_or(0);
    if nonce <= last_nonce {
        panic_with_error!(env, ErrorCode::InvalidNonce);
    }

    let public_key = read_submission_key(env, &source);
    let digest = hash_relayer_delegation_payload(env, &source, &relayer, nonce, expiration_ledger);
    let digest_bytes: Bytes = digest.clone().into();
    env.crypto()
        .ed25519_verify(&public_key, &digest_bytes, &signature);

    let delegation = SourceRelayerDelegation {
        source: source.clone(),
        relayer: relayer.clone(),
        nonce,
        expiration_ledger,
    };
    let key = DataKey::SourceRelayerDelegation(source.clone(), relayer.clone());
    env.storage().persistent().set(&key, &delegation);
    env.storage().persistent().set(&nonce_key, &nonce);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    env.storage()
        .persistent()
        .extend_ttl(&nonce_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn get_relayer_delegation(
    env: &Env,
    source: Address,
    relayer: Address,
) -> Option<SourceRelayerDelegation> {
    let key = DataKey::SourceRelayerDelegation(source, relayer);
    let result = env.storage().persistent().get(&key);
    if result.is_some() {
        env.storage().persistent().extend_ttl(
            &key,
            LEDGER_THRESHOLD,
            LEDGER_BUMP,
        );
    }
    result
}

pub fn has_valid_relayer_delegation(env: &Env, source: &Address, relayer: &Address) -> bool {
    let Some(delegation) = get_relayer_delegation(env, source.clone(), relayer.clone()) else {
        return false;
    };
    if env.ledger().sequence() > delegation.expiration_ledger {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Relayed price submission
// ---------------------------------------------------------------------------

/// Submits a price observation for an asset on behalf of an oracle source.
///
/// This is the core relayer operation, inspired by IBC relayer protocols (Hermes / Egypt).
/// The relayer must be admin-approved, and the oracle source must explicitly authorize
/// this specific invocation — enforced by Soroban's host-level auth mechanism.
///
/// In practice, the source generates a Soroban [`AuthorizationEntry`] off-chain (signing
/// the exact call with `relayer`, `source`, `asset`, `price`, and `timestamp`), hands it
/// to the relayer, and the relayer bundles it into the transaction alongside its own
/// signature. The contract verifies both before storing the price.
///
/// On success the function:
/// 1. Stores the [`PriceEntry`] under the same key as a direct submission.
/// 2. Emits a [`PriceRelayedEvent`].
/// 3. Re-runs aggregation — emitting [`PriceAggregatedEvent`] or [`SourcesInsufficientEvent`].
/// 4. Increments the relayer's submission counter and accrues the configured fee.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `relayer` - Address of the approved relayer submitting this transaction.
/// * `source` - Address of the oracle source whose price is being relayed.
/// * `asset` - Contract address of the asset being priced.
/// * `price` - Raw price value scaled by `10^decimals`. Must be > 0.
/// * `timestamp` - Unix timestamp (seconds) of the price observation.
///
/// # Errors
///
/// * [`ErrorCode::ContractPaused`] — contract is currently paused.
/// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
/// * [`ErrorCode::NotAuthorized`] — `source` is suspended.
/// * [`ErrorCode::SourceNotFound`] — `source` is not a registered oracle source.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::InvalidPrice`] — `price` is ≤ 0.
/// * [`ErrorCode::PriceBelowMinimum`] — `price` is below the asset's minimum price floor.
/// * [`ErrorCode::InvalidTimestamp`] — `timestamp` is too far in the future.
pub fn submit_price_relayed(
    env: &Env,
    relayer: Address,
    source: Address,
    asset: Address,
    price: i128,
    timestamp: u64,
) {
    check_not_paused(env);

    // Both the relayer and the source must authorize this specific invocation.
    // The relayer signs the transaction; the source provides a pre-signed
    // Soroban AuthorizationEntry (created off-chain and bundled by the relayer).
    relayer.require_auth();
    source.require_auth();

    if !process_relayed_leg(env, &relayer, &source, &asset, price, timestamp) {
        return;
    }

    accrue_relayer_submission_metrics(env, &relayer, 0u128);
}

/// Submits prices for multiple (source, asset) legs on behalf of one or more oracle
/// sources in a single transaction (#264).
///
/// Each leg is independently authorized by its `source` — the relayer bundles one
/// pre-signed authorization entry per leg alongside its own signature — while
/// `relayer` authorizes the batch once. Because a Soroban contract invocation is
/// atomic, any leg that fails validation aborts the *entire* batch and rolls back
/// every storage change made so far in this call: batches either fully succeed or
/// fully fail.
///
/// Legs implement the relayer priority fee market (#266): they must be supplied in
/// non-increasing `priority_fee` order. Because each source's authorization entry
/// covers the exact `priority_fee` it signed, a relayer cannot alter a source's fee
/// after the fact without invalidating that source's signature.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `relayer` - Approved relayer submitting the batch (authorizes once for all legs).
/// * `submissions` - Ordered batch of [`RelayedSubmission`] legs, non-empty and at
///   most [`MAX_BATCH_SIZE`] entries, sorted by non-increasing `priority_fee`.
///
/// # Errors
///
/// * [`ErrorCode::ContractPaused`] — contract is currently paused.
/// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
/// * [`ErrorCode::BatchEmpty`] — `submissions` is empty.
/// * [`ErrorCode::BatchTooLarge`] — `submissions` exceeds [`MAX_BATCH_SIZE`].
/// * [`ErrorCode::BatchNotFeePrioritized`] — legs are not ordered by non-increasing
///   `priority_fee`.
/// * Any error raised by [`submit_price_relayed`] for an individual leg.
pub fn submit_prices_relayed(env: &Env, relayer: Address, submissions: Vec<RelayedSubmission>) {
    check_not_paused(env);
    relayer.require_auth();

    let len = submissions.len();
    if len == 0 {
        panic_with_error!(env, ErrorCode::BatchEmpty);
    }
    if len > MAX_BATCH_SIZE {
        panic_with_error!(env, ErrorCode::BatchTooLarge);
    }

    validate_fee_priority_order(env, &submissions);

    let mut total_priority_fee: u128 = 0;
    for i in 0..len {
        let leg = submissions.get_unchecked(i);
        leg.source.require_auth();

        if process_relayed_leg(
            env,
            &relayer,
            &leg.source,
            &leg.asset,
            leg.price,
            leg.timestamp,
        ) {
            accrue_relayer_submission_metrics(env, &relayer, leg.priority_fee);
            total_priority_fee = total_priority_fee.saturating_add(leg.priority_fee);
        }
    }

    BatchPriceRelayedEvent {
        relayer,
        submission_count: len,
        total_priority_fee,
    }
    .publish(env);
}

/// Panics with [`ErrorCode::BatchNotFeePrioritized`] unless `submissions` is ordered
/// by non-increasing `priority_fee` — the on-chain enforcement of "relayers process
/// higher-fee submissions first" (#266).
fn validate_fee_priority_order(env: &Env, submissions: &Vec<RelayedSubmission>) {
    let len = submissions.len();
    for i in 1..len {
        let prev = submissions.get_unchecked(i - 1);
        let curr = submissions.get_unchecked(i);
        if curr.priority_fee > prev.priority_fee {
            panic_with_error!(env, ErrorCode::BatchNotFeePrioritized);
        }
    }
}

/// Validates and stores a single relayed price leg, shared by [`submit_price_relayed`]
/// and [`submit_prices_relayed`]. Returns `true` if the price was stored and aggregated,
/// or `false` if it was silently dropped by the deviation circuit breaker.
///
/// Does *not* perform authorization — callers must `require_auth` both `relayer` and
/// `source` (or each leg's source, for batches) before calling this.
fn process_relayed_leg(
    env: &Env,
    relayer: &Address,
    source: &Address,
    asset: &Address,
    price: i128,
    timestamp: u64,
) -> bool {
    // Relayer approval can come either from the admin registry or from a valid
    // source-issued delegation with an expiry ledger.
    if !is_relayer(env, relayer.clone()) && !has_valid_relayer_delegation(env, source, relayer) {
        panic_with_error!(env, ErrorCode::RelayerNotAuthorized);
    }

    // Relayer must have posted its required performance bond, if configured (#265).
    let required_bond = crate::relayer_bonds::get_relayer_bond_amount(env);
    if required_bond > 0 {
        let deposited = crate::relayer_bonds::get_relayer_bond_balance(env, relayer.clone());
        if deposited < required_bond {
            panic_with_error!(env, ErrorCode::RelayerBondInsufficient);
        }
    }

    // Standard source and asset checks.
    check_source(env, source);
    check_registered_asset(env, asset);
    check_source_asset(env, source, asset);

    if is_source_suspended(env, source.clone()) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    // Price validation.
    if price <= 0 {
        record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let min_price = get_min_price(env, asset.clone());
    if price < min_price {
        panic_with_error!(env, ErrorCode::PriceBelowMinimum);
    }

    // Timestamp validation.
    let ledger_time = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    if timestamp > ledger_time.saturating_add(threshold) {
        record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    // Persist the price entry under the same storage key as a direct submission
    // so aggregation logic treats it identically.
    if crate::prices::check_deviation_circuit_breaker(env, source, asset, price) {
        return false;
    }

    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();
    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
        ledger_timestamp: ledger_time,
        volume: None,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

    crate::prices::record_successful_submission(env, source.clone());

    // Emit the relayed-submission event before aggregation.
    PriceRelayedEvent {
        asset: asset.clone(),
        source: source.clone(),
        relayer: relayer.clone(),
        price,
        timestamp,
    }
    .publish(env);

    // Trigger aggregation (shared with the direct submit_price path).
    do_aggregate(env, asset);

    crate::relayer_dashboard::record_relayer_submission_stats(
        env,
        relayer,
        asset,
        timestamp,
        ledger_time,
    );

    true
}

/// Accrues per-submission relayer metrics: submission count, flat + priority fee
/// balance, and accuracy-weighted reward (#265, #266).
fn accrue_relayer_submission_metrics(env: &Env, relayer: &Address, priority_fee: u128) {
    let count_key = DataKey::RelayerSubmissionCount(relayer.clone());
    let count: u64 = env.storage().persistent().get(&count_key).unwrap_or(0u64);
    let new_count = count.saturating_add(1);
    env.storage().persistent().set(&count_key, &new_count);
    env.storage()
        .persistent()
        .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let flat_fee = get_relayer_fee_per_submission(env);
    let priority_fee_i128 = i128::try_from(priority_fee).unwrap_or(i128::MAX);
    let total_fee = flat_fee.saturating_add(priority_fee_i128);
    if total_fee > 0 {
        let balance_key = DataKey::RelayerFeeBalance(relayer.clone());
        let balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key)
            .unwrap_or(0i128);
        env.storage()
            .persistent()
            .set(&balance_key, &balance.saturating_add(total_fee));
        env.storage()
            .persistent()
            .extend_ttl(&balance_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    crate::relayer_bonds::accrue_relayer_reward(env, relayer, new_count);
}

// ---------------------------------------------------------------------------
// Relayer fee management
// ---------------------------------------------------------------------------

/// Sets the fee credited to a relayer (in stroops) for each successful relayed submission.
///
/// The fee accrues in [`DataKey::RelayerFeeBalance`] and can be read via
/// [`get_relayer_fee_balance`]. Actual settlement is handled off-chain or via a
/// separate token-contract integration.
///
/// Setting `fee` to `0` disables fee accrual.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
pub fn set_relayer_fee_per_submission(env: &Env, fee: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::RelayerFeePerSubmission, &fee);
    emit_relayer_fee_set(env, admin, fee);
}

/// Returns the current fee per relayed submission in stroops. Defaults to `0`.
pub fn get_relayer_fee_per_submission(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::RelayerFeePerSubmission)
        .unwrap_or(0i128)
}

/// Returns the total accumulated fee balance (in stroops) owed to `relayer`.
pub fn get_relayer_fee_balance(env: &Env, relayer: Address) -> i128 {
    let key = DataKey::RelayerFeeBalance(relayer);
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

/// Returns the total number of successful relayed submissions made by `relayer`.
pub fn get_relayer_submission_count(env: &Env, relayer: Address) -> u64 {
    let key = DataKey::RelayerSubmissionCount(relayer);
    env.storage().persistent().get(&key).unwrap_or(0u64)
}

/// Challenges a relayed submission as unauthorized if the source never granted a
/// valid delegation to the relayer, or the delegation expired before the submission.
/// On a valid challenge, the relayer's bond is slashed and the challenger is credited.
pub fn challenge_relayed_submission(
    env: &Env,
    challenger: Address,
    relayer: Address,
    source: Address,
    asset: Address,
    price: i128,
    timestamp: u64,
    proof_data: Bytes,
) {
    check_source(env, &source);
    check_registered_asset(env, &asset);
    check_source_asset(env, &source, &asset);

    let valid_auth = is_relayer(env, relayer.clone()) || has_valid_relayer_delegation(env, &source, &relayer);
    if valid_auth {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let current_submission: Option<PriceEntry> = env
        .storage()
        .persistent()
        .get(&DataKey::Submission(asset.clone(), source.clone()));
    match current_submission {
        Some(existing) => {
            if existing.price != price || existing.timestamp != timestamp {
                // Challenge still proves unauthorized behavior even if price data differs,
                // but caller must provide a valid proof object to certify the mismatch.
            }
        }
        None => {}
    }

    if proof_data.len() == 0 {
        panic_with_error!(env, ErrorCode::InvalidProof);
    }

    let bond = crate::relayer_bonds::get_relayer_bond_balance(env, relayer.clone());
    if bond <= 0 {
        panic_with_error!(env, ErrorCode::InsufficientBond);
    }

    let slash_percent = crate::relayer_bonds::get_relayer_slash_percent(env) as i128;
    let slash_amount = ((bond * slash_percent) / 100).max(1).min(bond);
    let new_balance = bond - slash_amount;

    env.storage()
        .persistent()
        .set(&DataKey::RelayerBond(relayer.clone()), &new_balance);
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::RelayerBond(relayer.clone()), LEDGER_THRESHOLD, LEDGER_BUMP);

    let treasury = env
        .storage()
        .persistent()
        .get(&DataKey::TreasuryBalance)
        .unwrap_or(0i128);
    env.storage()
        .persistent()
        .set(&DataKey::TreasuryBalance, &(treasury.saturating_add(slash_amount)));
    env.storage()
        .persistent()
        .extend_ttl(&DataKey::TreasuryBalance, LEDGER_THRESHOLD, LEDGER_BUMP);

    let reward = slash_amount / 2;
    let existing = env
        .storage()
        .persistent()
        .get(&DataKey::ChallengerRewards(challenger.clone()))
        .unwrap_or(0i128);
    env.storage()
        .persistent()
        .set(&DataKey::ChallengerRewards(challenger), &(existing.saturating_add(reward)));

    crate::relayer_bonds::record_relayer_failure(env, relayer.clone(), crate::types::RelayerFailureReason::UnauthorizedPrice);
}
