use soroban_sdk::{contractevent, Address, String};

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
