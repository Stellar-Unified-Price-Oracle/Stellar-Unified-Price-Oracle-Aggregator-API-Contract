#![no_std]

mod admin;
mod assets;
mod cross_reference;
mod errors;
mod events;
mod health;
mod history;
mod pause;
mod prices;
mod reentrancy;
mod sources;
mod storage;
mod timelock;
mod types;

#[cfg(test)]
mod cross_ref_tests;

#[cfg(test)]
mod override_tests;

#[cfg(test)]
mod prop_tests;

#[cfg(test)]
mod string_boundary_tests;

pub use types::{
    AggregatePrice, AggregationMethod, Asset, BatchOperation, DataKey, ErrorCode, OracleSources,
    PendingBatch, PriceData, PriceEntry, PriceHistoryEntry, PriceOverrideEntry,
};

use soroban_sdk::{contract, contractimpl, panic_with_error, Address, Env, String, Symbol, Vec};

use crate::storage::read_registered_assets;

/// Stellar Unified Price Oracle — a multi-source, aggregating price oracle smart contract.
///
/// The contract collects price submissions from a set of whitelisted oracle sources, aggregates
/// them (median by default), and exposes both a native query API and a SEP-40 compatible
/// interface. Administrative functions are protected by admin authentication, and sensitive
/// governance operations are additionally gated behind a configurable timelock.
#[contract]
pub struct PriceOracleContract;

#[contractimpl]
impl PriceOracleContract {
    // --- Admin ---

    /// Initializes the contract with its first administrator and global configuration.
    ///
    /// This function must be called exactly once after deployment. The calling `admin`
    /// address must authorize the invocation. Subsequent calls will panic.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `admin` - Address that will hold administrator privileges. Must authorize this call.
    /// * `min_sources_required` - Minimum number of contributing sources needed before an
    ///   aggregate price is published. Falls back to `1` when `0` is passed.
    /// * `max_history_length` - Maximum number of history entries retained per asset before
    ///   the oldest is pruned. Falls back to `100` when `0` is passed.
    /// * `decimals` - Fixed decimal precision applied to all prices stored in this oracle.
    /// * `description` - Human-readable description of this oracle instance (max 256 chars).
    ///
    /// # Panics
    ///
    /// * [`ErrorCode::AlreadyInitialized`] — if the contract has already been initialized.
    /// * [`ErrorCode::DescriptionTooLong`] — if `description` exceeds 256 characters.
    pub fn initialize(
        env: Env,
        admin: Address,
        min_sources_required: u32,
        max_history_length: u32,
        decimals: u32,
        description: String,
    ) {
        reentrancy::enter(&env);
        admin::initialize(
            &env,
            admin,
            min_sources_required,
            max_history_length,
            decimals,
            description,
        );
        reentrancy::exit(&env);
    }

    /// Replaces the contract's WASM with a new hash, upgrading the on-chain logic.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `new_wasm_hash` - 32-byte hash of the WASM module to upgrade to.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) {
        reentrancy::enter(&env);
        admin::upgrade(&env, new_wasm_hash);
        reentrancy::exit(&env);
    }

    /// Transfers administrator privileges to a new address.
    ///
    /// The current admin must authorize this call. The new admin takes effect immediately.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `new_admin` - Address that will become the new administrator.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_admin(env: Env, new_admin: Address) {
        reentrancy::enter(&env);
        admin::set_admin(&env, new_admin);
        reentrancy::exit(&env);
    }

    /// Returns the current administrator's address.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// The `Address` of the current admin.
    pub fn get_admin_address(env: Env) -> Address {
        admin::get_admin_address(&env)
    }

    /// Updates the minimum number of oracle sources required before a price is aggregated.
    ///
    /// The new value must be greater than zero and must not exceed the total number of
    /// currently registered sources (when sources are already present).
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `new_min` - New minimum-sources threshold (must be ≥ 1).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `new_min` is `0` or exceeds the
    ///   number of currently registered sources.
    pub fn set_min_sources_required(env: Env, new_min: u32) {
        reentrancy::enter(&env);
        admin::set_min_sources_required(&env, new_min);
        reentrancy::exit(&env);
    }

    /// Returns the current minimum-sources threshold.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Minimum number of sources required for aggregation. Defaults to `1`.
    pub fn get_min_sources_required(env: Env) -> u32 {
        admin::get_min_sources_required(&env)
    }

    /// Updates the maximum number of historical price entries retained per asset.
    ///
    /// When a new aggregate is written and the history exceeds this limit, the oldest
    /// entry is pruned and a [`HistoryPrunedEvent`](crate::events::HistoryPrunedEvent) is emitted.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `new_max` - New maximum history length.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_max_history_length(env: Env, new_max: u32) {
        reentrancy::enter(&env);
        admin::set_max_history_length(&env, new_max);
        reentrancy::exit(&env);
    }

    /// Returns the current maximum history length.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Maximum number of history entries kept per asset. Defaults to `100`.
    pub fn get_max_history_length(env: Env) -> u32 {
        admin::get_max_history_length(&env)
    }

    /// Sets the price resolution window in seconds (SEP-40 `resolution` field).
    ///
    /// When `resolution > 0`, [`get_price`] and the SEP-40 read methods return `None`
    /// for prices whose timestamp falls outside the window.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `new_resolution` - Resolution window in seconds. Use `0` to disable staleness
    ///   filtering by resolution.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_resolution(env: Env, new_resolution: u32) {
        reentrancy::enter(&env);
        admin::set_resolution(&env, new_resolution);
        reentrancy::exit(&env);
    }

    /// Returns the current price resolution window in seconds.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Resolution in seconds, or `0` if not set. Defaults to `0`.
    pub fn get_resolution(env: Env) -> u32 {
        admin::get_resolution(&env)
    }

    /// Updates the decimal precision used for all prices stored by this oracle.
    ///
    /// Changing decimals does **not** retroactively rescale existing price entries.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `new_decimals` - New decimal precision (e.g. `18` means prices are in units of
    ///   `10^-18`).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_decimals(env: Env, new_decimals: u32) {
        reentrancy::enter(&env);
        admin::set_decimals(&env, new_decimals);
        reentrancy::exit(&env);
    }

    /// Returns the contract-wide decimal precision.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Number of decimals. Defaults to `18`.
    pub fn get_decimals(env: Env) -> u32 {
        admin::get_decimals(&env)
    }

    /// Updates the human-readable description of this oracle instance.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `new_description` - New description string (max 256 characters).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::DescriptionTooLong`] — if the string exceeds 256 characters.
    pub fn set_description(env: Env, new_description: String) {
        reentrancy::enter(&env);
        admin::set_description(&env, new_description);
        reentrancy::exit(&env);
    }

    /// Returns the current oracle description string.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// The description `String`. Defaults to `"Stellar Price Oracle"`.
    pub fn get_description(env: Env) -> String {
        admin::get_description(&env)
    }

    /// Sets the maximum allowed gap (in seconds) between a submitted timestamp and
    /// the current ledger time.
    ///
    /// Submissions with a timestamp more than `threshold` seconds ahead of the ledger
    /// clock are rejected with [`ErrorCode::InvalidTimestamp`].
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `threshold` - Maximum tolerated future timestamp offset in seconds.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_timestamp_threshold(env: Env, threshold: u64) {
        reentrancy::enter(&env);
        admin::set_timestamp_threshold(&env, threshold);
        reentrancy::exit(&env);
    }

    /// Returns the current timestamp validity threshold in seconds.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Threshold in seconds. Defaults to `300` (5 minutes).
    pub fn get_timestamp_threshold(env: Env) -> u64 {
        admin::get_timestamp_threshold(&env)
    }

    /// Sets the maximum allowed price deviation, expressed in basis points (100 bp = 1 %).
    ///
    /// Submissions that deviate from the current aggregate by more than this amount are
    /// flagged. Must be in the range `[0, 100_000]`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `deviation_basis_points` - Deviation ceiling in basis points (max `100_000`).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `deviation_basis_points > 100_000`.
    pub fn set_max_price_deviation(env: Env, deviation_basis_points: u32) {
        reentrancy::enter(&env);
        admin::set_max_price_deviation(&env, deviation_basis_points);
        reentrancy::exit(&env);
    }

    /// Returns the current maximum price deviation in basis points.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Maximum deviation in basis points. Defaults to `500` (5 %).
    pub fn get_max_price_deviation(env: Env) -> u32 {
        admin::get_max_price_deviation(&env)
    }

    /// Sets the heartbeat interval — the period after which a silent source is considered
    /// inactive.
    ///
    /// Must be greater than zero.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `interval` - Heartbeat interval in seconds (must be ≥ 1).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `interval` is `0`.
    pub fn set_heartbeat_interval(env: Env, interval: u64) {
        reentrancy::enter(&env);
        admin::set_heartbeat_interval(&env, interval);
        reentrancy::exit(&env);
    }

    /// Returns the current heartbeat interval in seconds.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Heartbeat interval in seconds. Defaults to `3600` (1 hour).
    pub fn get_heartbeat_interval(env: Env) -> u64 {
        admin::get_heartbeat_interval(&env)
    }

    // --- #67: Per-asset resolution ---

    /// Sets a per-asset resolution override in seconds.
    ///
    /// When set, `get_price` and SEP-40 `lastprice` use this value instead of the
    /// contract-wide resolution for the given asset. Pass `0` to clear the override
    /// (reverts to contract-wide resolution).
    pub fn set_asset_resolution(env: Env, asset: Address, resolution: u32) {
        admin::set_asset_resolution(&env, asset, resolution);
    }

    /// Returns the effective resolution in seconds for an asset.
    ///
    /// Returns the per-asset override if set, otherwise the contract-wide resolution.
    pub fn get_asset_resolution(env: Env, asset: Address) -> u32 {
        admin::get_asset_resolution(&env, asset)
    }

    // --- #69: Periodic aggregation trigger ---

    /// Triggers a price aggregation re-computation for an asset.
    ///
    /// Callable by anyone. Subject to the configured aggregation cooldown.
    /// Panics with [`ErrorCode::InvalidConfiguration`] if called within the cooldown,
    /// or [`ErrorCode::InsufficientSources`] if too few compliant sources exist.
    pub fn trigger_aggregation(env: Env, asset: Address) {
        prices::trigger_aggregation(&env, asset);
    }

    /// Sets the minimum number of ledgers that must elapse between `trigger_aggregation` calls.
    pub fn set_aggregation_cooldown(env: Env, cooldown_ledgers: u32) {
        admin::set_aggregation_cooldown(&env, cooldown_ledgers);
    }

    /// Returns the current aggregation cooldown in ledgers. Defaults to `10`.
    pub fn get_aggregation_cooldown(env: Env) -> u32 {
        admin::get_aggregation_cooldown(&env)
    }

    // --- #70: Min submission interval ---

    /// Sets the minimum submission interval in ledgers.
    ///
    /// Sources that have not submitted within this many ledgers since their last
    /// submission are excluded from aggregation and flagged as non-compliant.
    /// Set to `0` to disable enforcement (default).
    pub fn set_min_submission_interval(env: Env, interval_ledgers: u32) {
        admin::set_min_submission_interval(&env, interval_ledgers);
    }

    /// Returns the current minimum submission interval in ledgers. Defaults to `0` (disabled).
    pub fn get_min_submission_interval(env: Env) -> u32 {
        admin::get_min_submission_interval(&env)
    }

    /// Returns the list of sources currently compliant with the submission interval for an asset.
    pub fn get_compliant_sources(env: Env, asset: Address) -> Vec<Address> {
        prices::get_compliant_sources(&env, asset)
    }

    // --- #68: Batch operations ---

    /// Proposes a batch of admin operations to be executed atomically after the timelock delay.
    ///
    /// Returns the unique batch ID. Each `BatchOperation` carries an `op_type` (0–7) and
    /// encoded `data` matching the same format as `propose_operation`.
    pub fn propose_batch(env: Env, operations: Vec<BatchOperation>) -> u32 {
        timelock::propose_batch(&env, operations)
    }

    /// Executes a proposed batch after its timelock delay has elapsed.
    ///
    /// All operations run sequentially. Any failure rolls back the entire transaction.
    pub fn execute_batch(env: Env, batch_id: u32) {
        timelock::execute_batch(&env, batch_id);
    }

    /// Cancels a pending batch operation without executing it.
    pub fn cancel_batch(env: Env, batch_id: u32) {
        timelock::cancel_batch(&env, batch_id);
    }

    // --- Sources ---

    /// Registers a new oracle source authorized to submit prices.
    ///
    /// The admin must authorize this call. The source address must not already be registered.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address of the oracle source to register.
    /// * `name` - Human-readable display name for the source.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::SourceAlreadyExists`] — if `source` is already registered.
    pub fn add_source(env: Env, source: Address, name: String) {
        reentrancy::enter(&env);
        sources::add_source(&env, source, name);
        reentrancy::exit(&env);
    }

    /// Removes an oracle source from the authorized set.
    ///
    /// The admin must authorize this call. Existing price submissions from the source
    /// are not deleted but will no longer contribute to future aggregations.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address of the oracle source to remove.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::SourceNotFound`] — if `source` is not currently registered.
    pub fn remove_source(env: Env, source: Address) {
        reentrancy::enter(&env);
        sources::remove_source(&env, source);
        reentrancy::exit(&env);
    }

    /// Returns whether the given address is a registered oracle source.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address to query.
    ///
    /// # Returns
    ///
    /// `true` if `source` is registered; `false` otherwise.
    pub fn is_source(env: Env, source: Address) -> bool {
        sources::is_source(&env, source)
    }

    /// Returns the full registry of oracle sources and their metadata.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// An [`OracleSources`] struct containing all source addresses and their display names.
    pub fn get_oracle_sources(env: Env) -> OracleSources {
        sources::get_oracle_sources(&env)
    }

    /// Records a liveness heartbeat for a source, resetting its inactivity timer.
    ///
    /// The `source` address must authorize this call. If the source was previously marked
    /// inactive, it is restored to active status and a
    /// [`SourceActiveAgainEvent`](crate::events::SourceActiveAgainEvent) is emitted.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address of the oracle source submitting the heartbeat.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::SourceNotFound`] — if `source` is not a registered oracle source.
    pub fn submit_heartbeat(env: Env, source: Address) {
        reentrancy::enter(&env);
        sources::submit_heartbeat(&env, source);
        reentrancy::exit(&env);
    }

    /// Returns whether the given source is currently considered inactive.
    ///
    /// A source is inactive if it has been explicitly marked so, or if the time elapsed
    /// since its last heartbeat exceeds the configured heartbeat interval.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address of the oracle source to check.
    ///
    /// # Returns
    ///
    /// `true` if the source is inactive; `false` otherwise.
    pub fn is_source_inactive(env: Env, source: Address) -> bool {
        sources::is_source_inactive(&env, source)
    }

    /// Returns the number of oracle sources currently classified as inactive.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Count of inactive sources among all registered sources.
    pub fn get_inactive_sources(env: Env) -> u32 {
        sources::get_inactive_sources(&env)
    }

    /// Returns the Unix timestamp of the last heartbeat submitted by a source.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address of the oracle source to query.
    ///
    /// # Returns
    ///
    /// Unix timestamp (seconds) of the last heartbeat, or `0` if none has been submitted.
    pub fn get_source_last_heartbeat(env: Env, source: Address) -> u64 {
        sources::get_source_last_heartbeat(&env, source)
    }

    // --- #65: Source Reputation ---

    pub fn get_source_reputation(env: Env, source: Address) -> i128 {
        sources::get_source_reputation(&env, source)
    }

    pub fn set_reputation_decay_factor(env: Env, factor: u32) {
        sources::set_reputation_decay_factor(&env, factor);
    }

    pub fn get_reputation_decay_factor(env: Env) -> u32 {
        sources::get_reputation_decay_factor(&env)
    }

    // --- #66: Phased Source Removal ---

    pub fn mark_source_for_removal(env: Env, source: Address) {
        sources::mark_source_for_removal(&env, source);
    }

    pub fn cancel_source_removal(env: Env, source: Address) {
        sources::cancel_source_removal(&env, source);
    }

    pub fn finalize_source_removal(env: Env, source: Address) {
        sources::finalize_source_removal(&env, source);
    }

    pub fn set_removal_cooldown(env: Env, ledgers: u32) {
        sources::set_removal_cooldown(&env, ledgers);
    }

    pub fn get_removal_cooldown(env: Env) -> u32 {
        sources::get_removal_cooldown(&env)
    }

    pub fn is_source_pending_removal(env: Env, source: Address) -> bool {
        sources::is_source_pending_removal(&env, source)
    }

    // --- Assets ---

    /// Sets the maximum number of assets that can be registered.
    ///
    /// Admin must authorize this call.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `count` is `0`.
    pub fn set_max_assets(env: Env, count: u32) {
        admin::set_max_assets(&env, count);
    }

    /// Returns the configured maximum number of assets that can be registered.
    ///
    /// Defaults to `100`.
    pub fn get_max_assets(env: Env) -> u32 {
        admin::get_max_assets(&env)
    }

    /// Registers an asset so it can receive price submissions.
    ///
    /// The admin must authorize this call. An asset cannot receive prices until it is
    /// registered.

    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the Stellar token to register.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::AssetAlreadyRegistered`] — if the asset is already registered.
    pub fn register_asset(env: Env, asset: Address) {
        reentrancy::enter(&env);
        assets::register_asset(&env, asset);
        reentrancy::exit(&env);
    }

    /// Removes an asset from the registry and deletes its aggregate price entry.
    ///
    /// The admin must authorize this call. Historical entries stored in temporary
    /// storage are not explicitly removed but will expire naturally.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset to unregister.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::AssetNotRegistered`] — if the asset is not currently registered.
    pub fn unregister_asset(env: Env, asset: Address) {
        reentrancy::enter(&env);
        assets::unregister_asset(&env, asset);
        reentrancy::exit(&env);
    }

    /// Returns whether the given asset contract address is currently registered.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Asset contract address to query.
    ///
    /// # Returns
    ///
    /// `true` if registered; `false` otherwise.
    pub fn is_asset_registered(env: Env, asset: Address) -> bool {
        assets::is_asset_registered(&env, asset)
    }

    // --- Prices ---

    /// Submits a price observation for an asset from an authorized oracle source.
    ///
    /// The `source` address must authorize this call. After storing the individual
    /// submission, the contract re-aggregates all available source prices. If the
    /// number of contributing sources meets `min_sources_required`, the aggregate is
    /// updated and a history entry is recorded.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address of the submitting oracle source. Must authorize this call.
    /// * `asset` - Contract address of the asset being priced.
    /// * `price` - Raw price value scaled by `10^decimals`. Must be greater than `0`.
    /// * `timestamp` - Unix timestamp (seconds) of the observation. Must not exceed
    ///   `ledger_time + timestamp_threshold`.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::ContractPaused`] — if the contract is currently paused.
    /// * [`ErrorCode::NotAuthorized`] — if the source is suspended or not authorized.
    /// * [`ErrorCode::SourceNotFound`] — if `source` is not a registered oracle source.
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    /// * [`ErrorCode::InvalidPrice`] — if `price` is ≤ 0.
    /// * [`ErrorCode::PriceBelowMinimum`] — if `price` is below the asset's minimum price.
    /// * [`ErrorCode::InvalidTimestamp`] — if `timestamp` is too far in the future.
    pub fn submit_price(env: Env, source: Address, asset: Address, price: i128, timestamp: u64) {
        reentrancy::enter(&env);
        prices::submit_price(&env, source, asset, price, timestamp);
        reentrancy::exit(&env);
    }

    /// Submits prices for multiple assets in a single atomic transaction.
    ///
    /// Authorization is checked once for `source`. All entries are validated before any
    /// are written — if any entry fails validation the entire call panics (all-or-nothing).
    /// Aggregation is triggered for each asset after all submissions are stored.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `source` - Address of the submitting oracle source. Must authorize this call.
    /// * `asset_prices` - List of `(asset, price, timestamp)` tuples to submit.
    ///
    /// # Errors
    ///
    /// Same error conditions as `submit_price`, applied per entry.
    pub fn submit_prices(env: Env, source: Address, asset_prices: Vec<(Address, i128, u64)>) {
        prices::submit_prices(&env, source, asset_prices);
    }

    /// Returns the latest aggregate price for an asset, filtered by a maximum age.
    ///
    /// When `max_age > 0`, returns `None` and emits a
    /// [`PriceStaleEvent`](crate::events::PriceStaleEvent) if the price timestamp is older
    /// than `ledger_time - max_age`. The configured `resolution` window is applied
    /// independently; both filters must pass.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset to query.
    /// * `max_age` - Maximum acceptable age of the price in seconds. Use `0` to disable
    ///   the age check (resolution filtering still applies).
    ///
    /// # Returns
    ///
    /// `Some(`[`AggregatePrice`]`)` if a fresh aggregate exists; `None` otherwise.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    pub fn get_price(env: Env, asset: Address, max_age: u64) -> Option<AggregatePrice> {
        prices::get_price(&env, asset, max_age)
    }

    /// Returns the most recent price submission from a specific oracle source for an asset.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset.
    /// * `source` - Address of the oracle source.
    ///
    /// # Returns
    ///
    /// The [`PriceEntry`] submitted by `source` for `asset`.
    ///
    /// # Panics
    ///
    /// Panics if no submission exists for the (`asset`, `source`) pair (via `unwrap`).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    /// * [`ErrorCode::SourceNotFound`] — if `source` is not registered.
    pub fn get_source_price(env: Env, asset: Address, source: Address) -> PriceEntry {
        prices::get_source_price(&env, asset, source)
    }

    /// Returns all price submissions currently stored for an asset, one per source.
    ///
    /// Only sources that have at least one stored submission for `asset` are included.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset.
    ///
    /// # Returns
    ///
    /// A [`Vec`] of [`PriceEntry`] values, one per contributing source.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    pub fn get_all_prices(env: Env, asset: Address) -> Vec<PriceEntry> {
        prices::get_all_prices(&env, asset)
    }

    pub fn override_price(
        env: Env,
        asset: Address,
        price: i128,
        reason: String,
        expiry_ledger: u32,
    ) {
        prices::override_price(&env, asset, price, reason, expiry_ledger);
    }

    pub fn remove_price_override(env: Env, asset: Address) {
        prices::remove_price_override(&env, asset);
    }

    pub fn get_price_override(env: Env, asset: Address) -> Option<PriceOverrideEntry> {
        prices::get_price_override(&env, asset)
    }

    pub fn get_latest_ledger(env: Env) -> u32 {
        env.ledger().sequence()
    }

    // --- History ---

    /// Returns the historical price snapshot recorded at a specific ledger.
    ///
    /// History is stored in temporary storage and expires after the configured TTL.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset.
    /// * `ledger` - Ledger sequence number of the desired snapshot.
    ///
    /// # Returns
    ///
    /// The [`PriceHistoryEntry`] recorded at `ledger`.
    ///
    /// # Panics
    ///
    /// Panics if no history entry exists at the specified ledger (via `unwrap`).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    pub fn get_historical_price(env: Env, asset: Address, ledger: u32) -> PriceHistoryEntry {
        history::get_historical_price(&env, asset, ledger)
    }

    /// Returns whether a price history entry exists for an asset at a specific ledger.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset.
    /// * `ledger` - Ledger sequence number to check.
    ///
    /// # Returns
    ///
    /// `true` if a snapshot exists at `ledger`; `false` otherwise (including when
    /// the asset is not registered).
    pub fn has_historical_price(env: Env, asset: Address, ledger: u32) -> bool {
        history::has_historical_price(&env, asset, ledger)
    }

    /// Returns all historical price snapshots for an asset within a ledger range.
    ///
    /// Only ledgers that actually contain a snapshot are included in the result.
    /// The range `[start_ledger, end_ledger]` must not exceed `max_history_length`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset.
    /// * `start_ledger` - First ledger in the range (inclusive).
    /// * `end_ledger` - Last ledger in the range (inclusive).
    ///
    /// # Returns
    ///
    /// A [`Vec`] of [`PriceHistoryEntry`] values for every ledger in the range that
    /// has a stored snapshot.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    /// * [`ErrorCode::NoData`] — if `end_ledger - start_ledger` exceeds `max_history_length`.
    pub fn get_historical_prices(
        env: Env,
        asset: Address,
        start_ledger: u32,
        end_ledger: u32,
    ) -> Vec<PriceHistoryEntry> {
        history::get_historical_prices(&env, asset, start_ledger, end_ledger)
    }

    /// Enables or disables linear interpolation for `get_historical_price` queries.
    ///
    /// When enabled, querying a ledger with no exact snapshot will return a
    /// linearly-interpolated estimate between the nearest surrounding data points.
    /// The result has `is_interpolated = true` so consumers can distinguish it
    /// from a real submission.
    ///
    /// Requires admin authorization.
    pub fn set_interpolation_enabled(env: Env, enabled: bool) {
        admin::set_interpolation_enabled(&env, enabled);
    }

    /// Returns whether linear interpolation is enabled for historical queries.
    pub fn get_interpolation_enabled(env: Env) -> bool {
        admin::get_interpolation_enabled(&env)
    }

    // --- SEP-40 Oracle Interface ---

    /// Returns the decimal precision used by this oracle (SEP-40 `decimals`).
    ///
    /// Identical to [`get_decimals`](Self::get_decimals).
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Number of decimals. Defaults to `18`.
    pub fn decimals(env: Env) -> u32 {
        Self::get_decimals(env)
    }

    /// Returns the base asset for all prices quoted by this oracle (SEP-40 `base`).
    ///
    /// Always returns `Asset::Other("USD")`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// [`Asset::Other`] with the symbol `"USD"`.
    pub fn base(env: Env) -> Asset {
        Asset::Other(Symbol::new(&env, "USD"))
    }

    /// Returns the list of all registered assets as SEP-40 [`Asset`] values (SEP-40 `assets`).
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// A [`Vec`] of [`Asset::Stellar`] wrapping each registered asset address.
    pub fn assets(env: Env) -> Vec<Asset> {
        let registered = read_registered_assets(&env);
        let mut result: Vec<Asset> = Vec::new(&env);
        for i in 0..registered.len() {
            result.push_back(Asset::Stellar(registered.get_unchecked(i)));
        }
        result
    }

    /// Returns the price resolution window in seconds (SEP-40 `resolution`).
    ///
    /// Identical to [`get_resolution`](Self::get_resolution).
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Resolution in seconds, or `0` if not configured.
    pub fn resolution(env: Env) -> u32 {
        admin::get_resolution(&env)
    }

    /// Returns the latest available price for an asset (SEP-40 `lastprice`).
    ///
    /// Returns `None` for non-Stellar asset variants, unregistered assets, or when
    /// the current aggregate is older than the configured resolution window.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - The asset to price. Non-`Stellar` variants always return `None`.
    ///
    /// # Returns
    ///
    /// `Some(`[`PriceData`]`)` with the latest aggregate price, or `None`.
    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        prices::lastprice(&env, asset)
    }

    /// Returns the price for an asset at or before the given Unix timestamp (SEP-40 `price`).
    ///
    /// First checks whether the current aggregate matches `timestamp` exactly; then
    /// searches backwards through the recent history (up to the last ~1000 ledgers).
    /// Returns `None` for non-Stellar assets, unregistered assets, or when no matching
    /// record is found.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - The asset to price. Non-`Stellar` variants always return `None`.
    /// * `timestamp` - Target Unix timestamp (seconds). The most recent entry whose
    ///   `timestamp ≤ this value` is returned.
    ///
    /// # Returns
    ///
    /// `Some(`[`PriceData`]`)` if a matching record is found; `None` otherwise.
    pub fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
        prices::price(&env, asset, timestamp)
    }

    /// Returns the most recent `records` price entries for an asset (SEP-40 `prices`).
    ///
    /// Walks backwards through recent history looking for up to `records` entries. If
    /// history is empty but an aggregate exists, falls back to returning a single entry
    /// derived from the current aggregate.
    ///
    /// Returns `None` for non-Stellar assets or unregistered assets. Returns
    /// `Some(empty Vec)` when `records` is `0`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - The asset to query. Non-`Stellar` variants always return `None`.
    /// * `records` - Maximum number of price records to return.
    ///
    /// # Returns
    ///
    /// `Some(`[`Vec<PriceData>`]`)` containing up to `records` entries in reverse
    /// chronological order, or `None`.
    pub fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
        prices::prices(&env, asset, records)
    }

    // --- Pause ---

    /// Pauses the contract, preventing new price submissions.
    ///
    /// While paused, any call to [`submit_price`](Self::submit_price) will fail with
    /// [`ErrorCode::ContractPaused`]. Read operations are unaffected.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn pause(env: Env) {
        reentrancy::enter(&env);
        pause::pause(&env);
        reentrancy::exit(&env);
    }

    /// Resumes the contract after it has been paused, re-enabling price submissions.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn unpause(env: Env) {
        reentrancy::enter(&env);
        pause::unpause(&env);
        reentrancy::exit(&env);
    }

    /// Returns whether the contract is currently paused.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// `true` if paused; `false` otherwise.
    pub fn is_paused(env: Env) -> bool {
        pause::is_paused(&env)
    }

    /// Returns a snapshot of the oracle's current health status.
    ///
    /// Aggregates information about registered sources, active sources, registered
    /// assets, assets with live prices, pause state, last aggregation ledger, stale
    /// price count, and suspended source count into a single [`HealthReport`].
    ///
    /// This is a read-only endpoint — no authentication required.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// A [`HealthReport`] reflecting current oracle state.
    pub fn health_check(env: Env) -> HealthReport {
        health::health_check(&env)
    }

    // --- Timelock ---

    /// Proposes a governance operation that will be executable after the timelock delay.
    ///
    /// The admin must authorize this call. The operation is assigned a unique ID and
    /// stored as a [`PendingOperation`](crate::types::PendingOperation). It cannot be
    /// executed until at least `timelock_duration` ledgers have elapsed.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `op_type` - Numeric discriminant identifying the operation type:
    ///   - `0` → Upgrade
    ///   - `1` → SetAdmin
    ///   - `2` → SetMinSources
    ///   - `3` → SetMaxHistory
    ///   - `4` → SetResolution
    ///   - `5` → SetDecimals
    ///   - `6` → SetDescription
    ///   - `7` → SetTimestampThreshold
    /// * `data` - Encoded payload whose interpretation depends on `op_type`.
    ///
    /// # Returns
    ///
    /// The unique `u32` ID assigned to the new pending operation.
    ///
    /// # Panics
    ///
    /// Panics with `"Invalid operation type"` if `op_type` is not in the range `[0, 7]`.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn propose_operation(env: Env, op_type: u32, data: soroban_sdk::Bytes) -> u32 {
        reentrancy::enter(&env);
        let op_enum = match op_type {
            0 => types::OperationType::Upgrade,
            1 => types::OperationType::SetAdmin,
            2 => types::OperationType::SetMinSources,
            3 => types::OperationType::SetMaxHistory,
            4 => types::OperationType::SetResolution,
            5 => types::OperationType::SetDecimals,
            6 => types::OperationType::SetDescription,
            7 => types::OperationType::SetTimestampThreshold,
            _ => panic_with_error!(&env, ErrorCode::InvalidOperationType),
        };
        let result = timelock::propose_operation(&env, op_enum, &data);
        reentrancy::exit(&env);
        result
    }

    /// Executes a previously proposed operation after its timelock delay has elapsed.
    ///
    /// The admin must authorize this call. The pending operation is removed from storage
    /// upon execution regardless of whether the underlying action succeeds.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `op_id` - ID of the pending operation to execute.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::OperationNotFound`] — if no pending operation with `op_id` exists.
    /// * [`ErrorCode::TimelockNotReady`] — if the required number of ledgers has not
    ///   elapsed since the operation was proposed.
    pub fn execute_operation(env: Env, op_id: u32) {
        reentrancy::enter(&env);
        timelock::execute_operation(&env, op_id);
        reentrancy::exit(&env);
    }

    /// Cancels a pending timelock operation, removing it without executing it.
    ///
    /// The admin must authorize this call.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `op_id` - ID of the pending operation to cancel.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::OperationNotFound`] — if no pending operation with `op_id` exists.
    pub fn cancel_operation(env: Env, op_id: u32) {
        reentrancy::enter(&env);
        timelock::cancel_operation(&env, op_id);
        reentrancy::exit(&env);
    }

    /// Returns the current timelock delay in ledgers.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Number of ledgers that must pass between proposing and executing an operation.
    /// Defaults to `10`.
    pub fn get_timelock_duration(env: Env) -> u32 {
        timelock::get_timelock_duration(&env)
    }

    /// Sets the timelock delay — the number of ledgers that must elapse between
    /// proposing and executing a governance operation.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `duration` - New timelock delay in ledgers.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_timelock_duration(env: Env, duration: u32) {
        reentrancy::enter(&env);
        timelock::set_timelock_duration(&env, duration);
        reentrancy::exit(&env);
    }

    // --- Relayer ---

    /// Approves a new relayer that can submit prices on behalf of oracle sources.
    ///
    /// Relayers are off-chain agents (inspired by IBC Hermes / Egypt) that bundle
    /// source-signed authorization entries and submit them to the contract. Only the
    /// admin may grant relayer approval.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Address to be approved as a relayer.
    /// * `name` - Human-readable display name for the relayer.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::RelayerAlreadyExists`] — if `relayer` is already approved.
    pub fn add_relayer(env: Env, relayer: Address, name: String) {
        relayer::add_relayer(&env, relayer, name);
    }

    /// Revokes a relayer's approval, preventing future relayed submissions.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Address of the relayer to revoke.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::RelayerNotAuthorized`] — if `relayer` is not currently approved.
    pub fn remove_relayer(env: Env, relayer: Address) {
        relayer::remove_relayer(&env, relayer);
    }

    /// Returns whether the given address is an approved relayer.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Address to query.
    ///
    /// # Returns
    ///
    /// `true` if `relayer` is approved; `false` otherwise.
    pub fn is_relayer(env: Env, relayer: Address) -> bool {
        relayer::is_relayer(&env, relayer)
    }

    /// Returns the [`RelayerInfo`] metadata for a given relayer, or `None` if not approved.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Address of the relayer to query.
    ///
    /// # Returns
    ///
    /// `Some(`[`RelayerInfo`]`)` with approval metadata, or `None` if not approved.
    pub fn get_relayer_info(env: Env, relayer: Address) -> Option<RelayerInfo> {
        relayer::get_relayer_info(&env, relayer)
    }

    /// Submits a price for an asset on behalf of an oracle source via an approved relayer.
    ///
    /// Both `relayer` and `source` must authorize this invocation. The source creates a
    /// Soroban [`AuthorizationEntry`] off-chain (pre-signing this exact call with the
    /// specific arguments), and the relayer bundles it into the transaction alongside its
    /// own signature.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Approved relayer submitting the transaction.
    /// * `source` - Registered oracle source whose price data is being relayed.
    /// * `asset` - Contract address of the asset being priced.
    /// * `price` - Raw price value scaled by `10^decimals`. Must be > 0.
    /// * `timestamp` - Unix timestamp (seconds) of the price observation.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::ContractPaused`] — contract is paused.
    /// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
    /// * [`ErrorCode::SourceNotFound`] — `source` is not a registered oracle source.
    /// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
    /// * [`ErrorCode::InvalidPrice`] — `price` is ≤ 0.
    /// * [`ErrorCode::PriceBelowMinimum`] — `price` is below asset's minimum.
    /// * [`ErrorCode::InvalidTimestamp`] — `timestamp` is too far in the future.
    pub fn submit_price_relayed(
        env: Env,
        relayer: Address,
        source: Address,
        asset: Address,
        price: i128,
        timestamp: u64,
    ) {
        relayer::submit_price_relayed(&env, relayer, source, asset, price, timestamp);
    }

    /// Sets the fee (in stroops) accrued to a relayer per successful relayed submission.
    ///
    /// Setting `fee` to `0` disables fee accrual. The admin must authorize this call.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `fee` - New fee per submission in stroops.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_relayer_fee_per_submission(env: Env, fee: i128) {
        relayer::set_relayer_fee_per_submission(&env, fee);
    }

    /// Returns the current fee per relayed submission in stroops. Defaults to `0`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Fee in stroops. `0` means no fee is currently configured.
    pub fn get_relayer_fee_per_submission(env: Env) -> i128 {
        relayer::get_relayer_fee_per_submission(&env)
    }

    /// Returns the total accumulated fee balance (in stroops) owed to `relayer`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Address of the relayer to query.
    ///
    /// # Returns
    ///
    /// Accumulated fee in stroops. `0` if the relayer has never submitted or no fee is set.
    pub fn get_relayer_fee_balance(env: Env, relayer: Address) -> i128 {
        relayer::get_relayer_fee_balance(&env, relayer)
    }

    /// Returns the total number of successful relayed submissions by `relayer`.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Address of the relayer to query.
    ///
    /// # Returns
    ///
    /// Submission count. `0` if no relayed submissions have been made.
    pub fn get_relayer_submission_count(env: Env, relayer: Address) -> u64 {
        relayer::get_relayer_submission_count(&env, relayer)
    }

    // --- Cross-Reference Oracle ---

    /// Registers an external oracle contract for cross-reference price verification.
    ///
    /// The `asset_mapping` maps each of our asset `Address` values to the corresponding
    /// asset `Address` used by the external oracle. On each
    /// [`get_cross_reference`](Self::get_cross_reference) call the contract invokes
    /// `lastprice(asset: Address) -> i128` on the registered oracle and compares the result.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `contract_id` - Contract address of the external reference oracle.
    /// * `asset_mapping` - Map from our asset addresses to the reference oracle's addresses.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn add_reference_oracle(
        env: Env,
        contract_id: Address,
        asset_mapping: Map<Address, Address>,
    ) {
        cross_reference::add_reference_oracle(&env, contract_id, asset_mapping);
    }

    /// Removes a previously registered reference oracle.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `contract_id` - Contract address of the reference oracle to remove.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn remove_reference_oracle(env: Env, contract_id: Address) {
        cross_reference::remove_reference_oracle(&env, contract_id);
    }

    /// Returns all registered reference oracle contract addresses.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// A [`Vec`] of `Address` values for all registered reference oracles.
    pub fn get_reference_oracles(env: Env) -> Vec<Address> {
        cross_reference::get_reference_oracles(&env)
    }

    /// Compares our current aggregated price for `asset` against the first registered
    /// reference oracle that has a mapping for this asset.
    ///
    /// If the deviation exceeds the configured threshold a
    /// [`CrossRefDeviationEvent`](crate::events::CrossRefDeviationEvent) is emitted.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset to check.
    ///
    /// # Returns
    ///
    /// `Some(`[`CrossReferenceResult`]`)` with both prices and the deviation in basis
    /// points, or `None` if no local aggregate exists or no reference oracle has a
    /// mapping for this asset.
    pub fn get_cross_reference(env: Env, asset: Address) -> Option<CrossReferenceResult> {
        cross_reference::get_cross_reference(&env, asset)
    }

    /// Sets the deviation threshold (in basis points) that triggers a
    /// [`CrossRefDeviationEvent`](crate::events::CrossRefDeviationEvent).
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `threshold_bps` - New threshold in basis points (100 bps = 1 %; max `100_000`).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `threshold_bps > 100_000`.
    pub fn set_cross_ref_deviation_threshold(env: Env, threshold_bps: u32) {
        cross_reference::set_cross_ref_deviation_threshold(&env, threshold_bps);
    }

    /// Returns the current cross-reference deviation threshold in basis points.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Threshold in basis points. Defaults to `500` (5 %).
    pub fn get_cross_ref_deviation_threshold(env: Env) -> u32 {
        cross_reference::get_cross_ref_deviation_threshold(&env)
    }

    // --- Cross-Reference Oracle ---

    /// Registers an external oracle as a cross-reference source for price verification.
    ///
    /// `asset_mapping` maps each of our asset `Address` values to the equivalent asset
    /// `Address` accepted by the external oracle's `lastprice` function. Calling
    /// [`get_cross_reference`](Self::get_cross_reference) will invoke this oracle when
    /// a mapping exists for the queried asset.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn add_reference_oracle(env: Env, contract_id: Address, asset_mapping: Map<Address, Address>) {
        cross_reference::add_reference_oracle(&env, contract_id, asset_mapping);
    }

    /// Removes a previously registered reference oracle.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn remove_reference_oracle(env: Env, contract_id: Address) {
        cross_reference::remove_reference_oracle(&env, contract_id);
    }

    /// Returns the ordered list of registered reference oracle contract addresses.
    pub fn get_reference_oracles(env: Env) -> Vec<Address> {
        cross_reference::get_reference_oracles(&env)
    }

    /// Queries our aggregated price for `asset` against the first registered reference
    /// oracle that has a mapping for it.
    ///
    /// Calls `lastprice(mapped_asset)` on the reference oracle via cross-contract
    /// invocation. Returns `None` when no local aggregate exists, no oracle has a
    /// mapping for `asset`, or every oracle returned `0`.
    ///
    /// Emits [`CrossRefDeviationEvent`](crate::events::CrossRefDeviationEvent) when
    /// the computed deviation exceeds the configured threshold.
    pub fn get_cross_reference(env: Env, asset: Address) -> Option<CrossReferenceResult> {
        cross_reference::get_cross_reference(&env, asset)
    }

    /// Sets the maximum acceptable price deviation between our oracle and a reference
    /// oracle, expressed in basis points (100 bps = 1 %).
    ///
    /// When the deviation for a given asset exceeds this threshold a
    /// [`CrossRefDeviationEvent`](crate::events::CrossRefDeviationEvent) is emitted.
    /// Defaults to `500` (5 %). Values above `100_000` are rejected.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `threshold_bps > 100_000`.
    pub fn set_cross_ref_deviation_threshold(env: Env, threshold_bps: u32) {
        cross_reference::set_cross_ref_deviation_threshold(&env, threshold_bps);
    }

    /// Returns the current cross-reference deviation threshold in basis points.
    ///
    /// Defaults to `500` (5 %) when no value has been configured.
    pub fn get_cross_ref_deviation_threshold(env: Env) -> u32 {
        cross_reference::get_cross_ref_deviation_threshold(&env)
    }
}

#[cfg(test)]
mod test_helpers;

mod test;

#[cfg(test)]
mod relayer_tests;
