use soroban_sdk::{contractevent, Address, Bytes, BytesN, String, Symbol};

/// Publishes a generic admin-action audit event.
///
/// Used by every admin-mutating function to emit a consistent on-chain audit trail.
/// Callers pass a short `action` symbol (≤8 chars), the acting `admin` address, and
/// optional arbitrary `data` bytes (may be empty).
#[allow(deprecated)]
pub fn emit_admin_action(env: &soroban_sdk::Env, action: Symbol, admin: Address, data: Bytes) {
    env.events().publish((action, admin), (data,));
}

// ContractInitializedEvent uses manual publishing due to String field
// limitations with the macro in soroban-sdk 26.

/// Emitted when a source submits a new price for an asset.
///
/// Topics: `asset`, `source`
#[contractevent]
#[derive(Clone)]
pub struct PriceSubmittedEvent {
    /// Address of the asset whose price was submitted.
    #[topic]
    pub asset: Address,
    /// Address of the oracle source that submitted the price.
    #[topic]
    pub source: Address,
    /// Raw price value scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp (seconds) provided by the source.
    pub timestamp: u64,
}

/// Emitted when a new optimistic price proposal is created.
///
/// Topics: `asset`, `proposer`
#[contractevent]
#[derive(Clone)]
pub struct PriceProposalCreatedEvent {
    /// Address of the asset for which the proposal was made.
    #[topic]
    pub asset: Address,
    /// Address of the proposer.
    #[topic]
    pub proposer: Address,
    /// Monotonic proposal id assigned by the contract.
    pub proposal_id: u32,
    /// Proposed price value.
    pub price: i128,
    /// Proposed timestamp.
    pub timestamp: u64,
    /// Bond amount posted for the proposal.
    pub bond_amount: i128,
    /// Ledger at which the proposal becomes final if not disputed.
    pub expires_at_ledger: u32,
}

/// Emitted when an optimistic price proposal is disputed.
///
/// Topics: `proposal_id`, `disputer`
#[contractevent]
#[derive(Clone)]
pub struct PriceProposalDisputedEvent {
    /// Proposal id being disputed.
    #[topic]
    pub proposal_id: u32,
    /// Address of the disputer.
    #[topic]
    pub disputer: Address,
    /// Bond amount posted by the disputer.
    pub bond_amount: i128,
}

/// Emitted when an optimistic price proposal is resolved.
///
/// Topics: `proposal_id`
#[contractevent]
#[derive(Clone)]
pub struct PriceProposalResolvedEvent {
    /// Proposal id being resolved.
    #[topic]
    pub proposal_id: u32,
    /// Whether the proposal was accepted by the admin.
    pub approved: bool,
    /// Whether the proposal was finalized into an aggregate price.
    pub finalized: bool,
}

/// Emitted when an optimistic proposal is resolved using external off-chain data (#291).
///
/// Topics: `proposal_id`
#[contractevent]
#[derive(Clone)]
pub struct ExternalDataResolvedEvent {
    #[topic]
    pub proposal_id: u32,
    pub external_price: i128,
    pub resolver: Address,
}

/// Emitted when the aggregate price for an asset changes.
///
/// Topics: `asset`
#[allow(dead_code)]
#[contractevent]
#[derive(Clone)]
pub struct PriceUpdatedEvent {
    /// Address of the asset whose aggregate price changed.
    #[topic]
    pub asset: Address,
    /// Newly computed aggregate price.
    pub new_price: i128,
    /// Previous aggregate price before this update.
    pub old_price: i128,
    /// Unix timestamp of the new aggregate.
    pub timestamp: u64,
    /// Unix timestamp of the previous aggregate.
    pub prev_timestamp: u64,
    /// Decimal precision applied to both price values.
    pub decimals: u32,
}

/// Emitted when a new oracle source is registered by the admin.
///
/// Topics: `source`, `admin`
#[contractevent]
#[derive(Clone)]
pub struct SourceAddedEvent {
    /// Address of the newly added oracle source.
    #[topic]
    pub source: Address,
    /// Address of the admin who performed the action.
    #[topic]
    pub admin: Address,
    /// Human-readable display name assigned to the source.
    pub name: String,
}

/// Emitted when an oracle source is de-registered by the admin.
///
/// Topics: `source`, `admin`
#[contractevent]
#[derive(Clone)]
pub struct SourceRemovedEvent {
    /// Address of the removed oracle source.
    #[topic]
    pub source: Address,
    /// Address of the admin who performed the action.
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceAssetAddedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub asset: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceAssetRemovedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub asset: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceVerificationSetEvent {
    #[topic]
    pub source: Address,
    pub verified: bool,
    pub verification_method: String,
    pub verifier: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceKeyRotatedEvent {
    #[topic]
    pub old_source: Address,
    #[topic]
    pub new_source: Address,
    pub ledger: u32,
}

/// Emitted when a new asset is registered for price tracking.
///
/// Topics: `asset`, `admin`
#[contractevent]
#[derive(Clone)]
pub struct AssetRegisteredEvent {
    /// Address of the newly registered asset.
    #[topic]
    pub asset: Address,
    /// Address of the admin who registered the asset.
    #[topic]
    pub admin: Address,
}

/// Emitted when a previously registered asset is removed.
///
/// Topics: `asset`, `admin`
#[contractevent]
#[derive(Clone)]
pub struct AssetUnregisteredEvent {
    /// Address of the asset that was removed.
    #[topic]
    pub asset: Address,
    /// Address of the admin who removed the asset.
    #[topic]
    pub admin: Address,
}

/// Emitted when the contract administrator is replaced.
///
/// Topics: `old_admin`, `new_admin`
#[contractevent]
#[derive(Clone)]
pub struct AdminChangedEvent {
    /// Address of the outgoing administrator.
    #[topic]
    pub old_admin: Address,
    /// Address of the incoming administrator.
    #[topic]
    pub new_admin: Address,
}

/// Emitted when the contract's WASM is upgraded to a new hash.
///
/// Topics: `new_wasm_hash`
#[contractevent]
#[derive(Clone)]
pub struct ContractUpgradedEvent {
    /// 32-byte hash of the new WASM module.
    #[topic]
    pub new_wasm_hash: soroban_sdk::BytesN<32>,
}

/// Emitted when `min_sources_required` is updated.
#[contractevent]
#[derive(Clone)]
pub struct MinSourcesChangedEvent {
    /// The new minimum-sources threshold.
    pub value: u32,
}

/// Emitted when `max_history_length` is updated.
#[contractevent]
#[derive(Clone)]
pub struct MaxHistoryChangedEvent {
    /// The new maximum history length (in entries per asset).
    pub value: u32,
}

/// Emitted when the price resolution window is updated.
#[contractevent]
#[derive(Clone)]
pub struct ResolutionChangedEvent {
    /// The new resolution value in seconds.
    pub value: u32,
}

/// Emitted when the decimal precision setting is updated.
#[contractevent]
#[derive(Clone)]
pub struct DecimalsChangedEvent {
    /// The new number of decimals.
    pub value: u32,
}

/// Emitted when the contract description is updated.
#[contractevent]
#[derive(Clone)]
pub struct DescriptionChangedEvent {
    /// The new human-readable description string.
    pub description: String,
}

/// Emitted when a price aggregation attempt fails due to too few contributing sources.
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct SourcesInsufficientEvent {
    /// Address of the asset for which aggregation failed.
    #[topic]
    pub asset: Address,
    /// Number of sources that had submitted prices at the time of the attempt.
    pub current_source_count: u32,
    /// Minimum number of sources required for aggregation to succeed.
    pub min_sources_required: u32,
}

/// Publishes the contract-initialized event.
///
/// Uses manual event publishing because `String` fields are not yet supported
/// by the `#[contractevent]` macro in soroban-sdk 26.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `admin` - Address set as the initial administrator.
/// * `min_sources` - Effective minimum-sources threshold (after defaulting).
/// * `max_history` - Effective maximum-history length (after defaulting).
/// * `decimals` - Decimal precision configured at initialization.
/// * `description` - Human-readable description string.
#[allow(deprecated)]
pub fn emit_initialized(
    env: &soroban_sdk::Env,
    admin: Address,
    min_sources: u32,
    max_history: u32,
    decimals: u32,
    description: String,
) {
    let sym = soroban_sdk::symbol_short!("init");
    env.events().publish(
        (sym, admin),
        (min_sources, max_history, decimals, description),
    );
}

/// Emitted each time a successful price aggregation occurs for an asset.
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct PriceAggregatedEvent {
    /// Address of the asset whose price was aggregated.
    #[topic]
    pub asset: Address,
    /// Newly computed aggregate price.
    pub price: i128,
    /// Number of sources that contributed to this aggregate.
    pub num_sources: u32,
    /// Unix timestamp of the most-recent contributing submission.
    pub timestamp: u64,
}

/// Emitted when an asset's circuit breaker trips and the update is rejected.
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct CircuitBreakerTrippedEvent {
    /// Address of the asset that triggered the breaker.
    #[topic]
    pub asset: Address,
    /// Previous aggregate price before the rejected update.
    pub previous_price: i128,
    /// Candidate aggregate price that would have been published.
    pub candidate_price: i128,
    /// Change amount in basis points that exceeded the configured limit.
    pub change_bps: u32,
    /// Maximum allowed change in basis points for a single ledger.
    pub max_change_bps: u32,
    /// Ledger at which the breaker tripped.
    pub ledger: u32,
    /// Unix timestamp of the breaker trip.
    pub timestamp: u64,
}

/// Emitted when the circuit breaker is manually reset by the admin.
///
/// Topics: `asset`, `admin`
#[contractevent]
#[derive(Clone)]
pub struct CircuitBreakerResetEvent {
    /// Address of the asset whose breaker was reset.
    #[topic]
    pub asset: Address,
    /// Admin who reset the breaker.
    #[topic]
    pub admin: Address,
}

/// Emitted when the oldest history entry for an asset is pruned to enforce `max_history_length`.
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct HistoryPrunedEvent {
    /// Address of the asset whose history was pruned.
    #[topic]
    pub asset: Address,
    /// Ledger sequence number of the entry that was removed.
    pub pruned_ledger: u32,
    /// Number of history entries remaining after pruning.
    pub remaining: u32,
}

/// Publishes the timestamp-threshold-changed event.
///
/// Uses manual event publishing because `u64` values in `#[contractevent]` trigger
/// a macro limitation in soroban-sdk 26.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `admin` - Address of the admin who made the change.
/// * `value` - New timestamp threshold in seconds.
#[allow(deprecated)]
pub fn emit_timestamp_threshold_changed(env: &soroban_sdk::Env, admin: Address, value: u64) {
    let sym = soroban_sdk::symbol_short!("tthr");
    env.events().publish((sym, admin), (value,));
}

/// Emitted when a source's submitted price deviates excessively from the current aggregate.
///
/// Topics: `asset`, `source`
#[allow(dead_code)]
#[contractevent]
#[derive(Clone)]
pub struct PriceDeviationFlaggedEvent {
    /// Address of the asset for which the deviation was detected.
    #[topic]
    pub asset: Address,
    /// Address of the source whose submission triggered the flag.
    #[topic]
    pub source: Address,
    /// Price submitted by the flagged source.
    pub price: i128,
    /// Current aggregate (median) price used as the reference.
    pub median_price: i128,
    /// Deviation magnitude expressed as a percentage (0–100).
    pub deviation_percent: u32,
}

/// Publishes the max-price-deviation-changed event.
///
/// Uses manual event publishing because the `#[contractevent]` macro does not
/// yet support all field types cleanly in soroban-sdk 26.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `admin` - Address of the admin who made the change.
/// * `value` - New maximum deviation in basis points (100 bp = 1 %).
#[allow(deprecated)]
pub fn emit_max_price_deviation_changed(env: &soroban_sdk::Env, admin: Address, value: u32) {
    let sym = soroban_sdk::symbol_short!("devn");
    env.events().publish((sym, admin), (value,));
}

/// Emitted when an oracle source submits a liveness heartbeat.
///
/// Topics: `source`
#[contractevent]
#[derive(Clone)]
pub struct SourceHeartbeatEvent {
    /// Address of the source that submitted the heartbeat.
    #[topic]
    pub source: Address,
    /// Unix timestamp of the ledger at which the heartbeat was recorded.
    pub timestamp: u64,
}

/// Emitted when a source is detected as inactive (heartbeat overdue).
///
/// Topics: `source`
#[contractevent]
#[derive(Clone)]
pub struct SourceInactiveEvent {
    /// Address of the source that was flagged inactive.
    #[topic]
    pub source: Address,
    /// Unix timestamp of the source's last recorded heartbeat.
    pub last_heartbeat: u64,
}

/// Emitted when the heartbeat interval is updated.
#[contractevent]
#[derive(Clone)]
pub struct HeartbeatIntervalChangedEvent {
    /// New heartbeat interval in seconds.
    pub value: u64,
}

/// Emitted when a previously inactive source submits a new heartbeat and becomes active.
///
/// Topics: `source`
#[contractevent]
#[derive(Clone)]
pub struct SourceActiveAgainEvent {
    /// Address of the source that resumed activity.
    #[topic]
    pub source: Address,
    /// Unix timestamp at which the source became active again.
    pub timestamp: u64,
}

/// Emitted when the contract is paused by the admin.
///
/// Topics: `admin`
#[contractevent]
#[derive(Clone)]
pub struct ContractPausedEvent {
    /// Address of the admin who paused the contract.
    #[topic]
    pub admin: Address,
}

/// Emitted when the contract is unpaused by the admin.
///
/// Topics: `admin`
#[contractevent]
#[derive(Clone)]
pub struct ContractUnpausedEvent {
    /// Address of the admin who unpaused the contract.
    #[topic]
    pub admin: Address,
}

/// Emitted when a stale price is detected during a read operation.
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct PriceStaleEvent {
    /// Address of the asset whose price was considered stale.
    #[topic]
    pub asset: Address,
    /// Ledger sequence number when the aggregate was last written (0 if unavailable).
    pub last_update_ledger: u32,
    /// Current ledger sequence number at the time of detection.
    pub current_ledger: u32,
}

/// Emitted when an admin proposes a new timelock-protected operation.
///
/// Topics: `proposed_by`
#[contractevent]
#[derive(Clone)]
pub struct OperationProposedEvent {
    /// Unique ID assigned to this pending operation.
    pub operation_id: u32,
    /// Numeric discriminant of the [`OperationType`](crate::types::OperationType).
    pub op_type: u32,
    /// Address of the admin who proposed this operation.
    #[topic]
    pub proposed_by: Address,
    /// Ledger sequence number when the operation was proposed.
    pub proposed_ledger: u32,
}

/// Emitted when a timelock-protected operation is successfully executed.
///
/// Topics: `executed_by`
#[contractevent]
#[derive(Clone)]
pub struct OperationExecutedEvent {
    /// ID of the operation that was executed.
    pub operation_id: u32,
    /// Numeric discriminant of the [`OperationType`](crate::types::OperationType).
    pub op_type: u32,
    /// Address of the admin who executed the operation.
    #[topic]
    pub executed_by: Address,
}

/// Emitted when a pending timelock operation is cancelled by the admin.
///
/// Topics: `cancelled_by`
#[contractevent]
#[derive(Clone)]
pub struct OperationCancelledEvent {
    /// ID of the operation that was cancelled.
    pub operation_id: u32,
    /// Numeric discriminant of the [`OperationType`](crate::types::OperationType).
    pub op_type: u32,
    /// Address of the admin who cancelled the operation.
    #[topic]
    pub cancelled_by: Address,
}

/// Emitted when the delay for a priority tier is changed by the admin.
///
/// Topics: `changed_by`
#[contractevent]
#[derive(Clone)]
pub struct PriorityDelayChangedEvent {
    /// Priority tier discriminant (0 = Urgent, 1 = Normal, 2 = LongTerm).
    pub priority: u32,
    /// New delay in ledgers for this tier.
    pub new_delay: u32,
    /// Admin address that changed the delay.
    #[topic]
    pub changed_by: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct PriceOverrideSetEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
    pub price: i128,
    pub reason: String,
    pub expiry_ledger: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct PriceOverrideRemovedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct PriceOverrideExpiredEvent {
    #[topic]
    pub asset: Address,
    pub expiry_ledger: u32,
    pub current_ledger: u32,
}

/// Emitted when the query rate limit is updated.
#[contractevent]
#[derive(Clone)]
pub struct QueryRateLimitChangedEvent {
    /// The new query rate limit value.
    pub value: u32,
}

/// Emitted when a rate limit is exceeded for an address.
///
/// Topics: `consumer`
#[contractevent]
#[derive(Clone)]
pub struct RateLimitExceededEvent {
    /// Address that exceeded the rate limit.
    #[topic]
    pub consumer: Address,
    /// Current count of operations.
    pub current_count: u32,
    /// The rate limit threshold.
    pub limit: u32,
}

/// Emitted when a subscription is created for a consumer.
///
/// Topics: `consumer`, `duration`
#[contractevent]
#[derive(Clone)]
pub struct SubscriptionCreatedEvent {
    /// Address of the consumer who created the subscription.
    #[topic]
    pub consumer: Address,
    /// Duration of the subscription in seconds.
    #[topic]
    pub duration: u64,
}

/// Emitted when a subscription is renewed by a consumer.
///
/// Topics: `consumer`
#[contractevent]
#[derive(Clone)]
pub struct SubscriptionRenewedEvent {
    /// Address of the consumer who renewed the subscription.
    #[topic]
    pub consumer: Address,
}

/// Emitted when a subscription expires for a consumer.
///
/// Topics: `consumer`
#[contractevent]
#[derive(Clone)]
pub struct SubscriptionExpiredEvent {
    /// Address of the consumer whose subscription expired.
    #[topic]
    pub consumer: Address,
}

// --- #294: Native token subscription payments ---

/// Emitted when a subscription payment is received in native token.
#[contractevent(topics = ["subscription_payment_received"])]
#[derive(Clone)]
pub struct SubscriptionPaymentReceivedEvent {
    #[topic]
    pub consumer: Address,
    pub amount: i128,
    pub ledger: u32,
}

/// Emitted when subscription fees are distributed to sources/relayers and treasury.
#[contractevent(topics = ["subscription_fees_distributed"])]
#[derive(Clone)]
pub struct SubscriptionFeesDistributedEvent {
    #[topic]
    pub consumer: Address,
    pub amount: i128,
    pub treasury_share: i128,
    pub sources_share: i128,
}

/// Emitted when a subscription payment is refunded.
#[contractevent(topics = ["subscription_payment_refunded"])]
#[derive(Clone)]
pub struct SubscriptionPaymentRefundedEvent {
    #[topic]
    pub consumer: Address,
    pub amount: i128,
    pub reason: String,
}

// --- #67: Per-asset resolution ---

/// Emitted when the per-asset resolution is set or cleared.
#[contractevent]
#[derive(Clone)]
pub struct AssetResolutionSetEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
    /// Resolution in seconds (0 = cleared, falls back to contract-wide).
    pub resolution: u32,
}

// --- #69: Periodic aggregation trigger ---

/// Emitted when trigger_aggregation is called and aggregation succeeds.
#[contractevent]
#[derive(Clone)]
pub struct AggregationTriggeredEvent {
    #[topic]
    pub asset: Address,
    pub price: i128,
    pub num_sources: u32,
    pub triggered_at_ledger: u32,
}

/// Emitted when the aggregation cooldown is updated.
#[contractevent]
#[derive(Clone)]
pub struct AggCooldownChangedEvent {
    pub cooldown_ledgers: u32,
}

// --- #70: Min submission interval ---

/// Emitted when the minimum submission interval is updated.
#[contractevent]
#[derive(Clone)]
pub struct SubmitIntervalChangedEvent {
    pub interval_ledgers: u32,
}

/// Emitted when a source is flagged as non-compliant for an asset.
#[contractevent]
#[derive(Clone)]
pub struct SourceNonCompliantEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub asset: Address,
    pub last_submission_ledger: u32,
    pub required_interval: u32,
}

// --- #68: Batch operations ---

/// Emitted when an admin proposes a new batch of operations.
#[contractevent]
#[derive(Clone)]
pub struct BatchProposedEvent {
    pub batch_id: u32,
    pub num_operations: u32,
    #[topic]
    pub proposed_by: Address,
    pub proposed_ledger: u32,
}

/// Emitted when a batch is successfully executed.
#[contractevent]
#[derive(Clone)]
pub struct BatchExecutedEvent {
    pub batch_id: u32,
    pub num_operations: u32,
    #[topic]
    pub executed_by: Address,
}

/// Emitted when a pending batch is cancelled.
#[contractevent]
#[derive(Clone)]
pub struct BatchCancelledEvent {
    pub batch_id: u32,
    #[topic]
    pub cancelled_by: Address,
}

// #65 reputation events
#[contractevent]
#[derive(Clone)]
pub struct SourceReputationUpdatedEvent {
    #[topic]
    pub source: Address,
    pub old_score: i128,
    pub new_score: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct ReputationDecayChangedEvent {
    pub value: u32,
}

// #66 phased removal events
#[contractevent]
#[derive(Clone)]
pub struct SourceMarkedForRemovalEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub admin: Address,
    pub eligible_at_ledger: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct SourceRemovalCancelledEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub admin: Address,
}

#[contractevent]
#[derive(Clone)]
pub struct RemovalCooldownChangedEvent {
    pub value: u32,
}

/// Emitted when the active aggregation method is changed by the admin.
///
/// Topics: `admin`
#[contractevent]
#[derive(Clone)]
pub struct AggregationMethodChangedEvent {
    /// Address of the admin who changed the method.
    #[topic]
    pub admin: Address,
    /// Previous aggregation method discriminant (0=Median,1=Mean,2=TrimmedMean,3=WeightedMedian).
    pub old_method: u32,
    /// New aggregation method discriminant.
    pub new_method: u32,
}

/// Emitted when an approved relayer successfully submits a price on behalf of a source.
///
/// Topics: `asset`, `source`, `relayer`
#[contractevent]
#[derive(Clone)]
pub struct PriceRelayedEvent {
    /// Address of the asset being priced.
    #[topic]
    pub asset: Address,
    /// Address of the oracle source whose price data was relayed.
    #[topic]
    pub source: Address,
    /// Address of the relayer that submitted the transaction.
    #[topic]
    pub relayer: Address,
    /// Raw price value scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp (seconds) of the price observation.
    pub timestamp: u64,
}

/// Publishes the relayer-fee-changed event.
///
/// Uses manual event publishing because `i128` fields in `#[contractevent]` may
/// trigger edge cases in some tooling.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `admin` - Address of the admin who set the new fee.
/// * `fee` - New fee per submission in stroops.
#[allow(deprecated)]
pub fn emit_relayer_fee_set(env: &soroban_sdk::Env, admin: Address, fee: i128) {
    let sym = soroban_sdk::symbol_short!("rfee");
    env.events().publish((sym, admin), (fee,));
}

// ─────────────────────────────────────────────────────────────────────────────
// Cross-reference oracle check events
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when a cross-reference check detects that our price deviates from a reference
/// oracle's price by more than the configured threshold.
///
/// Topics: `asset`, `ref_contract`
#[contractevent]
#[derive(Clone)]
pub struct CrossRefDeviationEvent {
    /// Address of the asset for which the deviation was detected.
    #[topic]
    pub asset: Address,
    /// Contract address of the reference oracle that reported the diverging price.
    #[topic]
    pub ref_contract: Address,
    /// Our current aggregated price for the asset.
    pub our_price: i128,
    /// Price reported by the reference oracle.
    pub ref_price: i128,
    /// Absolute deviation between the two prices in basis points (1 % = 100 bps).
    pub deviation_bps: u32,
    /// Configured deviation threshold (in basis points) that was exceeded.
    pub threshold_bps: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// #92/#93/#94: history cap, event spam protection, max aggregation sources
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when per-asset history is pruned beyond `max_history_per_asset` (issue #94).
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct HistoryPerAssetPrunedEvent {
    #[topic]
    pub asset: Address,
    /// Ledger removed from the history index.
    pub pruned_ledger: u32,
    /// Remaining entry count after pruning.
    pub remaining: u32,
}

/// Emitted when the `max_history_per_asset` limit is changed (issue #94).
#[contractevent]
#[derive(Clone)]
pub struct HistoryPerAssetChangedEvent {
    pub value: u32,
}

/// Emitted when the event-per-call cap is exceeded in a single invocation (issue #92).
/// The transaction still succeeds; this is a warning only.
///
/// Topics: `asset`
#[contractevent]
#[derive(Clone)]
pub struct EventLimitWarningEvent {
    #[topic]
    pub asset: Address,
    /// Number of events that would have been emitted.
    pub event_count: u32,
    /// Configured cap that was exceeded.
    pub max_events: u32,
}

/// Emitted when the `max_events_per_call` limit is changed (issue #92).
#[contractevent]
#[derive(Clone)]
pub struct EventsPerCallChangedEvent {
    pub value: u32,
}

/// Emitted when the `max_aggregation_sources` limit is changed (issue #93).
#[contractevent]
#[derive(Clone)]
pub struct MaxAggSourcesChangedEvent {
    pub value: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// #112: Storage migration events
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when a storage migration is resumed from a previously saved cursor.
#[contractevent]
#[derive(Clone)]
pub struct MigrationResumedEvent {
    #[topic]
    pub admin: Address,
    pub cursor: u32,
}

/// Emitted when a new storage migration begins.
#[contractevent]
#[derive(Clone)]
pub struct MigrationStartedEvent {
    #[topic]
    pub admin: Address,
    pub from_version: u32,
    pub to_version: u32,
    pub started_ledger: u32,
}

/// Emitted when a storage migration finishes processing all items.
#[contractevent]
#[derive(Clone)]
pub struct MigrationCompletedEvent {
    #[topic]
    pub admin: Address,
    pub from_version: u32,
    pub to_version: u32,
    pub items_processed: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Misc admin config events
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when historical-price interpolation is enabled or disabled.
#[contractevent]
#[derive(Clone)]
pub struct InterpolationChangedEvent {
    pub enabled: bool,
}

/// Emitted when the maximum number of registered oracle sources is changed.
#[contractevent]
#[derive(Clone)]
pub struct MaxSourcesChangedEvent {
    pub value: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// #210: Progressive Disqualification Events
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when a source accumulates enough demerits to trigger a warning.
#[contractevent]
#[derive(Clone)]
pub struct SourceWarningEvent {
    #[topic]
    pub source: Address,
    pub demerits: u32,
}

/// Emitted when a source accumulates enough demerits to be placed on probation.
#[contractevent]
#[derive(Clone)]
pub struct SourceProbationEvent {
    #[topic]
    pub source: Address,
    pub demerits: u32,
}

/// Emitted when a source accumulates enough demerits to be disqualified.
#[contractevent]
#[derive(Clone)]
pub struct SourceDisqualifiedEvent {
    #[topic]
    pub source: Address,
    pub demerits: u32,
    pub status_updated_ledger: u32,
}

/// Emitted when a source's demerits and disqualification status are reset by the admin.
#[contractevent]
#[derive(Clone)]
pub struct SourceDemeritsResetEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub admin: Address,
}

/// Emitted when the global demerit configuration is changed.
#[contractevent]
#[derive(Clone)]
pub struct DemeritConfigChangedEvent {
    #[topic]
    pub admin: Address,
    pub warning_threshold: u32,
    pub probation_threshold: u32,
    pub disqualified_threshold: u32,
    pub cooldown_ledgers: u32,
}

/// Emitted when an invalid price submission is recorded against a source.
#[contractevent]
#[derive(Clone)]
pub struct InvalidSubmissionEvent {
    #[topic]
    pub source: Address,
    pub demerits: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// #207: Multi-sig Source Governance Events
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when source governance config is updated.
#[contractevent]
#[derive(Clone)]
pub struct SourceGovConfigChangedEvent {
    #[topic]
    pub admin: Address,
    pub threshold: u32,
    pub approvers_count: u32,
}

/// Emitted when a new source proposal is proposed.
#[contractevent]
#[derive(Clone)]
pub struct SourceProposalCreatedEvent {
    #[topic]
    pub proposal_id: u32,
    #[topic]
    pub proposer: Address,
    #[topic]
    pub source: Address,
    pub name: String,
}

/// Emitted when an approver approves a source proposal.
#[contractevent]
#[derive(Clone)]
pub struct SourceProposalApprovedEvent {
    #[topic]
    pub proposal_id: u32,
    #[topic]
    pub approver: Address,
}

/// Emitted when a source proposal is executed (threshold met).
#[contractevent]
#[derive(Clone)]
pub struct SourceProposalExecutedEvent {
    #[topic]
    pub proposal_id: u32,
    #[topic]
    pub source: Address,
}

// ─────────────────────────────────────────────────────────────────────────────
// #208: Source Geolocation Events
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when geolocation metadata for a source is updated.
#[contractevent]
#[derive(Clone)]
pub struct SourceGeoUpdatedEvent {
    #[topic]
    pub source: Address,
    pub region: String,
    pub provider: String,
    pub jurisdiction: String,
}

// ─────────────────────────────────────────────────────────────────────────────
// #209: Source Heartbeat Liveness Bond Events
// ─────────────────────────────────────────────────────────────────────────────

/// Emitted when the required source bond amount is changed.
#[contractevent]
#[derive(Clone)]
pub struct SourceBondConfigChangedEvent {
    #[topic]
    pub admin: Address,
    pub amount: i128,
}

/// Emitted when a source deposits its liveness bond.
#[contractevent]
#[derive(Clone)]
pub struct SourceBondDepositedEvent {
    #[topic]
    pub source: Address,
    pub amount: i128,
}

/// Emitted when a source bond is forfeited.
#[contractevent]
#[derive(Clone)]
pub struct SourceBondForfeitedEvent {
    #[topic]
    pub source: Address,
    pub amount: i128,
}

/// Emitted when a source bond is returned.
#[contractevent]
#[derive(Clone)]
pub struct SourceBondReturnedEvent {
    #[topic]
    pub source: Address,
    pub amount: i128,
}

// =============================================================================
// Missing events for feature modules
// =============================================================================

/// Emitted when asset metadata is updated.
#[contractevent]
#[derive(Clone)]
pub struct AssetMetadataUpdatedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
    pub name: String,
    pub symbol: String,
    pub decimals: Option<u32>,
    pub logo_uri: Option<String>,
}

/// Circuit breaker event entry (used as a struct in some older modules).
/// NOTE: This is a struct, not an event, kept here for backward compatibility.
#[derive(Clone)]
#[soroban_sdk::contracttype]
pub struct CircuitBreakerEventEntry {
    pub asset: Address,
    pub previous_price: i128,
    pub candidate_price: i128,
    pub change_bps: u32,
    pub max_change_bps: u32,
    pub ledger: u32,
    pub timestamp: u64,
}

/// Emitted when a price is submitted with a deadline (#202).
#[contractevent]
#[derive(Clone)]
pub struct PriceSubmitDeadlineEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub deadline_ledger: u32,
    pub current_ledger: u32,
    pub rebate_amount: i128,
}

/// Emitted when a submission rebate is distributed (#202).
#[contractevent]
#[derive(Clone)]
pub struct RebateDistributedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub asset: Address,
    pub rebate_amount: i128,
}

/// Emitted when an exotic asset pricing config is set (#177).
#[contractevent]
#[derive(Clone)]
pub struct ExoticAssetConfigSetEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
}

/// Emitted when the fee market minimum priority fee is changed (#176).
#[contractevent]
#[derive(Clone)]
pub struct FmMinPriorityFeeEvent {
    pub value: u128,
}

/// Emitted when the fee distribution ratio is changed (#176).
#[contractevent]
#[derive(Clone)]
pub struct FmFeeRatioChangedEvent {
    pub ratio_bps: u32,
}

/// Emitted when a fee market submission is enqueued (#176).
#[contractevent]
#[derive(Clone)]
pub struct FmSubmissionEnqueuedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub asset: Address,
    pub priority_fee: u128,
    pub queue_depth: u32,
}

/// Emitted when a fee market submission is processed (#176).
#[contractevent]
#[derive(Clone)]
pub struct FmSubmissionProcessedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub asset: Address,
    pub price: i128,
    pub priority_fee: u128,
    pub source_share: u128,
    pub treasury_share: u128,
}

/// Emitted when multi-sig governors list is updated (#178).
#[contractevent]
#[derive(Clone)]
pub struct MsGovernorsUpdatedEvent {
    #[topic]
    pub admin: Address,
    pub governor_count: u32,
    pub required_approvals: u32,
}

/// Emitted when a multi-sig operation is proposed (#178).
#[contractevent]
#[derive(Clone)]
pub struct MsOperationProposedEvent {
    pub op_id: u32,
    pub op_type: u32,
    #[topic]
    pub proposed_by: Address,
    pub proposed_ledger: u32,
}

/// Emitted when multi-sig quorum is reached (#178).
#[contractevent]
#[derive(Clone)]
pub struct MsQuorumReachedEvent {
    pub op_id: u32,
    pub approval_count: u32,
}

/// Emitted when a governor approves a multi-sig operation (#178).
#[contractevent]
#[derive(Clone)]
pub struct MsOperationApprovedEvent {
    pub op_id: u32,
    #[topic]
    pub approver: Address,
}

/// Emitted when a multi-sig operation is retracted before execution (#178).
#[contractevent]
#[derive(Clone)]
pub struct MsOperationRetractedEvent {
    pub op_id: u32,
    #[topic]
    pub retracted_by: Address,
}

/// Emitted when a multi-sig operation is executed (#178).
#[contractevent]
#[derive(Clone)]
pub struct MsOperationExecutedEvent {
    pub op_id: u32,
    pub op_type: u32,
    #[topic]
    pub executed_by: Address,
}

/// Emitted when a multi-sig operation is cancelled (#178).
#[contractevent]
#[derive(Clone)]
pub struct MsOperationCancelledEvent {
    pub op_id: u32,
    #[topic]
    pub cancelled_by: Address,
}

/// Emitted when a source fee credit is recorded.
#[contractevent]
#[derive(Clone)]
pub struct SourceFeeCreditedEvent {
    #[topic]
    pub source: Address,
    pub amount: i128,
}

/// Emitted when a source withdraws accumulated fees.
#[contractevent]
#[derive(Clone)]
pub struct SourceFeesWithdrawnEvent {
    #[topic]
    pub source: Address,
    pub amount: i128,
}

/// Emitted when a ZK verifying key is set (#175).
#[contractevent]
#[derive(Clone)]
pub struct ZkVerifyingKeySetEvent {
    #[topic]
    pub admin: Address,
    pub set_at_ledger: u32,
}

/// Emitted when a ZK-verified price is submitted (#175).
#[contractevent]
#[derive(Clone)]
pub struct ZkPriceSubmittedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub price: i128,
    pub timestamp: u64,
    pub verified_at_ledger: u32,
}

/// Emitted when a challenge is submitted (#235).
#[contractevent]
#[derive(Clone)]
pub struct ChallengePricedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub challenger: Address,
    pub challenge_id: u32,
    pub expected_price: i128,
    pub challenged_ledger: u32,
}

/// Emitted when a challenge is resolved (#235).
#[contractevent]
#[derive(Clone)]
pub struct ChallengeResolvedEvent {
    #[topic]
    pub asset: Address,
    pub challenge_id: u32,
    pub is_valid: bool,
    pub reward_amount: i128,
    pub resolved_by: Address,
}

/// Emitted when challenger rewards are claimed (#235).
#[contractevent]
#[derive(Clone)]
pub struct RewardsClaimedEvent {
    #[topic]
    pub claimer: Address,
    pub amount: i128,
}

/// Emitted when a source rotation schedule is set (#206).
#[contractevent]
#[derive(Clone)]
pub struct SourceRotationSetEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
    pub rotation_interval: u32,
    pub overlap_period: u32,
    pub num_sources: u32,
}

/// Emitted when sources are rotated for an asset (#206).
#[contractevent]
#[derive(Clone)]
pub struct SourcesRotatedEvent {
    #[topic]
    pub asset: Address,
    pub rotated_at_ledger: u32,
    pub new_active_count: u32,
    pub next_rotation_ledger: u32,
}

/// Emitted when an admin audit entry is appended (#239).
#[contractevent]
#[derive(Clone)]
pub struct AdminAuditEntryAppendedEvent {
    #[topic]
    pub admin: Address,
    pub entry_id: u32,
    pub action: Symbol,
    pub timestamp: u64,
    pub ledger: u32,
}

/// Emitted when a role is delegated (#241).
#[contractevent]
#[derive(Clone)]
pub struct RoleDelegatedEvent {
    #[topic]
    pub delegator: Address,
    #[topic]
    pub delegatee: Address,
    pub role: u32,
}

/// Emitted when a role is revoked (#241).
#[contractevent]
#[derive(Clone)]
pub struct RoleRevokedEvent {
    #[topic]
    pub delegator: Address,
    #[topic]
    pub delegatee: Address,
    pub role: u32,
}

/// Emitted when an emergency pause is triggered (#240).
#[contractevent]
#[derive(Clone)]
pub struct EmergencyPausedEvent {
    #[topic]
    pub admin: Address,
    pub auto_unpause_ledger: u32,
    pub reason: String,
    pub initiated_by: Address,
}

/// Emitted when an emergency pause is lifted (#240).
#[contractevent]
#[derive(Clone)]
pub struct EmergencyUnpausedEvent {
    #[topic]
    pub admin: Address,
    pub reason: String,
    pub cancelled_by: Address,
}

/// Emitted when an emergency pause duration is extended (#240).
#[contractevent]
#[derive(Clone)]
pub struct EmergencyPauseExtendedEvent {
    #[topic]
    pub admin: Address,
    pub new_unpause_ledger: u32,
    pub reason: String,
    pub extended_by: Address,
}

/// Emitted when an asset TTL extension is performed (#203).
#[contractevent]
#[derive(Clone)]
pub struct AssetTtlExtendedEvent {
    #[topic]
    pub asset: Address,
    pub num_extended: u32,
    pub current_ledger: u32,
}

/// Emitted when the rate limit tier is changed.
#[contractevent]
#[derive(Clone)]
pub struct RateLimitTierChangedEvent {
    #[topic]
    pub consumer: Address,
    pub new_tier: u32,
}

// Emitted when an invalid submission is recorded against a source (re-export from events).
// Already defined elsewhere, but needed here as well.
// Note: InvalidSubmissionEvent is already defined above; this is the canonical copy.

// --- #217: Configurable optimistic-oracle parameters ---

/// Emitted when the admin updates the optimistic proposal dispute window.
#[contractevent]
#[derive(Clone)]
pub struct DisputeWindowChangedEvent {
    #[topic]
    pub admin: Address,
    pub dispute_window_ledgers: u32,
}

/// Emitted when the admin updates the optimistic proposal minimum bond.
#[contractevent]
#[derive(Clone)]
pub struct OptimisticBondChangedEvent {
    #[topic]
    pub admin: Address,
    pub min_bond: i128,
}

// --- #216: Off-chain signature-verified price submission ---

/// Emitted when a source registers (or rotates) its Ed25519 submission key.
#[contractevent]
#[derive(Clone)]
pub struct SubmissionKeyRegisteredEvent {
    #[topic]
    pub source: Address,
    pub public_key: BytesN<32>,
}

/// Emitted when a price is accepted via a pre-signed Ed25519 proof.
#[contractevent]
#[derive(Clone)]
pub struct PriceSubmittedWithProofEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub price: i128,
    pub timestamp: u64,
    pub nonce: u64,
}

// --- #218: Configurable aggregation triggers ---

/// Emitted when the admin (re)configures a per-asset aggregation trigger.
///
/// `trigger_type`: `0` = time interval (seconds), `1` = submission threshold
/// (count), `2` = deviation threshold (basis points).
#[contractevent]
#[derive(Clone)]
pub struct TriggerConfigChangedEvent {
    #[topic]
    pub asset: Address,
    pub trigger_type: u32,
    pub value: i128,
}

/// Emitted when a configured trigger fires and aggregation is re-run.
///
/// `trigger_type` uses the same encoding as [`TriggerConfigChangedEvent`].
#[contractevent]
#[derive(Clone)]
pub struct AutoTriggerFiredEvent {
    #[topic]
    pub asset: Address,
    pub trigger_type: u32,
}

/// Emitted when an admin freezes an asset's price during a market emergency (#223).
#[contractevent]
#[derive(Clone)]
pub struct PriceFrozenEvent {
    #[topic]
    pub asset: Address,
    pub reason: String,
    pub price: i128,
    pub frozen_at_ledger: u32,
}

/// Emitted when an admin unfreezes a previously frozen asset (#223).
#[contractevent]
#[derive(Clone)]
pub struct PriceUnfrozenEvent {
    #[topic]
    pub asset: Address,
    pub unfrozen_at_ledger: u32,
}

/// Emitted when an admin registers a notification preference for an event type (#243).
#[contractevent]
#[derive(Clone)]
pub struct NotifPrefSetEvent {
    #[topic]
    pub event_type: u32,
    pub channel: String,
    pub target: String,
}

/// Emitted when an admin clears all notification preferences for an event type (#243).
#[contractevent]
#[derive(Clone)]
pub struct NotifPrefsClearedEvent {
    #[topic]
    pub event_type: u32,
}

/// Emitted when a core configuration snapshot is taken before a parameter change.
#[contractevent]
#[derive(Clone)]
pub struct ConfigSnapshotTakenEvent {
    /// Address of the admin that triggered the snapshot.
    #[topic]
    pub admin: Address,
    /// Newly assigned snapshot version.
    pub version: u32,
    /// Ledger sequence when the snapshot was stored.
    pub ledger: u32,
}

/// Emitted when an admin rolls configuration back to a previous snapshot.
#[contractevent]
#[derive(Clone)]
pub struct ConfigRolledBackEvent {
    /// Address of the admin that performed the rollback.
    #[topic]
    pub admin: Address,
    /// Version that was restored as live config.
    pub restored_version: u32,
    /// Version created by snapshotting the pre-rollback live config.
    pub saved_version: u32,
}

// ---- Operation expiry events ----

/// Emitted when a new pending operation is enqueued.
#[contractevent]
#[derive(Clone)]
pub struct OperationQueuedEvent {
    #[topic]
    pub operation_id: u64,
    pub expires_at_ledger: u32,
}

/// Emitted when a pending operation is expired (either on-demand or via maintenance sweep).
#[contractevent]
#[derive(Clone)]
pub struct OperationExpiredEvent {
    #[topic]
    pub operation_id: u64,
    pub expired_at_ledger: u32,
}

/// Emitted when the default operation expiry window is changed.
#[contractevent]
#[derive(Clone)]
pub struct ExpiryWindowChangedEvent {
    pub ledgers: u32,
}

// ---- Template lifecycle events ----

/// Emitted when a new template is created.
#[contractevent]
#[derive(Clone)]
pub struct TemplateCreatedEvent {
    #[topic]
    pub name: Symbol,
    pub num_steps: u32,
}

/// Emitted when a template is applied (instantiated into pending operations).
#[contractevent]
#[derive(Clone)]
pub struct TemplateAppliedEvent {
    #[topic]
    pub name: Symbol,
    /// Number of pending operations created from this template application.
    pub operations_created: u32,
}

/// Emitted when a template is removed.
#[contractevent]
#[derive(Clone)]
pub struct TemplateRemovedEvent {
    #[topic]
    pub name: Symbol,
}

// =============================================================================
// #283 — Stellar DID Integration Events
// =============================================================================

/// Emitted when a DID document is registered.
#[contractevent]
#[derive(Clone)]
pub struct DidRegisteredEvent {
    #[topic]
    pub did: Address,
    #[topic]
    pub admin: Address,
}

/// Emitted when a DID document is verified.
#[contractevent]
#[derive(Clone)]
pub struct DidVerifiedEvent {
    #[topic]
    pub did: Address,
    pub verified: bool,
    pub verifier: Address,
}

/// Emitted when an oracle source is linked to a DID.
#[contractevent]
#[derive(Clone)]
pub struct SourceDidLinkedEvent {
    #[topic]
    pub source: Address,
    #[topic]
    pub did: Address,
    pub verified: bool,
}

// =============================================================================
// #282 — Bridge Oracle Events
// =============================================================================

/// Emitted when a bridge oracle is registered.
#[contractevent]
#[derive(Clone)]
pub struct BridgeOracleRegisteredEvent {
    #[topic]
    pub source_asset: Address,
    #[topic]
    pub target_asset: Address,
    pub oracle_contract: Address,
}

/// Emitted when a bridged price is submitted.
#[contractevent]
#[derive(Clone)]
pub struct BridgePriceSubmittedEvent {
    #[topic]
    pub asset: Address,
    pub price: i128,
    pub timestamp: u64,
    pub decimals: u32,
}

// =============================================================================
// #285 — Ecosystem Metadata Events
// =============================================================================

/// Emitted when feed metadata is registered.
#[contractevent]
#[derive(Clone)]
pub struct FeedMetadataRegisteredEvent {
    #[topic]
    pub asset: Address,
    pub symbol: String,
    pub description: String,
}

/// Emitted when feed metadata is updated.
#[contractevent]
#[derive(Clone)]
pub struct FeedMetadataUpdatedEvent {
    #[topic]
    pub asset: Address,
    pub symbol: String,
    pub updated_at: u64,
}

// =============================================================================
// Canonical Cross-Chain Asset Registry Events
// =============================================================================

/// Emitted when a new foreign-chain asset mapping is registered.
#[contractevent]
#[derive(Clone)]
pub struct ForeignAssetMappedEvent {
    #[topic]
    pub stellar_asset: Address,
    pub chain: String,
    pub foreign_address: BytesN<32>,
    pub decimals: u32,
}

/// Emitted when an existing foreign-chain asset mapping is updated
/// (decimals and/or enabled flag).
#[contractevent(topics = ["foreign_asset_mapping_updated"])]
#[derive(Clone)]
pub struct ForeignAssetMappingUpdatedEvent {
    #[topic]
    pub stellar_asset: Address,
    pub chain: String,
    pub foreign_address: BytesN<32>,
    pub decimals: u32,
    pub enabled: bool,
}

/// Emitted when a foreign-chain asset mapping is removed.
#[contractevent(topics = ["foreign_asset_mapping_removed"])]
#[derive(Clone)]
pub struct ForeignAssetMappingRemovedEvent {
    #[topic]
    pub stellar_asset: Address,
    pub chain: String,
    pub foreign_address: BytesN<32>,
}

// =============================================================================
// Axelar GMP Integration Events
// =============================================================================

/// Emitted when the trusted Axelar Gateway address is (re)configured.
#[contractevent]
#[derive(Clone)]
pub struct AxelarGatewaySetEvent {
    #[topic]
    pub gateway: Address,
}

/// Emitted when a trusted Axelar GMP source is registered.
#[contractevent]
#[derive(Clone)]
pub struct AxelarTrustedSourceSetEvent {
    #[topic]
    pub bridge_source: Address,
    pub source_chain: String,
    pub source_address: String,
}

/// Emitted when a price is applied from a verified Axelar GMP message.
#[contractevent]
#[derive(Clone)]
pub struct AxelarMessageExecutedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub bridge_source: Address,
    pub command_id: BytesN<32>,
    pub source_chain: String,
    pub source_address: String,
    pub price: i128,
}

// =============================================================================
// LayerZero Integration Events
// =============================================================================

/// Emitted when the trusted LayerZero Endpoint address is (re)configured.
#[contractevent]
#[derive(Clone)]
pub struct LzEndpointSetEvent {
    #[topic]
    pub endpoint: Address,
}

/// Emitted when a trusted LayerZero remote pathway is registered.
#[contractevent]
#[derive(Clone)]
pub struct LzTrustedRemoteSetEvent {
    #[topic]
    pub bridge_source: Address,
    pub src_eid: u32,
    pub sender: BytesN<32>,
}

/// Emitted when a price is applied from a verified LayerZero message.
#[contractevent]
#[derive(Clone)]
pub struct LzMessageReceivedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub bridge_source: Address,
    pub src_eid: u32,
    pub sender: BytesN<32>,
    pub nonce: u64,
    pub guid: BytesN<32>,
    pub price: i128,
}

// =============================================================================
// Restored events for feature modules (health, finality, commit-reveal,
// fee market, wormhole relay, subscription payments, source severity)
// =============================================================================

/// Emitted when a source's health status changes.
#[contractevent]
#[derive(Clone)]
pub struct SourceHealthChangedEvent {
    #[topic]
    pub source: Address,
    pub old_status: u32,
    pub new_status: u32,
    pub missed_heartbeats: u32,
}

/// Emitted when a source is automatically removed after prolonged inactivity.
#[contractevent]
#[derive(Clone)]
pub struct SourceAutoRemovedEvent {
    #[topic]
    pub source: Address,
    pub inactive_since_ledger: u32,
    pub removed_at_ledger: u32,
    pub missed_heartbeats: u32,
}

/// Emitted when the max-inactive-ledgers threshold is changed.
#[contractevent]
#[derive(Clone)]
pub struct InactiveLedgersChangedEvent {
    pub value: u32,
}

/// Emitted when the heartbeat window size is changed.
#[contractevent]
#[derive(Clone)]
pub struct HeartbeatWindowChangedEvent {
    pub value: u32,
}

/// Emitted when the fee market distribution ratio is changed.
#[contractevent]
#[derive(Clone)]
pub struct FmFeeDistRatioChangedEvent {
    pub value: u32,
}

/// Emitted when the finality window (ledgers) is changed.
#[contractevent]
#[derive(Clone)]
pub struct FinalityLedgersChangedEvent {
    pub value: u32,
}

/// Emitted when the commit-reveal commit window is changed.
#[contractevent]
#[derive(Clone)]
pub struct CommitWindowChangedEvent {
    pub value: u32,
}

/// Emitted when the commit-reveal reveal window is changed.
#[contractevent]
#[derive(Clone)]
pub struct RevealWindowChangedEvent {
    pub value: u32,
}

/// Emitted when a price enters the finality pending window.
#[contractevent]
#[derive(Clone)]
pub struct PricePendingFinalityEvent {
    #[topic]
    pub asset: Address,
    pub price: i128,
    pub committed_ledger: u32,
    pub finality_ledger: u32,
}

/// Emitted when a pending price is finalized.
#[contractevent]
#[derive(Clone)]
pub struct PriceFinalizedEvent {
    #[topic]
    pub asset: Address,
    pub price: i128,
    pub committed_ledger: u32,
    pub finalized_ledger: u32,
    pub num_sources: u32,
}

/// Emitted when a pending price is retracted (reorg or admin action).
#[contractevent]
#[derive(Clone)]
pub struct PriceRetractedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub admin: Address,
    pub committed_ledger: u32,
    pub retracted_at_ledger: u32,
}

/// Emitted when a reorg is detected for an asset.
#[contractevent]
#[derive(Clone)]
pub struct ReorgDetectedEvent {
    #[topic]
    pub asset: Address,
    pub detected_at_ledger: u32,
    pub suspect_ledger: u32,
}

/// Emitted when a price is committed in the commit-reveal scheme.
#[contractevent]
#[derive(Clone)]
pub struct PriceCommittedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub round_ledger: u32,
    pub committed_at_ledger: u32,
}

/// Emitted when a committed price is revealed.
#[contractevent]
#[derive(Clone)]
pub struct PriceRevealedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub price: i128,
    pub round_ledger: u32,
    pub revealed_at_ledger: u32,
}

/// Emitted when a source's stake is slashed for failing to reveal.
#[contractevent]
#[derive(Clone)]
pub struct SourceSlashedEvent {
    #[topic]
    pub source: Address,
    pub slash_amount: i128,
    pub remaining_stake: i128,
    pub slash_percent: u32,
}

/// Emitted when the Wormhole guardian set is (re)configured.
#[contractevent]
#[derive(Clone)]
pub struct WormholeGuardianSetEvent {
    pub set_index: u32,
    pub guardian_count: u32,
    pub quorum: u32,
}

/// Emitted when a price is applied from a verified Wormhole VAA.
#[contractevent]
#[derive(Clone)]
pub struct WormholePriceRelayedEvent {
    #[topic]
    pub asset: Address,
    pub emitter_chain: u32,
    pub price: i128,
    pub sequence: u64,
}

// =============================================================================
// Consumer authorization, price-update subscriptions, scheduling, verification
// (restored events + emit helpers for the feature modules wired in from disk)
// =============================================================================

/// Emitted when a consumer is explicitly authorized (#304).
#[contractevent]
#[derive(Clone)]
pub struct ConsumerAuthorizedEvent {
    #[topic]
    pub consumer: Address,
    pub admin: Address,
}

/// Emitted when a consumer is explicitly deauthorized (#304).
#[contractevent]
#[derive(Clone)]
pub struct ConsumerDeauthorizedEvent {
    #[topic]
    pub consumer: Address,
    pub admin: Address,
}

/// Emitted when the global consumer access mode is changed (#304).
#[contractevent(topics = ["consumer_access_mode_changed"])]
#[derive(Clone)]
pub struct ConsumerAccessModeChangedEvent {
    #[topic]
    pub admin: Address,
    pub new_mode: crate::types::ConsumerAccessMode,
}

/// Emitted when a consumer subscribes to price updates for an asset (#305).
#[contractevent]
#[derive(Clone)]
pub struct PriceUpdateSubscribedEvent {
    #[topic]
    pub consumer: Address,
    #[topic]
    pub asset: Address,
}

/// Emitted when a consumer unsubscribes from price updates for an asset (#305).
#[contractevent]
#[derive(Clone)]
pub struct PriceUpdateUnsubscribedEvent {
    #[topic]
    pub consumer: Address,
    #[topic]
    pub asset: Address,
}

/// Emitted when the subscription payment token contract is set (#294).
#[contractevent]
#[derive(Clone)]
pub struct SubscriptionTokenSetEvent {
    #[topic]
    pub admin: Address,
    pub token: Address,
}

/// Emitted when a consumer cancels a subscription (#294).
#[contractevent]
#[derive(Clone)]
pub struct SubscriptionCancelledEvent {
    #[topic]
    pub consumer: Address,
    pub refund_amount: i128,
}

/// Emitted when a submission schedule is removed (#290).
#[contractevent]
#[derive(Clone)]
pub struct ScheduleRemovedEvent {
    #[topic]
    pub source: Address,
    pub asset: Address,
}

/// Emitted when a pair of prices is checked for deviation (#226).
#[contractevent]
#[derive(Clone)]
pub struct PriceDeviationVerifiedEvent {
    pub price_a: i128,
    pub price_b: i128,
    pub deviation_bps: u32,
    pub max_deviation_bps: u32,
    pub within_tolerance: bool,
}

/// Emitted when an external reference price is compared against the aggregate (#226).
#[contractevent]
#[derive(Clone)]
pub struct CrossOracleDeviationEvent {
    #[topic]
    pub asset: Address,
    pub oracle_price: i128,
    pub reference_price: i128,
    pub deviation_bps: u32,
    pub max_deviation_bps: u32,
    pub within_tolerance: bool,
}

/// Emitted when a submission schedule is registered for a (source, asset) pair (#290).
pub fn emit_schedule_registered(
    env: &soroban_sdk::Env,
    source: Address,
    asset: Address,
    kind: u64,
    interval: u64,
    deadline_multiplier: u32,
) {
    let sym = soroban_sdk::symbol_short!("sched_reg");
    env.events()
        .publish((sym, source, asset), (kind, interval, deadline_multiplier));
}

/// Emitted when a source misses its submission-schedule deadline (#290).
pub fn emit_schedule_violation(
    env: &soroban_sdk::Env,
    source: Address,
    asset: Address,
    expected_interval: u64,
    actual_gap: u64,
    kind_code: u32,
) {
    let sym = soroban_sdk::symbol_short!("sviol");
    env.events().publish(
        (sym, source, asset),
        (expected_interval, actual_gap, kind_code),
    );
}

/// Emitted after a price-freshness check completes (#226).
pub fn emit_price_freshness_verified(
    env: &soroban_sdk::Env,
    asset: Address,
    is_fresh: bool,
    max_age: u64,
    price_age: u64,
) {
    let sym = soroban_sdk::symbol_short!("fresh_ver");
    env.events()
        .publish((sym, asset), (is_fresh, max_age, price_age));
}

/// Emitted when severity thresholds are (re)configured.
#[contractevent]
#[derive(Clone)]
pub struct SeverityThresholdsSetEvent {
    pub is_asset_override: bool,
    pub warning_bps: u32,
    pub critical_bps: u32,
    pub emergency_bps: u32,
}

/// Emitted when a price-movement alert fires at `Warning` severity or above.
#[contractevent]
#[derive(Clone)]
pub struct SeverityAlertEvent {
    #[topic]
    pub asset: Address,
    pub severity: u32,
    pub channel: u32,
    pub movement_bps: u32,
    pub ledger: u32,
}

/// Emitted when a price deviation is detected against a reference oracle.
#[contractevent]
#[derive(Clone)]
pub struct PriceDeviationAlertEvent {
    #[topic]
    pub asset: Address,
    pub our_price: i128,
    pub reference_price: i128,
    pub deviation_bps: u32,
    pub ledger: u32,
}

// =============================================================================
// Restored events for alerts, correlation, recovery, relayer network, staking
// =============================================================================

/// Emitted when a consumer subscribes to price-deviation alerts.
#[contractevent]
#[derive(Clone)]
pub struct AlertSubscribedEvent {
    #[topic]
    pub consumer: Address,
    #[topic]
    pub asset: Address,
    pub threshold_bps: u32,
    pub ttl_ledgers: u32,
}

/// Emitted when an alert subscription expires.
#[contractevent]
#[derive(Clone)]
pub struct AlertSubscriptionExpiredEvent {
    #[topic]
    pub consumer: Address,
    #[topic]
    pub asset: Address,
    pub expired_ledger: u32,
}

/// Emitted when a price-deviation alert triggers.
#[contractevent]
#[derive(Clone)]
pub struct AlertTriggeredEvent {
    #[topic]
    pub consumer: Address,
    #[topic]
    pub asset: Address,
    pub old_price: i128,
    pub new_price: i128,
    pub movement_bps: u32,
    pub threshold_bps: u32,
}

/// Emitted when a correlation band is (re)configured.
#[contractevent]
#[derive(Clone)]
pub struct CorrelationBandSetEvent {
    #[topic]
    pub base_asset: Address,
    #[topic]
    pub quote_asset: Address,
    pub min_ratio: u128,
    pub max_ratio: u128,
    pub enabled: bool,
}

/// Emitted when a submission violates a registered correlation band.
#[contractevent]
#[derive(Clone)]
pub struct CorrelationViolationEvent {
    #[topic]
    pub base_asset: Address,
    #[topic]
    pub quote_asset: Address,
    #[topic]
    pub source: Address,
    pub submitted_price: i128,
    pub counterpart_price: i128,
    pub ratio: u128,
    pub min_ratio: u128,
    pub max_ratio: u128,
}

/// Emitted when a source's price submission is flagged by correlation checks.
#[contractevent]
#[derive(Clone)]
pub struct CorrelationPriceFlaggedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    pub flagged_price: i128,
}

/// Emitted when a recovery guardian set is configured (#245).
#[contractevent]
#[derive(Clone)]
pub struct GuardiansSetEvent {
    #[topic]
    pub admin: Address,
    pub guardian_count: u32,
    pub threshold: u32,
}

/// Emitted when a recovery reaches guardian threshold and becomes ready (#245).
#[contractevent]
#[derive(Clone)]
pub struct RecoveryReadyEvent {
    #[topic]
    pub new_admin: Address,
    pub ready_ledger: u32,
    pub execute_after_ledger: u32,
}

/// Emitted when a guardian approves a pending recovery (#245).
#[contractevent]
#[derive(Clone)]
pub struct RecoveryApprovedEvent {
    #[topic]
    pub guardian: Address,
    pub new_admin: Address,
    pub approval_count: u32,
    pub threshold: u32,
}

/// Emitted when a pending recovery is cancelled (#245).
#[contractevent]
#[derive(Clone)]
pub struct RecoveryCancelledEvent {
    #[topic]
    pub admin: Address,
    pub new_admin: Address,
}

/// Emitted when a recovery executes and installs a new admin (#245).
#[contractevent]
#[derive(Clone)]
pub struct RecoveryExecutedEvent {
    #[topic]
    pub old_admin: Address,
    pub new_admin: Address,
}

/// Emitted when a relayer is approved (#264).
#[contractevent]
#[derive(Clone)]
pub struct RelayerAddedEvent {
    #[topic]
    pub relayer: Address,
    pub admin: Address,
    pub name: String,
}

/// Emitted when a relayer is removed (#264).
#[contractevent]
#[derive(Clone)]
pub struct RelayerRemovedEvent {
    #[topic]
    pub relayer: Address,
    pub admin: Address,
}

/// Emitted when a batch of prices is relayed (#264).
#[contractevent]
#[derive(Clone)]
pub struct BatchPriceRelayedEvent {
    #[topic]
    pub relayer: Address,
    pub submission_count: u32,
    pub total_priority_fee: i128,
}

/// Emitted when the required relayer bond amount is changed.
/// Uses manual publishing because the #[contractevent] macro panics at this position
/// in the file (likely soroban-sdk proc-macro event-count limit).
#[allow(deprecated)]
pub fn emit_relayer_bond_config_changed(env: &soroban_sdk::Env, admin: Address, amount: i128) {
    let sym = soroban_sdk::symbol_short!("rbcfg");
    env.events().publish((sym, admin), (amount,));
}

/// Emitted when a relayer deposits bond.
#[contractevent]
#[derive(Clone)]
pub struct RelayerBondDepositedEvent {
    #[topic]
    pub relayer: Address,
    pub amount: i128,
    pub total_deposited: i128,
}

/// Emitted when a relayer withdraws bond.
#[contractevent]
#[derive(Clone)]
pub struct RelayerBondWithdrawnEvent {
    #[topic]
    pub relayer: Address,
    pub amount: i128,
}

/// Emitted when a relayer failure incident is recorded.
#[contractevent]
#[derive(Clone)]
pub struct RelayerFailureRecordedEvent {
    #[topic]
    pub relayer: Address,
    pub reason: u32,
    pub failure_count: u32,
}

/// Emitted when a relayer is slashed for excessive failures.
#[contractevent]
#[derive(Clone)]
pub struct RelayerSlashedEvent {
    #[topic]
    pub relayer: Address,
    pub slash_amount: i128,
    pub remaining_bond: i128,
    pub slash_percent: u32,
}

/// Emitted when a source stakes tokens.
#[contractevent]
#[derive(Clone)]
pub struct SourceStakedEvent {
    #[topic]
    pub source: Address,
    pub amount: i128,
    pub total_stake: i128,
}

/// Emitted when a source unstakes tokens.
#[contractevent]
#[derive(Clone)]
pub struct SourceUnstakedEvent {
    #[topic]
    pub source: Address,
    pub amount_returned: i128,
}

/// Emitted when a consumer is registered with a tier (#173).
#[contractevent]
#[derive(Clone)]
pub struct ConsumerRegisteredEvent {
    #[topic]
    pub consumer: Address,
    pub tier: u32,
    pub subscription_expiry_ts: u64,
}

/// Emitted when a consumer's tier changes (#173).
#[contractevent]
#[derive(Clone)]
pub struct ConsumerTierChangedEvent {
    #[topic]
    pub consumer: Address,
    pub old_tier: u32,
    pub new_tier: u32,
}

/// Emitted when a consumer pays a tier fee (#173).
#[contractevent]
#[derive(Clone)]
pub struct TierFeePaidEvent {
    #[topic]
    pub consumer: Address,
    pub tier: u32,
    pub amount: i128,
}

/// Emitted when price history is pruned by timestamp cutoff.
pub fn emit_history_pruned_by_timestamp(
    env: &soroban_sdk::Env,
    asset: Address,
    ledger_seq: u32,
    timestamp: u64,
    cutoff: u64,
) {
    let sym = soroban_sdk::symbol_short!("prune_ts");
    env.events()
        .publish((sym, asset), (ledger_seq, timestamp, cutoff));
}
