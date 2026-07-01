use soroban_sdk::{panic_with_error, Address, Env, String};

use crate::admin::{get_decimals, get_timestamp_threshold};
use crate::assets::get_min_price;
use crate::events::{emit_relayer_fee_set, PriceRelayedEvent, RelayerAddedEvent, RelayerRemovedEvent};
use crate::pause::check_not_paused;
use crate::prices::do_aggregate;
use crate::sources::{is_source_suspended, record_invalid_submission};
use crate::storage::{check_registered_asset, check_source, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, PriceEntry, RelayerInfo};

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

    // Relayer must be admin-approved.
    if !is_relayer(env, relayer.clone()) {
        panic_with_error!(env, ErrorCode::RelayerNotAuthorized);
    }

    // Standard source and asset checks.
    check_source(env, &source);
    check_registered_asset(env, &asset);

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
    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();
    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

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
    do_aggregate(env, &asset);

    // Track relayer metrics: submission count and accrued fee balance.
    let count_key = DataKey::RelayerSubmissionCount(relayer.clone());
    let count: u64 = env
        .storage()
        .persistent()
        .get(&count_key)
        .unwrap_or(0u64);
    env.storage()
        .persistent()
        .set(&count_key, &count.saturating_add(1));
    env.storage()
        .persistent()
        .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let fee = get_relayer_fee_per_submission(env);
    if fee > 0 {
        let balance_key = DataKey::RelayerFeeBalance(relayer.clone());
        let balance: i128 = env
            .storage()
            .persistent()
            .get(&balance_key)
            .unwrap_or(0i128);
        env.storage()
            .persistent()
            .set(&balance_key, &balance.saturating_add(fee));
        env.storage()
            .persistent()
            .extend_ttl(&balance_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
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
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(0i128)
}

/// Returns the total number of successful relayed submissions made by `relayer`.
pub fn get_relayer_submission_count(env: &Env, relayer: Address) -> u64 {
    let key = DataKey::RelayerSubmissionCount(relayer);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(0u64)
}
