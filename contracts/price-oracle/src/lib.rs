#![no_std]
// The reputation/staking (#171), correlation (#172), tiered whitelisting (#173), and
// alert-subscription (#174) modules compile and are fully implemented, but a previous
// botched merge dropped their `#[contractimpl]` wiring in this file, so none of their
// public functions are reachable from the deployed contract yet. Silencing dead_code
// here until that wiring lands, rather than deleting working, tested implementations.
#![allow(dead_code)]

mod admin;
mod admin_op_limits;
mod alerts;
mod amm;
mod asset_inactivity;
mod assets;
// The core module is always compiled (it has no Env deps).
// When the `fuzz` feature is enabled it is also re-exported so that the
// fuzz crate can call `price_oracle::core::*` directly.
mod audit_log;
mod config_history;
#[cfg_attr(feature = "fuzz", allow(dead_code))]
pub(crate) mod core;
mod cross_reference;
mod deadline_rebate;
mod dex;
mod errors;
mod event_indexing;
mod events;
mod exotic_pricing;
mod export_history;
mod fee_market;
mod finality;
mod freeze;
mod calibration;
mod eth_bridge;
mod gas_metering;
mod health;
mod history;
mod ibc_oracle;
mod migration;
mod multisig;
mod notifications;
mod optimistic;
mod pause;
mod per_asset_decimals;
mod prices;
mod rate_limiting;
mod rbac;
mod recovery;
mod reentrancy;
mod relayer;
mod relayer_bonds;
mod relayer_dashboard;
mod reputation;
mod rotation;
mod signed_submission;
mod simulate_batch;
mod source_deviation;
mod sources;
mod state_channel;
mod state_introspection;
mod storage;
mod submission_deadline;
mod subscription;
mod timelock;
mod triggers;
mod ttl_batching;
mod types;
mod vdf_sampler;
mod whitelisting;
mod zk_verify;
mod audit_log;
mod rbac;
mod emergency_pause;
mod freeze;
mod notifications;
mod config_history;
mod batch_storage;
mod price_proof;
mod price_callback;
mod contribution_quality;

// =============================================================================
// #283 — Stellar DID Integration
// =============================================================================
mod did;

// =============================================================================
// #282 — Bridge Oracle for Non-Stellar Assets
// =============================================================================
mod bridge_oracle;

// =============================================================================
// #285 — Ecosystem Metadata Registration
// =============================================================================
mod ecosystem_metadata;

// =============================================================================
// #284 — Event Streaming to External Databases
// =============================================================================
mod event_streaming;

#[cfg(test)]
mod circuit_breaker_tests;

#[cfg(test)]
mod cross_ref_tests;

#[cfg(test)]
mod override_tests;

#[cfg(test)]
mod prop_tests;

#[cfg(test)]
mod twap_tests;

#[cfg(test)]
mod optimistic_oracle_tests;

#[cfg(test)]
mod string_boundary_tests;

#[cfg(test)]
mod challenger_tests;

#[cfg(test)]
mod audit_log_tests;

#[cfg(test)]
mod rbac_tests;

#[cfg(test)]
mod emergency_pause_tests;

#[cfg(test)]
mod config_history_tests;

#[cfg(test)]
mod state_introspection_tests;

#[cfg(test)]
mod dex_tests;

#[cfg(test)]
mod amm_integration_tests;

#[cfg(test)]
mod l2_sequencer_oracle_tests;

#[cfg(test)]
mod oracle_sync_tests;

#[cfg(test)]
mod early_submission_discount_tests;

#[cfg(test)]
mod upgrade_simulation_tests;

pub use types::{
    AggregatePrice,
    AggregationMethod,
    Asset,
    BatchOperation,
    BatchSimulationResult,
    ConfigSnapshot,
    CrossReferenceResult,
    DataKey,
    DecentralizationReport,
    DemeritConfig,
    DisqualificationStatus,
    ErrorCode,
    // History export
    ExportedEntry,
    ExportedHistorySnapshot,
    FinalityStatus,
    FinalizedPrice,
    FrozenPrice,
    GasRecord,
    HealthReport,
    MigrationState,
    NotificationPreference,
    // Timelock priority
    OperationPriority,
    OperationSimulationResult,
    OracleSources,
    PendingBatch,
    PendingFinalityEntry,
    PriceCommit,
    PriceData,
    PriceEntry,
    PriceHistoryEntry,
    PriceOverrideEntry,
    RelayerInfo,
    // Batch dry-run simulation
    SimulationWarning, OperationSimulationResult, BatchSimulationResult,
    // State introspection
    StateDump, StateAnalysis, StateDiff, StateDiffEntry,
    // DEX / AMM integration
    DexPrice, AmmWeightConfig, SoroswapPool,
    // Cosmos/IBC light-client price feeds
    TendermintValidator, IbcClientState, IbcConsensusState, IbcPricePacket, IbcPriceEntry,
    // Ethereum bridge price feeds
    EthBridgeConfig, EthPriceMessage, EthBridgedPrice,
    // Source accuracy calibration
    CalibrationConfig, CalibrationBenchmark, CalibrationScore,
};

use soroban_sdk::{
    contract, contractimpl, panic_with_error, Address, Bytes, BytesN, Env, Map, String, Symbol, Vec,
};

use crate::storage::{enter_reentrancy_guard, exit_reentrancy_guard, read_registered_assets};

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
        enter_reentrancy_guard(&env);
        let result = admin::get_admin_address(&env);
        exit_reentrancy_guard(&env);
        result
    }

    pub fn has_role(env: Env, caller: Address, role: crate::types::Role) -> bool {
        rbac::has_role(&env, &caller, role)
    }

    pub fn delegate_role(env: Env, delegatee: Address, role: crate::types::Role) {
        rbac::delegate_role(&env, delegatee, role);
    }

    pub fn revoke_role(env: Env, delegatee: Address, role: crate::types::Role) {
        rbac::revoke_role(&env, delegatee, role);
    }

    pub fn get_role_holders(env: Env, role: crate::types::Role) -> Vec<Address> {
        rbac::get_role_holders(&env, role)
    }

    pub fn get_roles_for_holder(env: Env, holder: Address) -> Vec<crate::types::Role> {
        rbac::get_roles_for_holder(&env, &holder)
    }

    pub fn emergency_pause(env: Env, reason: String, auto_unpause_ledgers: u32) {
        emergency_pause::emergency_pause(&env, reason, auto_unpause_ledgers);
    }

    pub fn extend_emergency_pause(env: Env, additional_ledgers: u32) {
        emergency_pause::extend_emergency_pause(&env, additional_ledgers);
    }

    pub fn cancel_emergency_pause(env: Env) {
        emergency_pause::cancel_emergency_pause(&env);
    }

    pub fn is_emergency_pause_active(env: Env) -> bool {
        emergency_pause::is_emergency_pause_active(&env)
    }

    pub fn get_emergency_pause(env: Env) -> Option<crate::types::EmergencyPause> {
        emergency_pause::get_emergency_pause(&env)
    }

    pub fn set_correlation_pair(
        env: Env,
        base_asset: Address,
        quote_asset: Address,
        min_ratio: u128,
        max_ratio: u128,
        enabled: bool,
    ) {
        correlation::set_correlation_pair(
            &env,
            base_asset,
            quote_asset,
            min_ratio,
            max_ratio,
            enabled,
        );
    }

    pub fn is_correlation_flagged(env: Env, source: Address, asset: Address) -> bool {
        correlation::is_correlation_flagged(&env, &source, &asset)
    }

    pub fn clear_correlation_flag(env: Env, source: Address, asset: Address) {
        correlation::clear_correlation_flag(&env, source, asset);
    }

    pub fn challenge_price(env: Env, asset: Address, expected_price: i128, proof_data: Bytes) {
        challenger::challenge_price(&env, asset, expected_price, proof_data);
    }

    pub fn resolve_challenge(env: Env, challenge_id: u32, is_valid: bool) {
        challenger::resolve_challenge(&env, challenge_id, is_valid);
    }

    pub fn get_challenge_history(
        env: Env,
        asset: Address,
        limit: u32,
    ) -> Vec<crate::types::Challenge> {
        challenger::get_challenge_history(&env, asset, limit)
    }

    pub fn get_challenger_rewards(env: Env, challenger: Address) -> i128 {
        challenger::get_challenger_rewards(&env, challenger)
    }

    pub fn get_audit_log_count(env: Env) -> u32 {
        audit_log::get_audit_log_count(&env)
    }

    pub fn get_admin_audit_log(
        env: Env,
        from_id: u32,
        limit: u32,
    ) -> Vec<crate::types::AuditEntry> {
        audit_log::get_admin_audit_log(&env, from_id, limit)
    }

    pub fn get_audit_log_head(env: Env) -> Bytes {
        audit_log::get_audit_log_head(&env)
    }

    pub fn verify_audit_chain(env: Env) -> bool {
        audit_log::verify_audit_chain(&env)
    }

    pub fn simulate_aggregation(
        env: Env,
        asset: Address,
        hypothetical_prices: Vec<(Address, i128)>,
    ) -> Option<i128> {
        prices::simulate_aggregation(&env, asset, hypothetical_prices)
    }

    pub fn submit_price_merkle(
        env: Env,
        source: Address,
        root: BytesN<32>,
        proofs: Vec<prices::MerkleProof>,
    ) {
        prices::submit_price_merkle(&env, source, root, proofs);
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
        enter_reentrancy_guard(&env);
        let result = admin::get_min_sources_required(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_max_history_length(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_resolution(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_decimals(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_description(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_timestamp_threshold(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_max_price_deviation(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_heartbeat_interval(&env);
        exit_reentrancy_guard(&env);
        result
    }

    /// Sets the query rate limit — the maximum number of queries allowed per ledger.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `max_per_ledger` - Maximum queries per ledger (must be > 0).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_query_rate_limit(env: Env, max_per_ledger: u32) {
        admin::set_query_rate_limit(&env, max_per_ledger);
    }

    /// Returns the current query rate limit.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Query rate limit per ledger. Defaults to `100`.
    pub fn get_query_rate_limit(env: Env) -> u32 {
        admin::get_query_rate_limit(&env)
    }

    // --- Subscription ---

    /// Creates a new subscription for the consumer with the given duration plan.
    ///
    /// The `consumer` address must authorize this call. The `duration` must match
    /// a registered plan. The expiry timestamp is set to `ledger_timestamp + duration`
    /// in seconds.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `consumer` - Address of the consumer purchasing the subscription.
    /// * `duration` - Duration in seconds. Must match an existing plan.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if `consumer` does not authorize the call.
    /// * [`ErrorCode::InvalidDuration`] — if `duration` does not match any registered plan.
    pub fn subscribe(env: Env, consumer: Address, duration: u32) {
        subscription::subscribe(&env, consumer, duration);
    }

    /// Renews an existing active subscription by extending its expiry with the remaining duration.
    ///
    /// The `consumer` address must authorize this call. The current subscription
    /// must not have expired. Expiry is extended by the remaining time on the subscription.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `consumer` - Address of the consumer renewing their subscription.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if `consumer` does not authorize the call.
    /// * [`ErrorCode::NoData`] — if no subscription exists for `consumer`.
    /// * [`ErrorCode::SubscriptionExpired`] — if the current subscription has expired.
    pub fn renew_subscription(env: Env, consumer: Address) {
        subscription::renew_subscription(&env, consumer);
    }

    /// Returns the expiry timestamp for a consumer's subscription, or `0` if none exists.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `consumer` - Address of the consumer to query.
    ///
    /// # Returns
    ///
    /// `expiry_timestamp` if a subscription exists; `0` otherwise.
    pub fn get_subscription_expiry(env: Env, consumer: Address) -> u64 {
        subscription::get_subscription_expiry(&env, consumer)
    }

    /// Returns all available subscription plans.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// A [`SubscriptionPlans`] map of duration (seconds) to amount (stroops).
    pub fn get_subscription_plans(env: Env) -> SubscriptionPlans {
        subscription::get_subscription_plans(&env)
    }

    /// Sets the price for a subscription plan.
    ///
    /// The admin must authorize this call. If a plan with the same duration already
    /// exists, it is updated.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `duration` - Duration in seconds identifying the plan.
    /// * `amount` - Cost amount in stroops.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_subscription_price(env: Env, duration: u32, amount: i128) {
        admin::set_subscription_price(&env, duration, amount);
    }

    // --- #306: SAC Token Integration for Subscriptions ---

    /// Sets the SAC token contract used for subscription payments.  Admin-only.
    ///
    /// When configured, calls to [`subscribe`](Self::subscribe) will transfer
    /// `plan_amount` tokens from the consumer to this contract.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_subscription_token(env: Env, token_contract: Address) {
        subscription::set_subscription_token(&env, token_contract);
    }

    /// Returns the currently configured SAC token contract address, or `None`.
    pub fn get_subscription_token(env: Env) -> Option<Address> {
        subscription::get_subscription_token(&env)
    }

    /// Cancels `consumer`'s subscription and returns a pro-rated token refund
    /// for the unused portion (when a SAC token is configured).
    ///
    /// `consumer` must authorize this call.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`]       — `consumer` did not authorize the call.
    /// * [`ErrorCode::NoActiveSubscription`] — no active subscription found.
    pub fn cancel_subscription(env: Env, consumer: Address) {
        subscription::cancel_subscription(&env, consumer);
    }

    // --- #304: Consumer Contract Authorization ---

    /// Grants explicit access to `consumer`.  Admin-only.
    ///
    /// In `AllowedOnly` mode this consumer may query price data.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn add_authorized_consumer(env: Env, consumer: Address) {
        consumer_auth::add_authorized_consumer(&env, consumer);
    }

    /// Revokes explicit access for `consumer`.  Admin-only.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn remove_authorized_consumer(env: Env, consumer: Address) {
        consumer_auth::remove_authorized_consumer(&env, consumer);
    }

    /// Sets the global consumer access mode.  Admin-only.
    ///
    /// | `mode` | Behaviour |
    /// |--------|-----------|
    /// | `0`    | `Public` — no restrictions (default) |
    /// | `1`    | `AllowedOnly` — only allowlisted consumers |
    /// | `2`    | `BlockedOnly` — all except blocklisted consumers |
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`]     — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `mode` is not `0`, `1`, or `2`.
    pub fn set_consumer_access_mode(env: Env, mode: u32) {
        consumer_auth::set_consumer_access_mode(&env, mode);
    }

    /// Returns the current consumer access mode as a [`ConsumerAccessMode`] variant.
    pub fn get_consumer_access_mode(env: Env) -> ConsumerAccessMode {
        consumer_auth::get_consumer_access_mode(&env)
    }

    /// Returns whether `consumer` is currently authorized to query prices.
    pub fn is_consumer_authorized(env: Env, consumer: Address) -> bool {
        consumer_auth::is_consumer_authorized(&env, &consumer)
    }

    // --- #303: On-chain Price Deviation Report ---

    /// Returns deviation statistics for a source's last `num_rounds` price submissions.
    ///
    /// Each submission's deviation from the aggregate at submission time is recorded
    /// automatically by [`submit_price`](Self::submit_price).
    ///
    /// # Returns
    ///
    /// A [`DeviationReport`] with `avg_deviation_bps`, `max_deviation_bps`,
    /// `outlier_count`, `trend` (linear regression slope), and `num_rounds`.
    pub fn get_source_deviation_report(
        env: Env,
        source: Address,
        asset: Address,
        num_rounds: u32,
    ) -> DeviationReport {
        source_deviation::get_source_deviation_report(&env, source, asset, num_rounds)
    }

    // --- #305: Price Update Subscription Registry (pull-based) ---

    /// Registers `consumer` as interested in price updates for `asset`.
    ///
    /// Off-chain relayers call [`get_subscribed_consumers`](Self::get_subscribed_consumers)
    /// to discover registered consumers and dispatch pull-requests on their behalf.
    ///
    /// `consumer` must authorize this call.  Idempotent if already subscribed.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    pub fn subscribe_price_updates(env: Env, consumer: Address, asset: Address) {
        price_update_subscription::subscribe_price_updates(&env, consumer, asset);
    }

    /// Removes `consumer`'s price-update subscription for `asset`.
    ///
    /// `consumer` must authorize this call.  Idempotent if not subscribed.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    pub fn unsubscribe_price_updates(env: Env, consumer: Address, asset: Address) {
        price_update_subscription::unsubscribe_price_updates(&env, consumer, asset);
    }

    /// Returns the list of all consumers currently subscribed to `asset`.
    ///
    /// Off-chain relayers read this list and dispatch individual calls to each
    /// subscriber after a price update.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    pub fn get_subscribed_consumers(env: Env, asset: Address) -> Vec<Address> {
        price_update_subscription::get_subscribed_consumers(&env, asset)
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

    // --- #191: Aggregation method selection ---

    /// Sets the active price aggregation method. Admin-only.
    ///
    /// | `method` | Algorithm |
    /// |----------|-----------|
    /// | `0` | **Median** (default) — O(n) quickselect, resistant to outliers |
    /// | `1` | **Mean** — arithmetic average of all prices |
    /// | `2` | **TrimmedMean** — mean after removing top/bottom 10% |
    /// | `3` | **WeightedMedian** — median weighted by source reputation scores |
    ///
    /// Emits `AggregationMethodChangedEvent`.
    pub fn set_aggregation_method(env: Env, method: u32) {
        reentrancy::enter(&env);
        admin::set_aggregation_method(&env, method);
        reentrancy::exit(&env);
    }

    /// Returns the current aggregation method discriminant.
    /// * `0` = Median, `1` = Mean, `2` = TrimmedMean, `3` = WeightedMedian
    pub fn get_aggregation_method(env: Env) -> u32 {
        admin::get_aggregation_method(&env)
    }

    /// Returns the newest retained core-configuration snapshots.
    ///
    /// Ordering is newest-first. `count == 0` returns an empty vector. At most
    /// 100 retained snapshots are ever returned.
    pub fn get_config_history(env: Env, count: u32) -> Vec<ConfigSnapshot> {
        config_history::get_config_history(&env, count)
    }

    /// Restores a previously captured core-configuration snapshot.
    ///
    /// Snapshots the current live config first (append-only), then applies the
    /// selected version. Admin-only.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::ConfigVersionNotFound`] — if `version` is missing or pruned.
    pub fn rollback_config(env: Env, version: u32) {
        reentrancy::enter(&env);
        config_history::rollback_config(&env, version);
        reentrancy::exit(&env);
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

    /// Dry-runs `operations` and returns a [`BatchSimulationResult`] describing what
    /// *would* happen if the batch were executed — **without committing any state changes**.
    ///
    /// Use this before calling [`propose_batch`] to catch misconfigured operations early.
    ///
    /// # Returns
    ///
    /// A [`BatchSimulationResult`] with per-operation results, warning counts, and an
    /// `all_succeed` flag indicating whether the full batch is safe to submit.
    pub fn simulate_batch(env: Env, operations: Vec<BatchOperation>) -> BatchSimulationResult {
        simulate_batch::simulate_batch(&env, operations)
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

    /// Sets the maximum number of oracle sources that may be registered. `0` = unlimited.
    pub fn set_max_sources(env: Env, new_max: u32) {
        admin::set_max_sources(&env, new_max);
    }

    /// Returns the current maximum registered-source cap. Defaults to `0` (unlimited).
    pub fn get_max_sources(env: Env) -> u32 {
        admin::get_max_sources(&env)
    }

    /// Sets the maximum number of price history entries retained per asset (issue #94).
    pub fn set_max_history_per_asset(env: Env, new_max: u32) {
        admin::set_max_history_per_asset(&env, new_max);
    }

    /// Returns the current per-asset history cap. Defaults to `1000`.
    pub fn get_max_history_per_asset(env: Env) -> u32 {
        admin::get_max_history_per_asset(&env)
    }

    /// Sets the maximum number of events emitted per aggregation call (issue #92).
    pub fn set_max_events_per_call(env: Env, new_max: u32) {
        admin::set_max_events_per_call(&env, new_max);
    }

    /// Returns the current per-call event cap. Defaults to `20`.
    pub fn get_max_events_per_call(env: Env) -> u32 {
        admin::get_max_events_per_call(&env)
    }

    /// Sets the maximum number of sources used per aggregation; `0` = no limit (issue #93).
    pub fn set_max_aggregation_sources(env: Env, new_max: u32) {
        admin::set_max_aggregation_sources(&env, new_max);
    }

    /// Returns the current maximum aggregation-sources limit. Defaults to `0` (no limit).
    pub fn get_max_aggregation_sources(env: Env) -> u32 {
        admin::get_max_aggregation_sources(&env)
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
        enter_reentrancy_guard(&env);
        let result = sources::is_source(&env, source);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = sources::get_oracle_sources(&env);
        exit_reentrancy_guard(&env);
        result
    }

    pub fn add_source_with_assets(env: Env, source: Address, name: String, assets: Vec<Address>) {
        reentrancy::enter(&env);
        sources::add_source_with_assets(&env, source, name, assets);
        reentrancy::exit(&env);
    }

    pub fn get_source_assets(env: Env, source: Address) -> Vec<Address> {
        sources::get_source_assets(&env, source)
    }

    pub fn add_source_asset(env: Env, source: Address, asset: Address) {
        reentrancy::enter(&env);
        sources::add_source_asset(&env, source, asset);
        reentrancy::exit(&env);
    }

    pub fn remove_source_asset(env: Env, source: Address, asset: Address) {
        reentrancy::enter(&env);
        sources::remove_source_asset(&env, source, asset);
        reentrancy::exit(&env);
    }

    pub fn set_source_verification(
        env: Env,
        source: Address,
        verified: bool,
        verification_method: String,
        verifier: Address,
    ) {
        reentrancy::enter(&env);
        sources::set_source_verification(&env, source, verified, verification_method, verifier);
        reentrancy::exit(&env);
    }

    pub fn get_source_verification(env: Env, source: Address) -> Option<SourceVerification> {
        sources::get_source_verification(&env, source)
    }

    pub fn rotate_source_key(env: Env, source: Address, new_address: Address) {
        reentrancy::enter(&env);
        sources::rotate_source_key(&env, source, new_address);
        reentrancy::exit(&env);
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
        enter_reentrancy_guard(&env);
        let result = sources::is_source_inactive(&env, source);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = sources::get_inactive_sources(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = sources::get_source_last_heartbeat(&env, source);
        exit_reentrancy_guard(&env);
        result
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

    // --- #210: Progressive Disqualification ---

    pub fn set_demerit_config(env: Env, config: DemeritConfig) {
        reentrancy::enter(&env);
        sources::set_demerit_config(&env, config);
        reentrancy::exit(&env);
    }

    pub fn get_demerit_config(env: Env) -> DemeritConfig {
        sources::get_demerit_config(&env)
    }

    pub fn get_source_demerits(env: Env, source: Address) -> SourceDemeritState {
        sources::get_source_demerits(&env, source)
    }

    pub fn reset_source_demerits(env: Env, source: Address) {
        reentrancy::enter(&env);
        sources::reset_source_demerits(&env, source);
        reentrancy::exit(&env);
    }

    // --- #207: Multi-sig Source Governance ---

    pub fn set_source_governance(env: Env, approvers: Vec<Address>, threshold: u32) {
        reentrancy::enter(&env);
        sources::set_source_governance(&env, approvers, threshold);
        reentrancy::exit(&env);
    }

    pub fn get_source_governance(env: Env) -> Option<SourceGovernance> {
        sources::get_source_governance(&env)
    }

    pub fn propose_source(env: Env, proposer: Address, source: Address, name: String) -> u32 {
        reentrancy::enter(&env);
        let id = sources::propose_source(&env, proposer, source, name);
        reentrancy::exit(&env);
        id
    }

    pub fn approve_source(env: Env, approver: Address, proposal_id: u32) {
        reentrancy::enter(&env);
        sources::approve_source(&env, approver, proposal_id);
        reentrancy::exit(&env);
    }

    pub fn get_source_proposal(env: Env, proposal_id: u32) -> SourceProposal {
        sources::get_source_proposal(&env, proposal_id)
    }

    // --- #208: Source Geolocation & Decentralization Metrics ---

    pub fn set_source_geo(env: Env, source: Address, metadata: SourceGeoMetadata) {
        reentrancy::enter(&env);
        sources::set_source_geo(&env, source, metadata);
        reentrancy::exit(&env);
    }

    pub fn get_source_geo(env: Env, source: Address) -> Option<SourceGeoMetadata> {
        sources::get_source_geo(&env, source)
    }

    pub fn get_decentralization_report(env: Env) -> DecentralizationReport {
        sources::get_decentralization_report(&env)
    }

    // --- #209: Source Heartbeat Liveness Bond ---

    pub fn set_source_bond(env: Env, amount: i128) {
        reentrancy::enter(&env);
        sources::set_source_bond(&env, amount);
        reentrancy::exit(&env);
    }

    pub fn get_source_bond(env: Env) -> i128 {
        sources::get_source_bond(&env)
    }

    pub fn deposit_source_bond(env: Env, source: Address) {
        reentrancy::enter(&env);
        sources::deposit_source_bond(&env, source);
        reentrancy::exit(&env);
    }

    pub fn get_source_deposited_bond(env: Env, source: Address) -> i128 {
        sources::get_source_deposited_bond(&env, source)
    }

    pub fn set_stake_token_contract(env: Env, token: Address) {
        reentrancy::enter(&env);
        crate::reputation::set_stake_token_contract(&env, token);
        reentrancy::exit(&env);
    }

    pub fn get_stake_token_contract(env: Env) -> Option<Address> {
        crate::reputation::get_stake_token_contract(&env)
    }

    /// Sets per-source deviation tolerance in basis points (admin only).
    /// Set to 0 to clear and fall back to global tolerance.
    pub fn set_source_deviation_tolerance(env: Env, source: Address, tolerance_bps: u32) {
        source_deviation::set_source_deviation_tolerance(&env, source, tolerance_bps);
    }

    /// Returns per-source deviation tolerance in bps, or None if global is used.
    pub fn get_source_deviation_tolerance(env: Env, source: Address) -> Option<u32> {
        source_deviation::get_source_deviation_tolerance(&env, &source)
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
        enter_reentrancy_guard(&env);
        let result = assets::is_asset_registered(&env, asset);
        exit_reentrancy_guard(&env);
        result
    }

    // --- Asset Inactivity (#301) ---

    /// Sets the global default inactivity timeout in ledgers (admin only, 0 = disabled).
    pub fn set_inactivity_timeout(env: Env, timeout_ledgers: u32) {
        asset_inactivity::set_inactivity_timeout(&env, timeout_ledgers);
    }

    pub fn get_inactivity_timeout(env: Env) -> u32 {
        asset_inactivity::get_inactivity_timeout(&env)
    }

    /// Sets per-asset inactivity timeout override (admin only, 0 = use global).
    pub fn set_asset_inactivity_timeout(env: Env, asset: Address, timeout_ledgers: u32) {
        asset_inactivity::set_asset_inactivity_timeout(&env, asset, timeout_ledgers);
    }

    pub fn get_asset_inactivity_timeout(env: Env, asset: Address) -> u32 {
        asset_inactivity::get_asset_inactivity_timeout(&env, &asset)
    }

    /// Returns true if the asset is considered inactive (past its timeout).
    pub fn is_asset_inactive(env: Env, asset: Address) -> bool {
        asset_inactivity::is_asset_inactive(&env, &asset)
    }

    /// Admin: check an asset and deregister it if it exceeds inactivity threshold.
    pub fn check_and_deregister_if_inactive(env: Env, asset: Address) {
        asset_inactivity::check_and_deregister_if_inactive(&env, asset);
    }

    pub fn set_price_bounds(
        env: Env,
        asset: Address,
        min_price: i128,
        max_price: i128,
        max_change_bps_per_ledger: u32,
    ) {
        reentrancy::enter(&env);
        assets::set_price_bounds(&env, asset, min_price, max_price, max_change_bps_per_ledger);
        reentrancy::exit(&env);
    }

    pub fn get_price_bounds(env: Env, asset: Address) -> PriceBounds {
        assets::get_price_bounds(&env, asset)
    }

    pub fn pause_asset(env: Env, asset: Address) {
        reentrancy::enter(&env);
        assets::pause_asset(&env, asset);
        reentrancy::exit(&env);
    }

    pub fn unpause_asset(env: Env, asset: Address) {
        reentrancy::enter(&env);
        assets::unpause_asset(&env, asset);
        reentrancy::exit(&env);
    }

    pub fn is_asset_paused(env: Env, asset: Address) -> bool {
        assets::is_asset_paused(&env, &asset)
    }

    pub fn is_circuit_breaker_tripped(env: Env, asset: Address) -> bool {
        assets::is_circuit_breaker_tripped(&env, &asset)
    }

    // --- Optimistic Oracle ---

    pub fn propose_price(
        env: Env,
        asset: Address,
        price: i128,
        timestamp: u64,
        bond_amount: i128,
    ) -> u32 {
        reentrancy::enter(&env);
        let result = optimistic::propose_price(&env, asset, price, timestamp, bond_amount);
        reentrancy::exit(&env);
        result
    }

    pub fn dispute_proposal(env: Env, proposal_id: u32) {
        reentrancy::enter(&env);
        optimistic::dispute_proposal(&env, proposal_id);
        reentrancy::exit(&env);
    }

    pub fn resolve_dispute(env: Env, proposal_id: u32, resolution: bool) {
        reentrancy::enter(&env);
        optimistic::resolve_dispute(&env, proposal_id, resolution);
        reentrancy::exit(&env);
    }

    pub fn get_proposal(env: Env, proposal_id: u32) -> Option<OptimisticProposal> {
        optimistic::get_proposal(&env, proposal_id)
    }

    /// Sets the dispute window (in ledgers) applied to new optimistic proposals.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — `dispute_window_ledgers` is `0`.
    pub fn set_optimistic_dispute_window(env: Env, dispute_window_ledgers: u32) {
        admin::set_optimistic_dispute_window(&env, dispute_window_ledgers);
    }

    /// Returns the dispute window (in ledgers) applied to new optimistic proposals.
    pub fn get_optimistic_dispute_window(env: Env) -> u32 {
        admin::get_optimistic_dispute_window(&env)
    }

    /// Sets the minimum bond required to propose or dispute an optimistic price.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — `min_bond` is `<= 0`.
    pub fn set_optimistic_min_bond(env: Env, min_bond: i128) {
        admin::set_optimistic_min_bond(&env, min_bond);
    }

    /// Returns the minimum bond required to propose or dispute an optimistic price.
    pub fn get_optimistic_min_bond(env: Env) -> i128 {
        admin::get_optimistic_min_bond(&env)
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
    pub fn submit_price(env: Env, source: Address, asset: Address, price: i128, timestamp: u64, nonce: u64) {
        reentrancy::enter(&env);
        // Measure budget before and after to record last submit_price cost.
        let before_cpu = env.budget().cpu_instruction_count();
        let before_mem = env.budget().memory_bytes_count();
        prices::submit_price(&env, source, asset, price, timestamp, nonce);
        let after_cpu = env.budget().cpu_instruction_count();
        let after_mem = env.budget().memory_bytes_count();
        let cpu_delta = after_cpu.saturating_sub(before_cpu);
        let mem_delta = after_mem.saturating_sub(before_mem);
        crate::gas_metering::write_last_gas(
            &env,
            String::from_str(&env, "submit_price"),
            cpu_delta,
            mem_delta,
        );
        reentrancy::exit(&env);
    }

    pub fn submit_price_with_volume(
        env: Env,
        source: Address,
        asset: Address,
        price: i128,
        timestamp: u64,
        volume: Option<i128>,
    ) {
        reentrancy::enter(&env);
        prices::submit_price_with_volume(&env, source, asset, price, timestamp, volume);
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

    // --- Off-chain signature-verified price submission (#216) ---

    /// Registers (or rotates) the Ed25519 public key `source` uses to sign
    /// off-chain price proofs for [`submit_price_with_proof`]. Must be
    /// authorized by `source`.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::SourceNotFound`] — `source` is not a registered oracle source.
    pub fn register_submission_key(env: Env, source: Address, public_key: BytesN<32>) {
        signed_submission::register_submission_key(&env, source, public_key);
    }

    /// Submits a price on behalf of `source` using a pre-signed Ed25519 proof
    /// instead of `source`'s Soroban transaction authorization. Callable by
    /// anyone (typically a relayer bundling proofs from many sources).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::ContractPaused`] — the contract is currently paused.
    /// * [`ErrorCode::SourceNotFound`] — `source` is not a registered oracle source.
    /// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
    /// * [`ErrorCode::SigningKeyNotRegistered`] — `source` has no registered submission key.
    /// * [`ErrorCode::SignatureExpired`] — `expiration_ledger` has already passed.
    /// * [`ErrorCode::InvalidNonce`] — `nonce` does not exceed the source's last accepted nonce.
    /// * [`ErrorCode::NotAuthorized`] — the Ed25519 signature is invalid, or `source` is suspended.
    /// * [`ErrorCode::InvalidPrice`] / [`ErrorCode::PriceBelowMinimum`] / [`ErrorCode::InvalidTimestamp`]
    pub fn submit_price_with_proof(
        env: Env,
        source: Address,
        asset: Address,
        price: i128,
        timestamp: u64,
        nonce: u64,
        expiration_ledger: u32,
        signature: BytesN<64>,
    ) {
        signed_submission::submit_price_with_proof(
            &env,
            source,
            asset,
            price,
            timestamp,
            nonce,
            expiration_ledger,
            signature,
        );
    }

    // --- Configurable aggregation triggers (#218) ---

    /// Sets the minimum number of seconds between time-triggered
    /// aggregations for `asset`. `0` disables the time-based trigger.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    /// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
    pub fn set_time_trigger(env: Env, asset: Address, interval_seconds: u64) {
        triggers::set_time_trigger(&env, asset, interval_seconds);
    }

    /// Returns the configured time-trigger interval (seconds) for `asset`. `0` = disabled.
    pub fn get_time_trigger(env: Env, asset: Address) -> u64 {
        triggers::get_time_trigger(&env, asset)
    }

    /// Sets the number of new submissions that auto-trigger aggregation for
    /// `asset`. `0` disables the threshold-based trigger.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    /// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
    pub fn set_submission_threshold_trigger(env: Env, asset: Address, threshold: u32) {
        triggers::set_submission_threshold_trigger(&env, asset, threshold);
    }

    /// Returns the configured submission-count trigger threshold for `asset`. `0` = disabled.
    pub fn get_submission_threshold_trigger(env: Env, asset: Address) -> u32 {
        triggers::get_submission_threshold_trigger(&env, asset)
    }

    /// Sets the price deviation (in basis points) that auto-triggers
    /// aggregation for `asset`. `0` disables the deviation-based trigger.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    /// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
    /// * [`ErrorCode::InvalidConfiguration`] — `deviation_bps` exceeds `100_000`.
    pub fn set_deviation_trigger(env: Env, asset: Address, deviation_bps: u32) {
        triggers::set_deviation_trigger(&env, asset, deviation_bps);
    }

    /// Returns the configured deviation trigger threshold (bps) for `asset`. `0` = disabled.
    pub fn get_deviation_trigger(env: Env, asset: Address) -> u32 {
        triggers::get_deviation_trigger(&env, asset)
    }

    /// Permissionless keeper endpoint: re-aggregates `asset` if at least the
    /// configured time-trigger interval has elapsed since the last
    /// trigger-driven aggregation. Returns `true` if aggregation ran.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
    pub fn poke_time_trigger(env: Env, asset: Address) -> bool {
        triggers::poke_time_trigger(&env, asset)
    }

    /// Returns current budget counters and the last recorded gas usage.
    ///
    /// Returns `(cpu_instructions_used, memory_bytes_used, last_recorded)` where
    /// `last_recorded` is the `GasRecord` for the most-recent submit/aggregate.
    pub fn get_gas_stats(env: Env) -> (u64, u64, Option<GasRecord>) {
        let cpu = env.budget().cpu_instruction_count();
        let mem = env.budget().memory_bytes_count();
        let last = crate::gas_metering::read_last_gas(&env);
        (cpu, mem, last)
    }

    /// Returns storage TTL status for well-known keys. Remaining TTL is `0`
    /// when the runtime does not expose a retrievable TTL value.
    pub fn get_storage_ttl_status(env: Env) -> Vec<StorageTtlEntry> {
        crate::storage::get_storage_ttl_status(&env)
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
    /// * [`ErrorCode::RateLimitExceeded`] — if the caller has exceeded the query rate limit.
    pub fn get_price(env: Env, asset: Address, max_age: u64) -> Option<AggregatePrice> {
        enter_reentrancy_guard(&env);
        let result = prices::get_price(&env, asset, max_age);
        exit_reentrancy_guard(&env);
        result
    }

    /// Consumer-authorized variant of [`get_price`](Self::get_price).
    ///
    /// `consumer` must authorize this call and must be permitted under the current
    /// [`ConsumerAccessMode`].  All other behaviour is identical to `get_price`.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if `consumer` is not allowed under the
    ///   current access mode.
    pub fn get_price_authorized(
        env: Env,
        consumer: Address,
        asset: Address,
        max_age: u64,
    ) -> Option<AggregatePrice> {
        consumer.require_auth();
        consumer_auth::check_consumer_authorized(&env, &consumer);
        enter_reentrancy_guard(&env);
        let result = prices::get_price(&env, asset, max_age);
        exit_reentrancy_guard(&env);
        result
    }

    pub fn get_price_with_confidence(env: Env, asset: Address) -> Option<(AggregatePrice, u32)> {
        prices::get_price_with_confidence(&env, asset)
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
        enter_reentrancy_guard(&env);
        let result = prices::get_source_price(&env, asset, source);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = prices::get_all_prices(&env, asset);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = env.ledger().sequence();
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = history::get_historical_price(&env, asset, ledger);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = history::has_historical_price(&env, asset, ledger);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = history::get_historical_prices(&env, asset, start_ledger, end_ledger);
        exit_reentrancy_guard(&env);
        result
    }

    /// Returns a cursor-paginated page of historical price entries for an asset (#229).
    ///
    /// `cursor` is the ledger sequence number to start from (inclusive); pass `0` for the
    /// first page. `limit` is capped at `history::MAX_PAGE_SIZE` (50).
    ///
    /// # Returns
    ///
    /// `(entries, next_cursor)` — `next_cursor` is `Some(ledger)` to request the next page,
    /// or `None` once all recorded entries have been returned.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    /// * [`ErrorCode::InvalidPageSize`] — if `limit` is `0` or exceeds the maximum page size.
    pub fn get_historical_prices_paginated(
        env: Env,
        asset: Address,
        cursor: u32,
        limit: u32,
    ) -> (Vec<PriceHistoryEntry>, Option<u32>) {
        enter_reentrancy_guard(&env);
        let result = history::get_historical_prices_paginated(&env, asset, cursor, limit);
        exit_reentrancy_guard(&env);
        result
    }

    // --- History export ---

    /// Exports up to `limit` price-history entries for `asset`, starting at `from_ledger`.
    ///
    /// Returns an [`ExportedHistorySnapshot`] containing the entries, a lightweight
    /// `data_hash` for integrity verification, and a `next_cursor` for pagination.
    ///
    /// # Arguments
    ///
    /// * `asset`       — Registered asset address.
    /// * `from_ledger` — Inclusive start ledger (pass `0` to start from the beginning).
    /// * `limit`       — Maximum entries to return (1–200).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`]  — if `asset` is not registered.
    /// * [`ErrorCode::ExportLimitExceeded`] — if `limit` is `0` or `> 200`.
    pub fn export_history(
        env: Env,
        asset: Address,
        from_ledger: u32,
        limit: u32,
    ) -> ExportedHistorySnapshot {
        enter_reentrancy_guard(&env);
        let result = export_history::export_history(&env, asset, from_ledger, limit);
        exit_reentrancy_guard(&env);
        result
    }

    /// Verifies that `expected_data_hash` matches the XOR-fold hash of all history
    /// entries for `asset` stored within `[from_ledger, to_ledger]`.
    ///
    /// Returns `true` when the hash matches (snapshot is consistent with on-chain state),
    /// `false` otherwise.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    /// * [`ErrorCode::ExportNotFound`]     — if no entries exist in the given range.
    pub fn verify_export(
        env: Env,
        asset: Address,
        from_ledger: u32,
        to_ledger: u32,
        expected_data_hash: u64,
    ) -> bool {
        enter_reentrancy_guard(&env);
        let result =
            export_history::verify_export(&env, asset, from_ledger, to_ledger, expected_data_hash);
        exit_reentrancy_guard(&env);
        result
    }

    // --- Price freeze (#223) ---

    /// Freezes the current aggregate price for an asset during a market emergency.
    ///
    /// While frozen, `get_price` returns the frozen snapshot and price submissions for
    /// the asset are rejected until `unfreeze_price` is called.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
    /// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
    /// * [`ErrorCode::ReasonTooLong`] — if `reason` exceeds 256 characters.
    /// * [`ErrorCode::PriceFrozen`] — if the asset is already frozen.
    /// * [`ErrorCode::NoData`] — if the asset has no aggregate price yet.
    pub fn freeze_price(env: Env, asset: Address, reason: String) {
        freeze::freeze_price(&env, asset, reason);
    }

    /// Unfreezes a previously frozen asset, resuming normal price updates.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
    /// * [`ErrorCode::PriceNotFrozen`] — if the asset is not currently frozen.
    pub fn unfreeze_price(env: Env, asset: Address) {
        freeze::unfreeze_price(&env, asset);
    }

    /// Returns whether an asset's price is currently frozen.
    pub fn is_price_frozen(env: Env, asset: Address) -> bool {
        freeze::is_price_frozen(&env, asset)
    }

    // --- Notification preferences (#243) ---

    /// Registers an admin notification preference for a given event type.
    ///
    /// `event_type` is a caller-defined discriminant (e.g. matching an event-indexing
    /// scheme); `channel` identifies the notification kind (e.g. `"webhook"`, `"email"`)
    /// and `target` is the channel-specific destination. Dispatch is performed by an
    /// off-chain relayer service watching contract events — this only stores the
    /// preference.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
    /// * [`ErrorCode::NotificationConfigInvalid`] — if `channel` or `target` exceeds 256 chars.
    pub fn set_notification_preference(env: Env, event_type: u32, channel: String, target: String) {
        notifications::set_notification_preference(&env, event_type, channel, target);
    }

    /// Returns all notification preferences registered for a given event type.
    pub fn list_notification_preferences(env: Env, event_type: u32) -> Vec<NotificationPreference> {
        notifications::list_notification_preferences(&env, event_type)
    }

    /// Returns every event-type discriminant that currently has at least one
    /// notification preference registered.
    pub fn list_notification_event_types(env: Env) -> Vec<u32> {
        notifications::list_notification_event_types(&env)
    }

    /// Clears all notification preferences registered for a given event type.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
    pub fn clear_notification_preferences(env: Env, event_type: u32) {
        notifications::clear_notification_preferences(&env, event_type);
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

    // ─────────────────────────────────────────────────────────────────────────
    // #252 — Versioned Aggregate Price
    // ─────────────────────────────────────────────────────────────────────────

    /// Returns the current aggregated price together with its version counter (#252).
    ///
    /// The `version` is a monotonically-incrementing `u32` that starts at `0` after
    /// the first aggregation and increments by `1` each time the aggregate price
    /// changes. Consumers can poll `version` instead of comparing full `i128` values
    /// to detect price changes efficiently.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Asset address to query.
    ///
    /// # Returns
    ///
    /// [`VersionedAggregatePrice`] containing the full aggregate and the version.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    /// * [`ErrorCode::NoData`] — no aggregate price exists yet for the asset.
    pub fn get_aggregate_with_version(env: Env, asset: Address) -> VersionedAggregatePrice {
        enter_reentrancy_guard(&env);
        let result = prices::get_aggregate_with_version(&env, asset);
        exit_reentrancy_guard(&env);
        result
    }

    // ─────────────────────────────────────────────────────────────────────────
    // #247 — History Compaction
    // ─────────────────────────────────────────────────────────────────────────

    /// Sets the history compaction threshold in basis points (admin only) (#247).
    ///
    /// Adjacent history entries whose price difference is within
    /// `threshold_bps / 100 %` of each other are eligible for merging during
    /// `compact_history` or on the on-write path. A value of `0` disables
    /// compaction entirely (default).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
    pub fn set_compaction_threshold_bps(env: Env, threshold_bps: u32) {
        enter_reentrancy_guard(&env);
        admin::set_compaction_threshold_bps(&env, threshold_bps);
        exit_reentrancy_guard(&env);
    }

    /// Returns the current history compaction threshold in basis points (0 = disabled).
    pub fn get_compaction_threshold_bps(env: Env) -> u32 {
        admin::get_compaction_threshold_bps(&env)
    }

    /// Runs on-demand history compaction for the given asset (admin only) (#247).
    ///
    /// Iterates the full history index and removes entries whose price deviates
    /// less than the configured compaction threshold from their preceding retained
    /// neighbour. The first and last entries are always retained to preserve range
    /// bounds. Returns [`CompactionMetadata`] with before/after entry counts.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    pub fn compact_history(env: Env, asset: Address) -> CompactionMetadata {
        enter_reentrancy_guard(&env);
        let admin = crate::storage::get_admin(&env);
        admin.require_auth();
        let result = history::compact_history(&env, asset);
        exit_reentrancy_guard(&env);
        result
    }

    /// Returns the most recent compaction metadata for an asset, if any.
    pub fn get_compaction_metadata(env: Env, asset: Address) -> Option<CompactionMetadata> {
        history::get_compaction_metadata(&env, asset)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // #251 — History Sharding
    // ─────────────────────────────────────────────────────────────────────────

    /// Migrates existing per-ledger history entries for `asset` into sharded
    /// weekly buckets (admin only) (#251).
    ///
    /// This is a non-destructive, idempotent operation: legacy reads continue
    /// to work after migration. Returns the number of entries migrated.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    pub fn migrate_history_to_shards(env: Env, asset: Address) -> u32 {
        enter_reentrancy_guard(&env);
        let admin = crate::storage::get_admin(&env);
        admin.require_auth();
        let result = history::migrate_history_to_shards(&env, asset);
        exit_reentrancy_guard(&env);
        result
    }

    /// Returns all history entries from the weekly shard bucket that contains `ledger`.
    ///
    /// Transparent to consumers — no knowledge of the sharding scheme is needed.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    pub fn get_bucket_entries(
        env: Env,
        asset: Address,
        ledger: u32,
    ) -> Vec<PriceHistoryEntry> {
        history::get_bucket_entries(&env, asset, ledger)
    }

    // ─────────────────────────────────────────────────────────────────────────
    // #253 — Storage Budget Calculator
    // ─────────────────────────────────────────────────────────────────────────

    /// Estimates current and projected storage costs for a single asset (#253).
    ///
    /// Returns a [`StorageBudget`] with entry counts, estimated TTL costs, and a
    /// monthly cost projection. All figures are advisory estimates based on
    /// approximate Soroban fee constants.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    pub fn get_storage_budget(env: Env, asset: Address) -> StorageBudget {
        enter_reentrancy_guard(&env);
        let result = history::get_storage_budget(&env, asset);
        exit_reentrancy_guard(&env);
        result
    }

    /// Aggregates storage budgets across all registered assets (#253).
    ///
    /// Returns a [`TotalStorageBudget`] summing entry counts and cost projections
    /// for every registered asset. Potentially expensive for large asset sets.
    pub fn get_total_storage_budget(env: Env) -> TotalStorageBudget {
        enter_reentrancy_guard(&env);
        let result = history::get_total_storage_budget(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = admin::get_decimals(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = Asset::Other(Symbol::new(&env, "USD"));
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let registered = read_registered_assets(&env);
        let mut result: Vec<Asset> = Vec::new(&env);
        for i in 0..registered.len() {
            result.push_back(Asset::Stellar(registered.get_unchecked(i)));
        }
        exit_reentrancy_guard(&env);
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
        enter_reentrancy_guard(&env);
        let result = admin::get_resolution(&env);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = prices::lastprice(&env, asset);
        exit_reentrancy_guard(&env);
        result
    }

    /// Consumer-authorized variant of [`lastprice`](Self::lastprice).
    ///
    /// `consumer` must authorize this call and must be permitted under the current
    /// [`ConsumerAccessMode`].
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if `consumer` is not allowed.
    pub fn lastprice_authorized(
        env: Env,
        consumer: Address,
        asset: Asset,
    ) -> Option<PriceData> {
        consumer.require_auth();
        consumer_auth::check_consumer_authorized(&env, &consumer);
        enter_reentrancy_guard(&env);
        let result = prices::lastprice(&env, asset);
        exit_reentrancy_guard(&env);
        result
    }

    /// Returns the TWAP for an asset over a window of ledgers.
    ///
    /// Supports arithmetic and geometric TWAP computation.
    pub fn get_twap(
        env: Env,
        asset: Address,
        window_ledgers: u32,
        method: TwapMethod,
    ) -> Option<PriceData> {
        prices::get_twap(&env, Asset::Stellar(asset), window_ledgers, method)
    }

    pub fn claim_rewards(env: Env) -> i128 {
        challenger::claim_rewards(&env)
    }

    pub fn get_address_roles(env: Env, holder: Address) -> Vec<u32> {
        rbac::get_address_roles(&env, &holder)
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
        enter_reentrancy_guard(&env);
        let result = prices::price(&env, asset, timestamp);
        exit_reentrancy_guard(&env);
        result
    }

    /// Consumer-authorized variant of [`price`](Self::price).
    ///
    /// `consumer` must authorize this call and must be permitted under the current
    /// [`ConsumerAccessMode`].
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if `consumer` is not allowed.
    pub fn price_authorized(
        env: Env,
        consumer: Address,
        asset: Asset,
        timestamp: u64,
    ) -> Option<PriceData> {
        consumer.require_auth();
        consumer_auth::check_consumer_authorized(&env, &consumer);
        enter_reentrancy_guard(&env);
        let result = prices::price(&env, asset, timestamp);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = prices::prices(&env, asset, records);
        exit_reentrancy_guard(&env);
        result
    }

    /// Consumer-authorized variant of [`prices`](Self::prices).
    ///
    /// `consumer` must authorize this call and must be permitted under the current
    /// [`ConsumerAccessMode`].
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if `consumer` is not allowed.
    pub fn prices_authorized(
        env: Env,
        consumer: Address,
        asset: Asset,
        records: u32,
    ) -> Option<Vec<PriceData>> {
        consumer.require_auth();
        consumer_auth::check_consumer_authorized(&env, &consumer);
        enter_reentrancy_guard(&env);
        let result = prices::prices(&env, asset, records);
        exit_reentrancy_guard(&env);
        result
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
        enter_reentrancy_guard(&env);
        let result = pause::is_paused(&env);
        exit_reentrancy_guard(&env);
        result
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

    // --- Storage Migration (#112) ---

    /// Starts or resumes a storage migration to [`CURRENT_VERSION`].
    ///
    /// Admin must authorize. Each call processes up to `batch_size` items
    /// (use `0` for the default of 50). Call repeatedly until
    /// [`get_migration_state`] returns `None`, which signals completion.
    ///
    /// Emits [`MigrationStartedEvent`] on the first call, [`MigrationResumedEvent`]
    /// on subsequent calls, and [`MigrationCompletedEvent`] when finished.
    pub fn migrate_storage(env: Env, batch_size: u32) {
        migration::migrate_storage(&env, batch_size);
    }

    /// Returns the current on-chain storage schema version.
    ///
    /// Returns `1` for contracts deployed before Issue #112.
    pub fn get_storage_version(env: Env) -> u32 {
        migration::get_storage_version(&env)
    }

    /// Returns the active [`MigrationState`], or `None` when no migration is running.
    pub fn get_migration_state(env: Env) -> Option<MigrationState> {
        migration::get_migration_state(&env)
    }

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
        enter_reentrancy_guard(&env);
        let result = timelock::get_timelock_duration(&env);
        exit_reentrancy_guard(&env);
        result
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

    // --- Timelock priority queues ---

    /// Proposes a timelock operation with an explicit priority tier.
    ///
    /// * `priority` — `0` = Urgent (fast), `1` = Normal (default), `2` = LongTerm (slow)
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`]      — caller is not the admin.
    /// * [`ErrorCode::InvalidOperationType`] — `op_type` is not in `0..=7`.
    /// * [`ErrorCode::InvalidPriority`]    — `priority` is not in `0..=2`.
    pub fn propose_operation_with_priority(
        env: Env,
        op_type: u32,
        data: soroban_sdk::Bytes,
        priority: u32,
    ) -> u32 {
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
        let priority_enum = match priority {
            0 => types::OperationPriority::Urgent,
            1 => types::OperationPriority::Normal,
            2 => types::OperationPriority::LongTerm,
            _ => panic_with_error!(&env, ErrorCode::InvalidPriority),
        };
        let result = timelock::propose_operation_with_priority(&env, op_enum, &data, priority_enum);
        reentrancy::exit(&env);
        result
    }

    /// Returns the required delay (in ledgers) for a given priority tier.
    ///
    /// * `priority` — `0` = Urgent, `1` = Normal, `2` = LongTerm.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::InvalidPriority`] — `priority` is not in `0..=2`.
    pub fn get_priority_delay(env: Env, priority: u32) -> u32 {
        let priority_enum = match priority {
            0 => types::OperationPriority::Urgent,
            1 => types::OperationPriority::Normal,
            2 => types::OperationPriority::LongTerm,
            _ => panic_with_error!(&env, ErrorCode::InvalidPriority),
        };
        timelock::get_priority_delay(&env, &priority_enum)
    }

    /// Sets the required delay (in ledgers) for a given priority tier.  Admin-only.
    ///
    /// * `priority` — `0` = Urgent, `1` = Normal, `2` = LongTerm.
    /// * `delay`    — New delay in ledgers.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`]   — caller is not the admin.
    /// * [`ErrorCode::InvalidPriority`] — `priority` is not in `0..=2`.
    pub fn set_priority_delay(env: Env, priority: u32, delay: u32) {
        reentrancy::enter(&env);
        let priority_enum = match priority {
            0 => types::OperationPriority::Urgent,
            1 => types::OperationPriority::Normal,
            2 => types::OperationPriority::LongTerm,
            _ => panic_with_error!(&env, ErrorCode::InvalidPriority),
        };
        timelock::set_priority_delay(&env, priority_enum, delay);
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

    /// Submits prices for multiple (source, asset) legs on behalf of one or more oracle
    /// sources in a single, atomic transaction (#264).
    ///
    /// `relayer` authorizes the batch once; each leg's `source` must additionally
    /// authorize its own leg (per-source auth). Legs must be ordered by non-increasing
    /// `priority_fee` — the on-chain enforcement of the relayer priority fee market
    /// (#266): relayers process higher-fee submissions first, and because each
    /// source's authorization entry covers the exact fee it signed, a relayer cannot
    /// alter it after the fact without invalidating that source's signature. Because a
    /// Soroban invocation is atomic, any leg that fails validation rolls back the
    /// entire batch.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `relayer` - Approved relayer submitting the batch.
    /// * `submissions` - Non-empty batch of [`RelayedSubmission`] legs (at most
    ///   [`relayer::MAX_BATCH_SIZE`]), sorted by non-increasing `priority_fee`.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::ContractPaused`] — contract is paused.
    /// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
    /// * [`ErrorCode::BatchEmpty`] — `submissions` is empty.
    /// * [`ErrorCode::BatchTooLarge`] — `submissions` exceeds the maximum batch size.
    /// * [`ErrorCode::BatchNotFeePrioritized`] — legs are not fee-ordered.
    /// * Any error `submit_price_relayed` raises for an individual leg.
    pub fn submit_prices_relayed(env: Env, relayer: Address, submissions: Vec<RelayedSubmission>) {
        relayer::submit_prices_relayed(&env, relayer, submissions);
    }

    // --- Relayer performance bonds (#265) ---

    /// Sets the required relayer performance bond amount (in stroops). Admin-only.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    pub fn set_relayer_bond_amount(env: Env, amount: i128) {
        relayer_bonds::set_relayer_bond_amount(&env, amount);
    }

    /// Returns the currently configured required relayer bond amount. Defaults to `0`.
    pub fn get_relayer_bond_amount(env: Env) -> i128 {
        relayer_bonds::get_relayer_bond_amount(&env)
    }

    /// Deposits (tops up to) the required performance bond for `relayer`.
    ///
    /// The relayer must authorize this call. A no-op if no bond is required or the
    /// relayer is already fully bonded.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
    /// * [`ErrorCode::StakeTokenNotConfigured`] — no staking token has been configured.
    pub fn deposit_relayer_bond(env: Env, relayer: Address) {
        relayer_bonds::deposit_relayer_bond(&env, relayer);
    }

    /// Returns the currently deposited bond balance (in stroops) for `relayer`.
    pub fn get_relayer_bond_balance(env: Env, relayer: Address) -> i128 {
        relayer_bonds::get_relayer_bond_balance(&env, relayer)
    }

    /// Withdraws the entire deposited performance bond back to `relayer`.
    ///
    /// The relayer must authorize this call. A no-op if nothing is deposited.
    pub fn withdraw_relayer_bond(env: Env, relayer: Address) {
        relayer_bonds::withdraw_relayer_bond(&env, relayer);
    }

    /// Records a failure incident against `relayer` (unauthorized price, invalid
    /// submission, or other operator-attested misbehavior), making it eligible for
    /// slashing once the configured failure threshold is reached. Admin-only.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    /// * [`ErrorCode::RelayerNotAuthorized`] — `relayer` is not admin-approved.
    pub fn record_relayer_failure(env: Env, relayer: Address, reason: RelayerFailureReason) {
        relayer_bonds::record_relayer_failure(&env, relayer, reason);
    }

    /// Returns the number of reported failure incidents for `relayer`.
    pub fn get_relayer_failure_count(env: Env, relayer: Address) -> u32 {
        relayer_bonds::get_relayer_failure_count(&env, relayer)
    }

    /// Slashes a configured percentage of `relayer`'s deposited bond into the shared
    /// treasury. Admin-only.
    ///
    /// Unless `force` is `true`, the relayer's reported failure count must be at or
    /// above the configured failure threshold.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — caller is not the current admin.
    /// * [`ErrorCode::RelayerFailureThresholdNotReached`] — not forced, and below the
    ///   slash-eligibility threshold.
    pub fn slash_relayer(env: Env, relayer: Address, force: bool) {
        relayer_bonds::slash_relayer(&env, relayer, force);
    }

    /// Sets the slash percentage (0-100) applied to a relayer's bond. Admin-only.
    pub fn set_relayer_slash_percent(env: Env, percent: u32) {
        relayer_bonds::set_relayer_slash_percent(&env, percent);
    }

    /// Returns the current relayer slash percentage.
    pub fn get_relayer_slash_percent(env: Env) -> u32 {
        relayer_bonds::get_relayer_slash_percent(&env)
    }

    /// Sets the failure-count threshold at/above which a relayer becomes
    /// slash-eligible. Admin-only.
    pub fn set_relayer_failure_threshold(env: Env, threshold: u32) {
        relayer_bonds::set_relayer_failure_threshold(&env, threshold);
    }

    /// Returns the current relayer failure threshold.
    pub fn get_relayer_failure_threshold(env: Env) -> u32 {
        relayer_bonds::get_relayer_failure_threshold(&env)
    }

    /// Sets the reward rate (in stroops) credited per accuracy-weighted relayed
    /// submission. Admin-only. `0` disables reward accrual.
    pub fn set_relayer_reward_rate(env: Env, rate: i128) {
        relayer_bonds::set_relayer_reward_rate(&env, rate);
    }

    /// Returns the current relayer reward rate in stroops.
    pub fn get_relayer_reward_rate(env: Env) -> i128 {
        relayer_bonds::get_relayer_reward_rate(&env)
    }

    /// Returns the total accumulated reward balance (in stroops) owed to `relayer`.
    pub fn get_relayer_reward_balance(env: Env, relayer: Address) -> i128 {
        relayer_bonds::get_relayer_reward_balance(&env, relayer)
    }

    // --- Relayer dashboard (#267) ---

    /// Returns an aggregated operational [`RelayerDashboard`] for `relayer`: submission
    /// volume, success rate, average latency, fee/reward earnings, bond, per-asset
    /// breakdown, and a comparative percentile rank against every approved relayer.
    pub fn get_relayer_dashboard(env: Env, relayer: Address) -> RelayerDashboard {
        relayer_dashboard::get_relayer_dashboard(&env, relayer)
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
    pub fn set_cross_ref_deviation_bps(env: Env, threshold_bps: u32) {
        cross_reference::set_cross_ref_deviation_bps(&env, threshold_bps);
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
    pub fn get_cross_ref_deviation_bps(env: Env) -> u32 {
        cross_reference::get_cross_ref_deviation_bps(&env)
    }

    // -------------------------------------------------------------------------
    // #227: Per-asset decimal precision configuration
    // -------------------------------------------------------------------------

    /// Sets the decimal precision for a specific asset.
    ///
    /// Allows overriding the contract-wide decimals for individual assets.
    /// For example: BTC=8, USDC=6, governance tokens=18.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset to configure.
    /// * `decimals` - Decimal precision for this asset (0-18).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
    /// * [`ErrorCode::InvalidConfiguration`] — if `decimals > 18`.
    pub fn set_asset_decimals(env: Env, asset: Address, decimals: u32) {
        per_asset_decimals::set_asset_decimals(&env, asset, decimals);
    }

    /// Gets the effective decimal precision for an asset.
    ///
    /// Returns the asset-specific setting if configured, otherwise returns
    /// the contract-wide decimals setting.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset.
    ///
    /// # Returns
    ///
    /// Decimal precision for the asset.
    pub fn get_asset_decimals(env: Env, asset: Address) -> u32 {
        per_asset_decimals::get_asset_decimals(&env, &asset)
    }

    /// Clears the per-asset decimal override, reverting to contract-wide decimals.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Contract address of the asset.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn clear_asset_decimals(env: Env, asset: Address) {
        per_asset_decimals::clear_asset_decimals(&env, asset);
    }

    // -------------------------------------------------------------------------
    // #226: Cross-chain price verification
    // -------------------------------------------------------------------------

    /// Enables or disables cross-chain price verification globally.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `enabled` - `true` to enable verification, `false` to disable.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn set_cross_chain_verification_enabled(env: Env, enabled: bool) {
        cross_chain_verify::set_cross_chain_verification_enabled(&env, enabled);
    }

    /// Checks if cross-chain price verification is currently enabled.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// `true` if verification is enabled, `false` otherwise.
    pub fn is_cross_chain_verification_enabled(env: Env) -> bool {
        cross_chain_verify::is_cross_chain_verification_enabled(&env)
    }

    /// Sets the maximum allowed deviation between this chain and cross-chain prices.
    ///
    /// Expressed in basis points (100 bps = 1%). Maximum allowed is < 10000 (100%).
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `threshold_bps` - Deviation threshold in basis points.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `threshold_bps >= 10000`.
    pub fn set_cross_chain_deviation_threshold(env: Env, threshold_bps: u32) {
        cross_chain_verify::set_cross_chain_deviation_threshold(&env, threshold_bps);
    }

    /// Gets the current cross-chain deviation threshold in basis points.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// Deviation threshold in basis points.
    pub fn get_cross_chain_deviation_threshold(env: Env) -> u32 {
        cross_chain_verify::get_cross_chain_deviation_threshold(&env)
    }

    /// Submits a cross-chain price observation for verification.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `asset` - Asset address.
    /// * `oracle_chain` - Address of the oracle on the other chain.
    /// * `price` - Price from the external oracle.
    /// * `decimals` - Decimal precision of the external price.
    /// * `chain_id` - Identifier of the source chain (e.g., "ethereum").
    /// * `timestamp` - Unix timestamp of the price observation.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
    /// * [`ErrorCode::InvalidPrice`] — if the price is <= 0.
    pub fn submit_cross_chain_price(
        env: Env,
        asset: Address,
        oracle_chain: Address,
        price: i128,
        decimals: u32,
        chain_id: String,
        timestamp: u64,
    ) {
        cross_chain_verify::submit_cross_chain_price(
            &env,
            asset,
            oracle_chain,
            price,
            decimals,
            chain_id,
            timestamp,
        );
    }

    // -------------------------------------------------------------------------
    // #238: Admin operation spending limits
    // -------------------------------------------------------------------------

    /// Sets the daily limit for a specific admin operation type.
    ///
    /// Helps defend against compromised admin keys by limiting damage.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `op_type` - Operation type discriminant (0=AddSource, 1=RemoveSource, etc.).
    /// * `daily_limit` - Maximum operations per day of this type (must be > 0).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `daily_limit == 0`.
    pub fn set_admin_op_daily_limit(env: Env, op_type: u32, daily_limit: u32) {
        admin_op_limits::set_admin_op_daily_limit(&env, op_type, daily_limit);
    }

    /// Gets the daily limit for a specific admin operation type.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `op_type` - Operation type discriminant.
    ///
    /// # Returns
    ///
    /// Daily limit for the operation type.
    pub fn get_admin_op_daily_limit(env: Env, op_type: u32) -> u32 {
        admin_op_limits::get_admin_op_daily_limit(&env, op_type)
    }

    /// Gets the count of operations performed today for a given operation type.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `op_type` - Operation type discriminant.
    ///
    /// # Returns
    ///
    /// Number of operations of this type performed today.
    pub fn get_admin_op_daily_count(env: Env, op_type: u32) -> u32 {
        admin_op_limits::get_admin_op_daily_count(&env, op_type)
    }

    // -------------------------------------------------------------------------
    // #225: Source submission deadline enforcement
    // -------------------------------------------------------------------------

    /// Starts a new aggregation round with a submission deadline window.
    ///
    /// Submissions outside the [start_ledger, end_ledger] window will be excluded.
    /// This prevents last-millisecond price manipulation.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    /// * `start_ledger` - First ledger of the submission window (inclusive).
    /// * `end_ledger` - Last ledger of the submission window (inclusive).
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    /// * [`ErrorCode::InvalidConfiguration`] — if `end_ledger <= start_ledger`.
    pub fn start_aggregation_round(env: Env, start_ledger: u32, end_ledger: u32) {
        submission_deadline::start_aggregation_round(&env, start_ledger, end_ledger);
    }

    /// Gets the current aggregation round configuration, if any.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Returns
    ///
    /// The current [`AggregationRound`] configuration, or `None` if no round is active.
    pub fn get_current_aggregation_round(env: Env) -> Option<crate::types::AggregationRound> {
        submission_deadline::get_current_round(&env)
    }

    /// Clears the current aggregation round, allowing submissions from any ledger.
    ///
    /// # Arguments
    ///
    /// * `env` - The Soroban execution environment.
    ///
    /// # Errors
    ///
    /// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
    pub fn clear_aggregation_round(env: Env) {
        submission_deadline::clear_current_round(&env);
    }

    // =========================================================================
    // #186 — Adaptive Heartbeat / Source Liveness Detection
    // =========================================================================

    /// Returns the health status of a source as a [`SourceHealthStatus`] enum variant.
    ///
    /// - `Healthy` — source is submitting heartbeats and prices within the adaptive interval.
    /// - `Degraded` — source has missed ≥1 heartbeat but is still below the miss threshold.
    /// - `Inactive` — source has exceeded the consecutive-miss threshold.
    /// - `AutoRemoved` — source was automatically removed after extended inactivity.
    ///
    /// This is a read-only query; it does not mutate state.
    pub fn get_source_health(env: Env, source: Address) -> SourceHealthStatus {
        sources::get_source_health(&env, source)
    }

    /// Returns the number of consecutive missed heartbeats for a source.
    pub fn get_missed_heartbeats(env: Env, source: Address) -> u32 {
        sources::get_missed_heartbeats(&env, &source)
    }

    /// Returns the ledger sequence number of the most recent price submission from a source.
    pub fn get_last_price_ledger(env: Env, source: Address) -> u32 {
        sources::get_last_price_ledger(&env, source)
    }

    /// Sets the maximum number of ledgers a source may remain inactive before automatic removal.
    ///
    /// Admin-only. Minimum value is 1. Default is 64.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
    /// * [`ErrorCode::InvalidConfiguration`] — value is 0.
    pub fn set_max_inactive_ledgers(env: Env, ledgers: u32) {
        sources::set_max_inactive_ledgers(&env, ledgers);
    }

    /// Returns the configured max-inactive-ledgers threshold.
    pub fn get_max_inactive_ledgers(env: Env) -> u32 {
        sources::get_max_inactive_ledgers(&env)
    }

    /// Sets the heartbeat window size used in the adaptive-interval formula.
    ///
    /// Admin-only. The window is the denominator in:
    /// `adaptive_interval = base_interval × (window + missed) / window`
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
    /// * [`ErrorCode::InvalidConfiguration`] — value is 0.
    pub fn set_heartbeat_window(env: Env, window: u32) {
        sources::set_heartbeat_window(&env, window);
    }

    /// Returns the configured heartbeat window size (default 10).
    pub fn get_heartbeat_window(env: Env) -> u32 {
        sources::get_heartbeat_window(&env)
    }

    /// Scans all registered sources and automatically removes any that have been inactive
    /// for more than `max_inactive_ledgers` ledgers.
    ///
    /// **Race-condition guard**: never removes the last sources needed to maintain
    /// `min_sources_required` active sources.  Returns the count of sources removed.
    ///
    /// Callable by anyone — no authorization required.
    pub fn check_and_prune_inactive_sources(env: Env) -> u32 {
        sources::check_and_prune_inactive_sources(&env)
    }

    /// Computes the adaptive heartbeat deadline interval for a given miss count.
    ///
    /// Useful for off-chain clients that want to predict when their next heartbeat is due.
    ///
    /// Returns `base_interval × (window + missed) / window`, capped at `3 × base_interval`.
    pub fn compute_adaptive_interval(env: Env, missed: u32) -> u64 {
        let base = admin::get_heartbeat_interval(&env);
        let window = sources::get_heartbeat_window(&env);
        sources::compute_adaptive_interval(base, missed, window)
    }

    // =========================================================================
    // #187 — Commit-Reveal MEV Resistance
    // =========================================================================

    /// Commits a price hash for an asset in the current round.
    ///
    /// The `source` must call this during the open commit window for the round.
    /// The hash must be computed as:
    ///   `sha256(price_le_16bytes || salt_bytes || round_ledger_le_4bytes)`
    ///
    /// The commit is stored in temporary storage for at most
    /// `commit_window + reveal_window + 1` ledgers before automatic expiry.
    ///
    /// # Errors
    /// * [`ErrorCode::SourceNotFound`] — source is not registered.
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    /// * [`ErrorCode::AlreadyCommitted`] — source already committed this round.
    /// * [`ErrorCode::RevealWindowClosed`] — called after commit window closed.
    pub fn commit_price(env: Env, source: Address, asset: Address, hash: soroban_sdk::BytesN<32>) {
        reentrancy::enter(&env);
        prices::commit_price(&env, source, asset, hash);
        reentrancy::exit(&env);
    }

    /// Reveals a committed price for a given round.
    ///
    /// Verifies `sha256(price_le || salt || round_ledger_le)` matches the stored hash.
    /// If the hash matches, the price is stored as a regular `PriceEntry` and aggregation
    /// is triggered.
    ///
    /// Must be called in the window `[round + commit_window, round + commit_window + reveal_window)`.
    ///
    /// # Errors
    /// * [`ErrorCode::CommitNotFound`] — no commit exists for this (source, asset, round).
    /// * [`ErrorCode::CommitExpired`] — reveal window has closed.
    /// * [`ErrorCode::RevealWindowClosed`] — called before reveal window opened.
    /// * [`ErrorCode::CommitHashMismatch`] — hash does not match.
    pub fn reveal_price(
        env: Env,
        source: Address,
        asset: Address,
        price: i128,
        salt: soroban_sdk::Bytes,
        round_ledger: u32,
    ) {
        reentrancy::enter(&env);
        prices::reveal_price(&env, source, asset, price, salt, round_ledger);
        reentrancy::exit(&env);
    }

    /// Reveals up to 100 committed prices in a single atomic transaction.
    ///
    /// Each element is `(asset, price, salt, round_ledger)`. The entire call reverts
    /// if any single reveal fails.
    ///
    /// # Errors
    /// * [`ErrorCode::InvalidConfiguration`] — more than 100 entries provided.
    /// * Same per-entry errors as `reveal_price`.
    pub fn reveal_prices_batch(
        env: Env,
        source: Address,
        reveals: Vec<(Address, i128, soroban_sdk::Bytes, u32)>,
    ) {
        reentrancy::enter(&env);
        prices::reveal_prices_batch(&env, source, reveals);
        reentrancy::exit(&env);
    }

    /// Returns the canonical round-start ledger for the current ledger.
    ///
    /// Computed as `(current_ledger / commit_window) * commit_window`.
    pub fn current_round_ledger(env: Env) -> u32 {
        prices::current_round_ledger(&env)
    }

    /// Sets the commit window length (in ledgers).  Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — value is 0.
    pub fn set_commit_window(env: Env, ledgers: u32) {
        prices::set_commit_window(&env, ledgers);
    }

    /// Returns the current commit window in ledgers (default 20).
    pub fn get_commit_window(env: Env) -> u32 {
        prices::get_commit_window(&env)
    }

    /// Sets the reveal window length (in ledgers).  Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — value is 0.
    pub fn set_reveal_window(env: Env, ledgers: u32) {
        prices::set_reveal_window(&env, ledgers);
    }

    /// Returns the current reveal window in ledgers (default 20).
    pub fn get_reveal_window(env: Env) -> u32 {
        prices::get_reveal_window(&env)
    }

    /// Configures the BFT consensus guardrail for price aggregation.
    ///
    /// When `fault_tolerance > 0`, direct submissions are rejected and sources must
    /// commit and reveal prices before an aggregate is accepted.
    pub fn set_bft_parameters(env: Env, fault_tolerance: u32, method: u32) {
        reentrancy::enter(&env);
        prices::set_bft_parameters(&env, fault_tolerance, method);
        reentrancy::exit(&env);
    }

    /// Returns the configured BFT fault tolerance.
    pub fn get_bft_fault_tolerance(env: Env) -> u32 {
        prices::get_bft_fault_tolerance(&env)
    }

    /// Returns the configured BFT aggregation method.
    pub fn get_bft_aggregation_method(env: Env) -> u32 {
        prices::get_bft_aggregation_method(&env)
    }

    // =========================================================================
    // #188 — Economic Finality Gadget
    // =========================================================================

    /// Places the most-recently aggregated price for an asset into the pending-finality queue.
    ///
    /// Records the current ledger hash for reorg detection and emits
    /// `PricePendingFinalityEvent`.  Normally called by an off-chain keeper after each
    /// aggregation, or can be integrated into an on-chain keeper pattern.
    ///
    /// # Errors
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    /// * [`ErrorCode::NoData`] — no aggregate price exists for the asset.
    pub fn mark_price_pending(env: Env, asset: Address) {
        reentrancy::enter(&env);
        use crate::storage::check_registered_asset;
        check_registered_asset(&env, &asset);
        let agg_key = crate::types::DataKey::Aggregate(asset.clone());
        let agg: AggregatePrice = env
            .storage()
            .persistent()
            .get(&agg_key)
            .unwrap_or_else(|| panic_with_error!(&env, ErrorCode::NoData));
        finality::mark_price_pending(&env, &asset, &agg);
        reentrancy::exit(&env);
    }

    /// Attempts to finalize a pending price entry for an asset at `committed_ledger`.
    ///
    /// Returns `true` if finalization occurred; `false` if the finality window has not
    /// yet elapsed.
    ///
    /// # Errors
    /// * [`ErrorCode::NoData`] — no pending entry exists for this (asset, committed_ledger).
    /// * [`ErrorCode::AlreadyFinalized`] — entry is already finalized.
    /// * [`ErrorCode::PriceRetracted`] — entry was retracted.
    pub fn try_finalize_price(env: Env, asset: Address, committed_ledger: u32) -> bool {
        reentrancy::enter(&env);
        let result = finality::try_finalize_price(&env, &asset, committed_ledger);
        reentrancy::exit(&env);
        result
    }

    /// Returns the most-recently finalized price for an asset.
    ///
    /// `min_finality` is the minimum number of ledgers that must have elapsed since
    /// `committed_ledger` for the caller to accept it.  Use `0` for no minimum.
    ///
    /// # Errors
    /// * [`ErrorCode::NoData`] — no finalized price exists.
    /// * [`ErrorCode::InsufficientFinality`] — price is too new for `min_finality`.
    pub fn get_finalized_price(env: Env, asset: Address, min_finality: u32) -> FinalizedPrice {
        finality::get_finalized_price(&env, asset, min_finality)
    }

    /// Returns the current finality status of a pending price entry.
    ///
    /// # Errors
    /// * [`ErrorCode::NoData`] — no pending entry exists for (asset, committed_ledger).
    pub fn get_finality_status(env: Env, asset: Address, committed_ledger: u32) -> FinalityStatus {
        finality::get_finality_status(&env, asset, committed_ledger)
    }

    /// Admin: retracts a pending price before finalization (reorg protection).
    ///
    /// Must be called before `finality_ledger` is reached.  Emits `PriceRetractedEvent`.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::NoData`] — no pending entry exists.
    /// * [`ErrorCode::AlreadyFinalized`] — price already finalized.
    /// * [`ErrorCode::PriceRetracted`] — price already retracted.
    pub fn retract_price(env: Env, asset: Address, committed_ledger: u32) {
        reentrancy::enter(&env);
        finality::retract_price(&env, asset, committed_ledger);
        reentrancy::exit(&env);
    }

    /// Sets the finality window in ledgers.  Admin-only.  Default is 64.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — value is 0.
    pub fn set_finality_ledgers(env: Env, ledgers: u32) {
        finality::set_finality_ledgers(&env, ledgers);
    }

    /// Returns the configured finality window in ledgers (default 64).
    pub fn get_finality_ledgers(env: Env) -> u32 {
        finality::get_finality_ledgers(&env)
    }

    /// Checks whether the stored ledger hash for `suspect_ledger` indicates a reorg.
    ///
    /// Returns `true` if a reorg is detected, `false` otherwise.
    /// Note: in Soroban v26, only the current ledger can be checked automatically;
    /// for past ledgers, use `retract_price` after external reorg detection.
    pub fn check_reorg(env: Env, asset: Address, suspect_ledger: u32) -> bool {
        finality::check_reorg(&env, asset, suspect_ledger)
    }

    // =========================================================================
    // #176 — Prioritized Submission Fee Market
    // =========================================================================

    /// Enqueues a price submission into the priority fee market buffer.
    ///
    /// The `source` must authorize this call. Submissions are ordered by
    /// `priority_fee DESC, timestamp ASC` and processed in batches via
    /// `process_fee_market`.
    ///
    /// # Errors
    /// * [`ErrorCode::SourceNotFound`] — source is not registered.
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    /// * [`ErrorCode::InvalidPrice`] — price is zero.
    /// * [`ErrorCode::FeeMarketBelowMinimum`] — priority_fee < min_priority_fee.
    pub fn fm_enqueue_submission(
        env: Env,
        source: Address,
        asset: Asset,
        price: u128,
        timestamp: u64,
        priority_fee: u128,
    ) {
        reentrancy::enter(&env);
        fee_market::enqueue_submission(&env, source, asset, price, timestamp, priority_fee);
        reentrancy::exit(&env);
    }

    /// Processes up to 20 queued submissions from the priority buffer.
    ///
    /// Callable by anyone. Returns the number of submissions processed.
    pub fn fm_process_fee_market(env: Env) -> u32 {
        reentrancy::enter(&env);
        let result = fee_market::process_fee_market(&env);
        reentrancy::exit(&env);
        result
    }

    /// Returns the current depth of the fee market priority queue.
    pub fn fm_get_pending_submissions(env: Env) -> u32 {
        fee_market::get_pending_submissions(&env)
    }

    /// Returns the accumulated fee balance owed to a source.
    pub fn fm_get_source_fee_balance(env: Env, source: Address) -> u128 {
        fee_market::get_source_fee_balance(&env, source)
    }

    /// Returns the accumulated treasury fee balance.
    pub fn fm_get_treasury_fee_balance(env: Env) -> u128 {
        fee_market::get_treasury_fee_balance(&env)
    }

    /// Sets the minimum priority fee. Admin only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    pub fn fm_set_min_priority_fee(env: Env, min_fee: u128) {
        fee_market::set_min_priority_fee(&env, min_fee);
    }

    /// Returns the current minimum priority fee. Default: 0.
    pub fn fm_get_min_priority_fee(env: Env) -> u128 {
        fee_market::get_min_priority_fee(&env)
    }

    /// Sets the fee distribution ratio (% to sources, remainder to treasury). Admin only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — ratio > 100.
    pub fn fm_set_fee_distribution_ratio(env: Env, ratio: u32) {
        fee_market::set_fee_distribution_ratio(&env, ratio);
    }

    /// Returns the current fee distribution ratio (% to sources). Default: 80.
    pub fn fm_get_fee_distribution_ratio(env: Env) -> u32 {
        fee_market::get_fee_distribution_ratio(&env)
    }

    /// Sets the treasury address for fee disbursement. Admin only.
    pub fn fm_set_treasury_address(env: Env, treasury: Address) {
        fee_market::set_treasury_address(&env, treasury);
    }

    // =========================================================================
    // #178 — N-of-M Multi-Sig Governance & Ordered Timelock
    // =========================================================================

    /// Sets the governor list and required approval threshold. Admin only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — required is 0 or > governor count.
    pub fn ms_set_governors(env: Env, governors: Vec<Address>, required: u32) {
        multisig::set_governors(&env, governors, required);
    }

    /// Returns the current governor list.
    pub fn ms_get_governors(env: Env) -> Vec<Address> {
        multisig::get_governors(&env)
    }

    /// Returns the required approvals threshold.
    pub fn ms_get_required_approvals(env: Env) -> u32 {
        multisig::get_required_approvals(&env)
    }

    /// Proposes a new multi-sig governance operation.
    ///
    /// Any registered governor may propose. The operation enters the queue
    /// in pending state and requires N-of-M approvals before the timelock starts.
    ///
    /// Returns the assigned operation ID.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — proposer is not a registered governor.
    pub fn ms_propose_operation(
        env: Env,
        proposer: Address,
        op_type: u32,
        data: soroban_sdk::Bytes,
    ) -> u32 {
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
        let result = multisig::propose_ms_operation(&env, proposer, op_enum, data);
        reentrancy::exit(&env);
        result
    }

    /// Approves a pending multi-sig operation.
    ///
    /// Once the N-th approval is submitted the timelock clock starts.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — governor is not registered.
    /// * [`ErrorCode::OperationNotFound`] — no operation with this ID.
    /// * [`ErrorCode::AlreadyApproved`] — governor already approved this op.
    pub fn ms_approve_operation(env: Env, governor: Address, op_id: u32) {
        reentrancy::enter(&env);
        multisig::approve_operation(&env, governor, op_id);
        reentrancy::exit(&env);
    }

    /// Retracts a governor's previous approval.
    ///
    /// If approval count drops below quorum the timelock is paused.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — governor is not registered.
    /// * [`ErrorCode::OperationNotFound`] — no operation with this ID.
    /// * [`ErrorCode::ApprovalNotFound`] — governor had not approved this op.
    pub fn ms_retract_approval(env: Env, governor: Address, op_id: u32) {
        reentrancy::enter(&env);
        multisig::retract_approval(&env, governor, op_id);
        reentrancy::exit(&env);
    }

    /// Executes the head operation of the multi-sig queue after quorum and timelock.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — executor is not admin or governor.
    /// * [`ErrorCode::MsNotQueueHead`] — op_id is not the current queue head.
    /// * [`ErrorCode::MsQuorumNotReached`] — insufficient approvals.
    /// * [`ErrorCode::TimelockNotReady`] — timelock delay not elapsed.
    pub fn ms_execute_operation(env: Env, executor: Address, op_id: u32) {
        reentrancy::enter(&env);
        multisig::execute_ms_operation(&env, executor, op_id);
        reentrancy::exit(&env);
    }

    /// Cancels a pending multi-sig operation. Admin or any governor may cancel.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — canceller is not admin or governor.
    /// * [`ErrorCode::OperationNotFound`] — no operation with this ID.
    pub fn ms_cancel_operation(env: Env, canceller: Address, op_id: u32) {
        reentrancy::enter(&env);
        multisig::cancel_ms_operation(&env, canceller, op_id);
        reentrancy::exit(&env);
    }

    /// Returns a pending multi-sig operation by ID.
    ///
    /// # Errors
    /// * [`ErrorCode::OperationNotFound`] — no operation with this ID.
    pub fn ms_get_operation(env: Env, op_id: u32) -> MultiSigOperation {
        multisig::get_ms_operation(&env, op_id)
    }

    /// Returns the ID of the current queue head (0 if the queue is empty).
    pub fn ms_get_queue_head(env: Env) -> u32 {
        multisig::get_ms_queue_head(&env)
    }

    // =========================================================================
    // #177 — Exotic Asset Fair-Value Pricing Engine
    // =========================================================================

    /// Registers the pricing configuration for an exotic asset. Admin only.
    ///
    /// Supports Direct, LPToken, Index, and Option asset types.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    pub fn exotic_set_config(env: Env, asset: Address, config: AssetPricingConfig) {
        exotic_pricing::set_exotic_asset_config(&env, asset, config);
    }

    /// Returns the pricing configuration for an exotic asset.
    pub fn exotic_get_config(env: Env, asset: Address) -> Option<AssetPricingConfig> {
        exotic_pricing::get_exotic_asset_config(&env, asset)
    }

    /// Computes and returns the fair value of an exotic asset.
    ///
    /// Performs recursive component resolution (max depth 3) with cycle detection.
    /// Returns the price scaled by 10^18.
    ///
    /// # Errors
    /// * [`ErrorCode::ExoticAssetNotConfigured`] — no config set for asset.
    /// * [`ErrorCode::ExoticCycleDetected`] — circular component dependency.
    /// * [`ErrorCode::ExoticCycleLimitExceeded`] — max recursion depth reached.
    /// * [`ErrorCode::NoData`] — a required component price is unavailable.
    pub fn exotic_get_price(env: Env, asset: Address) -> i128 {
        exotic_pricing::get_exotic_price(&env, &asset)
    }

    // =========================================================================
    // #175 — Off-Chain ZK Proof Verification (Groth16/BN254)
    // =========================================================================

    /// Stores the Groth16 verifying key. Admin / governance only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    pub fn zk_set_verification_key(env: Env, vk: Groth16VerifyingKey) {
        zk_verify::set_verification_key(&env, vk);
    }

    /// Returns the stored Groth16 verifying key, or `None` if not configured.
    pub fn zk_get_verification_key(env: Env) -> Option<Groth16VerifyingKey> {
        zk_verify::get_verification_key(&env)
    }

    /// Verifies a Groth16/BN254 proof and submits the attested price if valid.
    ///
    /// `public_signals` must contain at least 3 field elements:
    /// `[asset_id_hash, price, timestamp]`.
    ///
    /// # Errors
    /// * [`ErrorCode::ZkVkNotSet`] — no verifying key has been set.
    /// * [`ErrorCode::ZkProofInvalid`] — proof verification failed.
    /// * [`ErrorCode::ZkInvalidPublicSignals`] — wrong number of public signals.
    /// * [`ErrorCode::SourceNotFound`] — source is not registered.
    /// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
    /// * [`ErrorCode::InvalidPrice`] — attested price is zero or negative.
    pub fn zk_submit_price(
        env: Env,
        source: Address,
        asset: Address,
        proof: Groth16Proof,
        public_signals: Vec<soroban_sdk::BytesN<32>>,
    ) {
        reentrancy::enter(&env);
        zk_verify::submit_zk_price(&env, source, asset, proof, public_signals);
        reentrancy::exit(&env);
    }

    // =========================================================================
    // #179 — State Channel for High-Frequency Price Updates
    // =========================================================================

    /// Opens a state channel for `source` with a locked `deposit`.
    ///
    /// The `source` must authorize this call. The deposit is transferred from
    /// `source` to the contract using the SAC token at `token_contract`.
    ///
    /// # Errors
    /// * [`ErrorCode::ChannelAlreadyOpen`] — channel already exists and is open.
    /// * [`ErrorCode::InvalidPrice`]       — deposit is ≤ 0.
    pub fn sc_open_channel(env: Env, source: Address, deposit: i128, token_contract: Address) {
        reentrancy::enter(&env);
        state_channel::open_channel(&env, source, deposit, token_contract);
        reentrancy::exit(&env);
    }

    /// Submits a batch of signed price updates to an open state channel.
    ///
    /// All items must have strictly increasing nonces. The highest-nonce item
    /// becomes the channel's new state.
    ///
    /// # Errors
    /// * [`ErrorCode::ChannelNotFound`]  — no open channel for `source`.
    /// * [`ErrorCode::NotAuthorized`]    — Ed25519 signature invalid.
    /// * [`ErrorCode::InvalidTimestamp`] — nonces are not strictly increasing.
    pub fn sc_submit_batch(
        env: Env,
        source: Address,
        batch: Vec<BatchItem>,
        signature: BytesN<64>,
        source_pubkey: BytesN<32>,
    ) {
        reentrancy::enter(&env);
        state_channel::submit_batch(&env, source, batch, signature, source_pubkey);
        reentrancy::exit(&env);
    }

    /// Closes an open state channel and refunds the remaining deposit to `source`.
    ///
    /// The `source` must authorize this call.
    ///
    /// # Errors
    /// * [`ErrorCode::ChannelNotFound`] — no open channel for `source`.
    pub fn sc_close_channel(env: Env, source: Address) {
        reentrancy::enter(&env);
        state_channel::close_channel(&env, source);
        reentrancy::exit(&env);
    }

    /// Disputes a state channel when the source has gone offline.
    ///
    /// May be called by anyone after `dispute_timeout` has elapsed. If the
    /// presented batch has a higher nonce and a valid signature, the channel
    /// state is updated.
    ///
    /// # Errors
    /// * [`ErrorCode::ChannelNotFound`]  — no open channel for `source`.
    /// * [`ErrorCode::TimelockNotReady`] — dispute_timeout not yet elapsed.
    /// * [`ErrorCode::NotAuthorized`]    — Ed25519 signature invalid.
    /// * [`ErrorCode::InvalidTimestamp`] — presented nonces do not advance state.
    pub fn sc_dispute_channel(
        env: Env,
        source: Address,
        last_known_batch: Vec<BatchItem>,
        signature: BytesN<64>,
        source_pubkey: BytesN<32>,
    ) {
        reentrancy::enter(&env);
        state_channel::dispute_channel(&env, source, last_known_batch, signature, source_pubkey);
        reentrancy::exit(&env);
    }

    /// Returns the current state of a channel, or `None` if not found.
    pub fn sc_get_channel(env: Env, source: Address) -> Option<StateChannel> {
        state_channel::get_channel(&env, source)
    }

    // =========================================================================
    // #180 — AMM Data Feeds
    // =========================================================================

    /// Initialises a constant-product AMM pool for `asset`. Admin-only.
    ///
    /// Seeds the pool with `initial_x` and `initial_y` reserves.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
    /// * [`ErrorCode::PoolAlreadyExists`]    — pool already exists.
    /// * [`ErrorCode::InvalidConfiguration`] — either initial reserve is ≤ 0.
    pub fn amm_init(
        env: Env,
        asset: Symbol,
        asset_x: Address,
        asset_y: Address,
        initial_x: i128,
        initial_y: i128,
    ) {
        reentrancy::enter(&env);
        amm::init_amm(&env, asset, asset_x, asset_y, initial_x, initial_y);
        reentrancy::exit(&env);
    }

    /// Adds liquidity to an existing AMM pool.
    ///
    /// Transfers `amount_x` and `amount_y` from `caller` to the pool and
    /// recomputes `k`.
    ///
    /// # Errors
    /// * [`ErrorCode::PoolNotFound`] — pool does not exist.
    /// * [`ErrorCode::InvalidPrice`] — either amount is ≤ 0.
    pub fn amm_add_liquidity(
        env: Env,
        caller: Address,
        asset: Symbol,
        amount_x: i128,
        amount_y: i128,
    ) {
        reentrancy::enter(&env);
        amm::add_liquidity(&env, caller, asset, amount_x, amount_y);
        reentrancy::exit(&env);
    }

    /// Executes a constant-product swap in the pool for `asset`.
    ///
    /// Returns the actual output amount received after the 0.3 % fee.
    ///
    /// # Errors
    /// * [`ErrorCode::PoolNotFound`]         — pool not found or disabled.
    /// * [`ErrorCode::InvalidPrice`]         — `amount_in` ≤ 0.
    /// * [`ErrorCode::SlippageExceeded`]     — output < `min_return`.
    /// * [`ErrorCode::AmmPriceManipulation`] — post-swap price deviation too high.
    pub fn amm_swap(
        env: Env,
        caller: Address,
        asset: Symbol,
        from_asset: Address,
        to_asset: Address,
        amount_in: i128,
        min_return: i128,
    ) -> i128 {
        reentrancy::enter(&env);
        let result = amm::swap(
            &env, caller, asset, from_asset, to_asset, amount_in, min_return,
        );
        reentrancy::exit(&env);
        result
    }

    /// Enables or disables an AMM pool. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::PoolNotFound`]  — pool does not exist.
    pub fn amm_set_status(env: Env, asset: Symbol, enabled: bool) {
        reentrancy::enter(&env);
        amm::set_amm_status(&env, asset, enabled);
        reentrancy::exit(&env);
    }

    /// Returns the current pool state, or `None` if not found.
    pub fn amm_get_pool(env: Env, asset: Symbol) -> Option<AmmPool> {
        amm::get_amm_pool(&env, asset)
    }

    /// Sets the maximum allowed AMM-to-oracle price deviation (basis points). Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — `bps > 100_000`.
    pub fn amm_set_max_deviation_bps(env: Env, bps: u32) {
        amm::set_amm_max_deviation_bps(&env, bps);
    }

    /// Returns the current AMM max-deviation setting (basis points). Default: 500.
    pub fn amm_get_max_deviation_bps(env: Env) -> u32 {
        amm::get_amm_max_deviation_bps(&env)
    }

    /// Sets the AMM weight for an asset used during aggregation. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — `weight_bps > 10_000`.
    pub fn amm_set_weight(env: Env, asset: Address, weight_bps: u32, enabled: bool) {
        amm::set_amm_weight(&env, asset, weight_bps, enabled);
    }

    /// Returns the AMM weight configuration for an asset, or `None` if not set.
    pub fn amm_get_weight(env: Env, asset: Address) -> Option<AmmWeightConfig> {
        amm::get_amm_weight(&env, asset)
    }

    /// Registers a Soroswap pool for price derivation. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — either reserve is ≤ 0.
    pub fn soroswap_register_pool(
        env: Env,
        asset_a: Address,
        asset_b: Address,
        reserve_a: i128,
        reserve_b: i128,
        fee_bps: u32,
    ) {
        amm::register_soroswap_pool(&env, asset_a, asset_b, reserve_a, reserve_b, fee_bps);
    }

    /// Returns the Soroswap pool configuration, or `None` if not found.
    pub fn soroswap_get_pool(env: Env, asset_a: Address, asset_b: Address) -> Option<SoroswapPool> {
        amm::get_soroswap_pool(&env, asset_a, asset_b)
    }

    /// Enables or disables a Soroswap pool. Admin-only.
    pub fn soroswap_set_pool_status(env: Env, asset_a: Address, asset_b: Address, enabled: bool) {
        amm::set_soroswap_pool_status(&env, asset_a, asset_b, enabled);
    }

    /// Reads the Soroswap spot price for an asset pair.
    ///
    /// Returns `None` if the pool is disabled or unregistered.
    pub fn get_soroswap_price(env: Env, asset_a: Address, asset_b: Address) -> Option<i128> {
        amm::read_soroswap_price(&env, asset_a, asset_b)
    }

    /// Registers a Stellar DEX pool pair. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — either reserve is ≤ 0.
    pub fn dex_register_pool(
        env: Env,
        asset_a: Address,
        asset_b: Address,
        reserve_a: i128,
        reserve_b: i128,
    ) {
        dex::register_dex_pool(&env, asset_a, asset_b, reserve_a, reserve_b);
    }

    /// Returns the DEX price for `asset` against its paired asset, or `None`.
    pub fn get_dex_price(env: Env, asset: Address) -> Option<DexPrice> {
        dex::get_dex_price(&env, asset)
    }

    /// Returns a serialized state dump for off-chain inspection.
    pub fn oracle_state_dump(env: Env) -> StateDump {
        state_introspection::build_state_dump(&env)
    }

    /// Returns aggregated state analysis statistics.
    pub fn oracle_state_analyze(env: Env) -> StateAnalysis {
        state_introspection::build_state_analysis(&env)
    }

    // =========================================================================
    // #181 — VDF Randomness for Source Sampling
    // =========================================================================

    /// Sets the number of sources to select per VDF sampling round. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`]        — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — `n` is 0.
    pub fn vdf_set_sampling_size(env: Env, n: u32) {
        vdf_sampler::set_sampling_size(&env, n);
    }

    /// Returns the configured VDF sampling size. Default: 3.
    pub fn vdf_get_sampling_size(env: Env) -> u32 {
        vdf_sampler::get_sampling_size(&env)
    }

    /// Returns the current VDF seed derived from ledger sequence and timestamp.
    ///
    /// Off-chain VDF provers call this to obtain the input seed.
    pub fn vdf_get_current_seed(env: Env) -> BytesN<32> {
        vdf_sampler::get_current_seed(&env)
    }

    /// Verifies a VDF proof and returns `true` if it passes the lightweight check.
    ///
    /// This exposes the verifier for off-chain testing purposes. The full
    /// `sample_sources` call internally invokes this check.
    pub fn vdf_verify_proof(
        env: Env,
        seed: BytesN<32>,
        proof: Bytes,
        iterations: u64,
        output: BytesN<32>,
    ) -> bool {
        vdf_sampler::verify_vdf_proof(&env, seed, proof, iterations, output)
    }

    /// Samples `n` source addresses using VDF randomness.
    ///
    /// Verifies the proof against the current ledger seed. Falls back to all
    /// registered sources if the proof is empty or invalid.
    ///
    /// # Returns
    ///
    /// A `Vec<Address>` of selected source addresses.
    pub fn vdf_sample_sources(
        env: Env,
        proof: Bytes,
        output: BytesN<32>,
        iterations: u64,
    ) -> Vec<Address> {
        vdf_sampler::sample_sources(&env, proof, output, iterations)
    }

    // =========================================================================
    // #182 — Cross-Chain Price Relay
    // =========================================================================

    /// Emits a structured cross-chain price update event for `asset_symbol`.
    ///
    /// Should be called after a successful price aggregation. The event is
    /// indexed under `(symbol!("price_upd"), asset_symbol)` for off-chain
    /// relayers to pick up.
    pub fn relay_emit_price_update(env: Env, asset_symbol: Symbol, payload: PriceEventPayload) {
        cross_chain_relay::emit_price_update(&env, asset_symbol, payload);
    }

    /// Configures cross-chain relay settings (quorum threshold, Merkle path bits).
    /// Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    pub fn relay_set_config(env: Env, config: CrossChainRelayConfig) {
        cross_chain_relay::set_relay_config(&env, config);
    }

    /// Returns the current cross-chain relay configuration, or `None` if not set.
    pub fn relay_get_config(env: Env) -> Option<CrossChainRelayConfig> {
        cross_chain_relay::get_relay_config(&env)
    }

    /// Verifies SCP validator quorum signatures over a Stellar ledger header hash.
    ///
    /// Returns `true` when the required fraction of validators (per config) have
    /// produced valid Ed25519 signatures.
    pub fn relay_verify_validator_set(
        env: Env,
        header_hash: BytesN<32>,
        validators: Vec<BytesN<32>>,
        signatures: Vec<BytesN<64>>,
    ) -> bool {
        cross_chain_relay::verify_validator_set(&env, header_hash, validators, signatures)
    }

    /// Verifies a SHA-256 Merkle proof authenticating a price event in a Stellar ledger.
    ///
    /// Returns `true` if the proof resolves to `header_hash`.
    pub fn relay_verify_event_proof(
        env: Env,
        header_hash: BytesN<32>,
        proof: Vec<BytesN<32>>,
        event_data: PriceEventPayload,
    ) -> bool {
        cross_chain_relay::verify_event_proof(&env, header_hash, proof, event_data)
    }

    /// Checks internal consistency of a `StellarHeader` by verifying its hash.
    ///
    /// Returns `true` if `sha256(sequence || tx_set_hash || bucket_list_hash)`
    /// matches `header.expected_hash`.
    pub fn relay_verify_header(env: Env, header: StellarHeader) -> bool {
        cross_chain_relay::verify_header_consistency(&env, &header)
    }

    // =========================================================================
    // #245 — Admin Key Social Recovery
    // =========================================================================

    /// Registers the guardian set and required approval threshold. Admin-only.
    /// Replaces any previously configured guardian set.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidGuardianConfig`] — threshold is `0` or exceeds guardian count.
    pub fn recovery_set_guardians(env: Env, guardians: Vec<Address>, threshold: u32) {
        recovery::set_guardians(&env, guardians, threshold);
    }

    /// Returns the currently registered guardian addresses.
    pub fn recovery_get_guardians(env: Env) -> Vec<Address> {
        recovery::get_guardians(&env)
    }

    /// Returns the number of guardian approvals required to reach recovery threshold.
    pub fn recovery_get_threshold(env: Env) -> u32 {
        recovery::get_recovery_threshold(&env)
    }

    /// Sets the cancellation-window delay in ledgers between reaching guardian
    /// threshold and a recovery becoming eligible for auto-execution. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — `delay_ledgers` is `0`.
    pub fn recovery_set_delay(env: Env, delay_ledgers: u32) {
        recovery::set_recovery_delay(&env, delay_ledgers);
    }

    /// Returns the configured cancellation-window delay in ledgers. Default: ~1 day.
    pub fn recovery_get_delay(env: Env) -> u32 {
        recovery::get_recovery_delay(&env)
    }

    /// A guardian approves recovery, naming `new_admin` as the candidate replacement
    /// admin. The first guardian to call this initiates the recovery; once the
    /// configured threshold of distinct guardian approvals is reached, the
    /// cancellation-window delay starts.
    ///
    /// # Errors
    /// * [`ErrorCode::NotGuardian`] — caller is not a registered guardian.
    /// * [`ErrorCode::RecoveryAlreadyPending`] — a recovery is already pending for a
    ///   different candidate; the admin must cancel it first.
    /// * [`ErrorCode::RecoveryAlreadyApproved`] — this guardian already approved.
    pub fn recovery_approve(env: Env, guardian: Address, new_admin: Address) {
        reentrancy::enter(&env);
        recovery::approve_recovery(&env, guardian, new_admin);
        reentrancy::exit(&env);
    }

    /// Cancels the pending recovery. Admin-only — the cancellation window that lets
    /// a still-in-control admin stop a recovery before it executes.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::RecoveryNotPending`] — no recovery is currently pending.
    pub fn recovery_cancel(env: Env) {
        reentrancy::enter(&env);
        recovery::cancel_recovery(&env);
        reentrancy::exit(&env);
    }

    /// Executes a ready recovery, installing its candidate as the new contract admin.
    /// Callable by anyone once guardian threshold has been reached and the
    /// cancellation-window delay has elapsed.
    ///
    /// # Errors
    /// * [`ErrorCode::RecoveryNotPending`] — no recovery is currently pending.
    /// * [`ErrorCode::RecoveryDelayNotElapsed`] — threshold not yet reached, or the
    ///   cancellation-window delay has not yet elapsed.
    pub fn recovery_execute(env: Env) {
        reentrancy::enter(&env);
        recovery::execute_recovery(&env);
        reentrancy::exit(&env);
    }

    /// Returns the currently pending recovery, if any.
    pub fn recovery_get_pending(env: Env) -> Option<GuardianRecovery> {
        recovery::get_pending_recovery(&env)
    }

    // =========================================================================
    // #283 — Stellar DID Integration
    // =========================================================================

    /// Registers a DID document under `did_address`. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — document exceeds length limit.
    pub fn did_register(env: Env, did_address: Address, document: String) {
        did::register_did(&env, did_address, document);
    }

    /// Links an oracle source to a DID address. Admin-only.
    pub fn did_link_source(env: Env, source: Address, did: Address, verified: bool) {
        did::link_source_did(&env, source, did, verified);
    }

    /// Verifies a DID document exists on-chain.
    pub fn did_verify(env: Env, did_address: Address) -> bool {
        did::verify_did(&env, did_address)
    }

    /// Returns the DID document for a given DID address, or `None`.
    pub fn did_get_document(env: Env, did_address: Address) -> Option<String> {
        did::get_did_document(&env, did_address)
    }

    /// Returns the DID link for a source, or `None`.
    pub fn did_get_source_link(env: Env, source: Address) -> Option<SourceDidLink> {
        did::get_source_did(&env, source)
    }

    /// Returns all source-DID links.
    pub fn did_get_all_source_links(env: Env) -> Vec<SourceDidLink> {
        did::get_all_source_dids(&env)
    }

    // =========================================================================
    // #282 — Bridge Oracle for Non-Stellar Assets
    // =========================================================================

    /// Registers a bridge oracle contract for a non-Stellar asset pair. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not admin.
    /// * [`ErrorCode::InvalidConfiguration`] — validation fails.
    pub fn bridge_register_oracle(env: Env, config: BridgeOracleConfig) {
        bridge_oracle::register_bridge_oracle(&env, config);
    }

    /// Returns the bridge oracle configuration for an asset pair, or `None`.
    pub fn bridge_get_oracle(env: Env, source_asset: Address, target_asset: Address) -> Option<BridgeOracleConfig> {
        bridge_oracle::get_bridge_oracle(&env, source_asset, target_asset)
    }

    /// Submits a bridged price observation. Must be called by the bridge oracle contract.
    ///
    /// # Errors
    /// * [`ErrorCode::NotAuthorized`] — caller is not the bridge oracle.
    /// * [`ErrorCode::InvalidConfiguration`] — price is non-positive.
    pub fn bridge_submit_price(env: Env, source_asset: Address, target_asset: Address, price: i128, timestamp: u64) {
        bridge_oracle::submit_bridged_price(&env, source_asset, target_asset, price, timestamp);
    }

    /// Returns the latest bridged price for an asset pair, or `None`.
    pub fn bridge_get_price(env: Env, source_asset: Address, target_asset: Address) -> Option<BridgedPrice> {
        bridge_oracle::get_bridged_price(&env, source_asset, target_asset)
    }

    /// Normalizes a raw bridge price into the oracle decimal scale.
    pub fn bridge_normalize_price(env: Env, raw_price: i128, target_decimals: u32, config: BridgeOracleConfig) -> i128 {
        bridge_oracle::normalize_bridged_price(&env, raw_price, target_decimals, &config)
    }

    // =========================================================================
    // #285 — Ecosystem Metadata Registration
    // =========================================================================

    /// Registers the oracle contract in the Stellar ecosystem metadata registry. Admin-only.
    pub fn metadata_register(env: Env, metadata: EcosystemMetadata) {
        ecosystem_metadata::register_ecosystem_metadata(&env, metadata);
    }

    /// Updates the ecosystem metadata. Admin-only.
    pub fn metadata_update(env: Env, metadata: EcosystemMetadata) {
        ecosystem_metadata::update_ecosystem_metadata(&env, metadata);
    }

    /// Returns the ecosystem metadata, or `None`.
    pub fn metadata_get(env: Env) -> Option<EcosystemMetadata> {
        ecosystem_metadata::get_ecosystem_metadata(&env)
    }

    /// Registers a price feed in the ecosystem metadata directory. Admin-only.
    pub fn metadata_register_feed(env: Env, feed: FeedMetadata) {
        ecosystem_metadata::register_feed_metadata(&env, feed);
    }

    /// Returns all registered feed metadata.
    pub fn metadata_list_feeds(env: Env) -> Vec<FeedMetadata> {
        ecosystem_metadata::list_feed_metadata(&env)
    }

    /// Returns feed metadata for a specific asset, or `None`.
    pub fn metadata_get_feed(env: Env, asset: Address) -> Option<FeedMetadata> {
        ecosystem_metadata::get_feed_metadata(&env, asset)
    }

    // =========================================================================
    // Cosmos/IBC Light-Client Verified Price Feeds
    // =========================================================================

    /// Configures the IBC light client for a counterparty Cosmos chain. Admin-only.
    pub fn ibc_update_light_client(env: Env, client_state: IbcClientState) {
        ibc_oracle::update_light_client(&env, client_state);
    }

    /// Returns the current IBC light client configuration, or `None`.
    pub fn ibc_get_light_client(env: Env) -> Option<IbcClientState> {
        ibc_oracle::get_light_client(&env)
    }

    /// Submits a validator-signed consensus state for a revision height. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::IbcClientNotSet`] — no light client configured.
    /// * [`ErrorCode::IbcQuorumNotMet`] — signed voting power below the trust threshold.
    pub fn ibc_submit_consensus_state(
        env: Env,
        consensus_state: IbcConsensusState,
        validators: Vec<TendermintValidator>,
        signatures: Vec<BytesN<64>>,
    ) {
        ibc_oracle::submit_consensus_state(&env, consensus_state, validators, signatures);
    }

    /// Returns the trusted consensus state at a revision height, or `None`.
    pub fn ibc_get_consensus_state(env: Env, revision_height: u64) -> Option<IbcConsensusState> {
        ibc_oracle::get_consensus_state(&env, revision_height)
    }

    /// Maps an IBC denom to a registered Stellar asset. Admin-only.
    pub fn ibc_register_asset(env: Env, denom: String, asset: Address) {
        ibc_oracle::register_ibc_asset_mapping(&env, denom, asset);
    }

    /// Returns the Stellar asset mapped to an IBC denom, or `None`.
    pub fn ibc_get_asset(env: Env, denom: String) -> Option<Address> {
        ibc_oracle::get_ibc_asset(&env, denom)
    }

    /// Verifies an IBC price packet's Merkle proof against the trusted app hash
    /// for its revision height, then stores the resulting price.
    ///
    /// # Errors
    /// * [`ErrorCode::IbcDenomNotMapped`], [`ErrorCode::IbcConsensusStateNotFound`],
    ///   [`ErrorCode::IbcClientExpired`], [`ErrorCode::IbcPacketReplayed`],
    ///   [`ErrorCode::IbcInvalidProof`], [`ErrorCode::InvalidPrice`].
    pub fn ibc_submit_price(
        env: Env,
        packet: IbcPricePacket,
        proof: Vec<BytesN<32>>,
        path_bits: u32,
    ) {
        ibc_oracle::verify_and_submit_price(&env, packet, proof, path_bits);
    }

    /// Returns the latest verified IBC price for a denom, or `None`.
    pub fn ibc_get_price(env: Env, denom: String) -> Option<IbcPriceEntry> {
        ibc_oracle::get_ibc_price(&env, denom)
    }

    // =========================================================================
    // Ethereum Bridge Price Feeds
    // =========================================================================

    /// Sets the Ethereum bridge configuration (relayer, finality/staleness). Admin-only.
    pub fn eth_set_bridge_config(env: Env, config: EthBridgeConfig) {
        eth_bridge::set_eth_bridge_config(&env, config);
    }

    /// Returns the current Ethereum bridge configuration, or `None`.
    pub fn eth_get_bridge_config(env: Env) -> Option<EthBridgeConfig> {
        eth_bridge::get_eth_bridge_config(&env)
    }

    /// Maps an ERC-20 token address to a registered Stellar asset. Admin-only.
    pub fn eth_map_asset(env: Env, erc20: BytesN<20>, asset: Address) {
        eth_bridge::map_erc20_asset(&env, erc20, asset);
    }

    /// Returns the ERC-20 address mapped to a Stellar asset, or `None`.
    pub fn eth_get_erc20_for_asset(env: Env, asset: Address) -> Option<BytesN<20>> {
        eth_bridge::get_erc20_for_asset(&env, asset)
    }

    /// Submits a bridged Ethereum price update. Must be called by the configured relayer.
    ///
    /// # Errors
    /// * [`ErrorCode::EthBridgeNotConfigured`], [`ErrorCode::EthAssetNotMapped`],
    ///   [`ErrorCode::EthInsufficientFinality`], [`ErrorCode::EthPriceStale`],
    ///   [`ErrorCode::EthOutOfOrder`], [`ErrorCode::InvalidPrice`].
    pub fn eth_submit_price(env: Env, msg: EthPriceMessage) {
        eth_bridge::submit_eth_price(&env, msg);
    }

    /// Returns the latest bridged Ethereum price for an ERC-20 address, or `None`.
    pub fn eth_get_price(env: Env, erc20: BytesN<20>) -> Option<EthBridgedPrice> {
        eth_bridge::get_eth_price(&env, erc20)
    }

    /// Returns the latest bridged Ethereum price for a mapped Stellar asset, or `None`.
    pub fn eth_get_price_for_asset(env: Env, asset: Address) -> Option<EthBridgedPrice> {
        eth_bridge::get_eth_price_for_asset(&env, asset)
    }

    // =========================================================================
    // Source Accuracy Calibration
    // =========================================================================

    /// Sets the global calibration configuration. Admin-only.
    pub fn calibration_set_config(env: Env, config: CalibrationConfig) {
        calibration::set_calibration_config(&env, config);
    }

    /// Returns the current calibration configuration (defaults if unset).
    pub fn calibration_get_config(env: Env) -> CalibrationConfig {
        calibration::get_calibration_config(&env)
    }

    /// Sets the reference benchmark price for an asset. Admin-only.
    pub fn calibration_set_benchmark(env: Env, asset: Address, reference_price: i128, decimals: u32, timestamp: u64) {
        calibration::set_calibration_benchmark(&env, asset, reference_price, decimals, timestamp);
    }

    /// Returns the reference benchmark for an asset, or `None`.
    pub fn calibration_get_benchmark(env: Env, asset: Address) -> Option<CalibrationBenchmark> {
        calibration::get_calibration_benchmark(&env, asset)
    }

    /// Records one accuracy sample for (asset, source) against the stored
    /// benchmark and updates its rolling calibration score. Admin-only.
    ///
    /// # Errors
    /// * [`ErrorCode::CalibrationBenchmarkNotFound`] — no benchmark set for the asset.
    pub fn calibration_record_sample(env: Env, asset: Address, source: Address, source_price: i128) {
        calibration::record_calibration_sample(&env, asset, source, source_price);
    }

    /// Returns the rolling calibration score for (asset, source) (neutral default if none).
    pub fn calibration_get_score(env: Env, asset: Address, source: Address) -> CalibrationScore {
        calibration::get_calibration_score(&env, asset, source)
    }

    /// Returns the calibration-derived aggregation weight (bps) for (asset, source);
    /// `0` if calibration is disabled, the source lacks enough samples, or no score exists.
    pub fn calibration_get_weight(env: Env, asset: Address, source: Address) -> u32 {
        calibration::get_calibration_weight(&env, asset, source)
    }

    /// Returns calibration scores for every registered source against `asset`.
    pub fn calibration_report(env: Env, asset: Address) -> Vec<CalibrationScore> {
        calibration::calibration_report(&env, asset)
    }
}

#[cfg(test)]
mod test_helpers;

#[cfg(test)]
mod test;

#[cfg(test)]
mod relayer_tests;

#[cfg(test)]
mod asset_registry_gas_tests;

#[cfg(test)]
mod heartbeat_tests;

#[cfg(test)]
mod commit_reveal_tests;

#[cfg(test)]
mod bft_tests;

#[cfg(test)]
mod finality_tests;

#[cfg(test)]
mod correlation_feature_tests;

#[cfg(test)]
mod did_bridge_metadata_tests;

#[cfg(test)]
mod issue_307_alert_rules_tests;

#[cfg(test)]
mod issue_308_health_monitoring_tests;

#[cfg(test)]
mod issue_309_rate_limiting_tests;

#[cfg(test)]
mod issue_310_fee_market_tests;
