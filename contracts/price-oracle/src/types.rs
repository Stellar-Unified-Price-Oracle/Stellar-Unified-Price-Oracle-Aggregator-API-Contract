use soroban_sdk::{contracttype, Address, Bytes, Map, String, Symbol, Vec};

pub use crate::errors::ErrorCode;

/// Storage keys used to address contract state in persistent, temporary, and instance storage.
///
/// ## Key Schema (namespace → variants)
///
/// | Namespace | Prefix | Variants |
/// |-----------|--------|----------|
/// | Admin identity | (none) | `Admin` |
/// | Global config | `Cfg` | `CfgMinSources`, `CfgMaxHistory`, `CfgResolution`, `CfgDecimals`, `CfgDescription`, `CfgTimestampThreshold`, `CfgMaxDeviation`, `CfgHeartbeatInterval`, `CfgMaxInvalidSubs`, `CfgAggregationMethod`, `CfgPauseFlag`, `CfgTimelockDuration` |
/// | Source registry | `Src` | `SrcActive(addr)`, `SrcRegistry`, `SrcHeartbeat(addr)`, `SrcInactive(addr)` |
/// | Asset registry | `Asset` | `AssetRegistered(addr)`, `AssetRegistry`, `AssetMetadata(addr)`, `AssetMinPrice(addr)` |
/// | Price data | `Price` | `Submission(asset, src)`, `PriceSubmissionLedger(asset, src)`, `Aggregate(asset)`, `PriceOverride(asset)`, `PriceDeviant(asset, src)` |
/// | History | `Hist` | `PriceHistory(asset, ledger)`, `PriceHistoryLedgers(asset)` |
/// | Timelock ops | `Tl` | `TlPendingOpCount`, `TlPendingOp(id)` |
///
/// Soroban encodes each variant name as an XDR `Symbol` discriminant, so variants are
/// inherently collision-free. The namespace prefixes make the category explicit at the
/// call site and prevent accidental re-use of a name across categories in future additions.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum DataKey {
    // --- Admin ---
    /// The contract administrator's address.
    Admin,
    ReentrancyGuard,
    /// Existence flag for a registered oracle source (`true` when present).
    Source(Address),
    /// Existence flag for a registered asset (`true` when present).
    AssetRegistered(Address),
    /// Latest [`PriceEntry`] submitted by a specific source for a specific asset.
    Submission(Address, Address),
    /// Ledger sequence number of the last submission by a source for an asset.
    SubmissionLedger(Address, Address),
    /// Latest [`AggregatePrice`] computed across all contributing sources for an asset.
    Aggregate(Address),
    /// [`PriceHistoryEntry`] recorded at a specific ledger for an asset (temporary storage).
    PriceHistory(Address, u32),
    /// Ordered list of ledger numbers for which history exists for an asset.
    PriceHistoryLedgers(Address),
    /// The [`OracleSources`] registry (list of sources and their metadata).
    OracleSources,
    /// Ordered list of all registered asset addresses.
    RegisteredAssets,
    /// Minimum number of contributing sources required to publish an aggregate price.
    CfgMinSources,
    /// Maximum number of history entries retained per asset before pruning.
    CfgMaxHistory,
    /// Price resolution window in seconds (SEP-40 `resolution` field).
    CfgResolution,
    /// Decimal precision applied to all prices stored by this contract.
    CfgDecimals,
    /// Human-readable description of this oracle instance.
    CfgDescription,
    /// Maximum allowed difference (in seconds) between a submitted timestamp and ledger time.
    CfgTimestampThreshold,
    /// Maximum allowed price deviation in basis points before flagging a submission.
    CfgMaxDeviation,
    /// Interval in seconds after which a source with no heartbeat is considered inactive.
    CfgHeartbeatInterval,
    /// Maximum number of invalid submissions allowed before a source is suspended.
    CfgMaxInvalidSubs,
    /// Currently active [`AggregationMethod`] stored as a `u32` discriminant.
    CfgAggregationMethod,
    /// Boolean flag indicating whether the contract is paused.
    CfgPauseFlag,
    /// Number of ledgers that must pass between proposing and executing a timelock operation.
    CfgTimelockDuration,

    // --- Source registry (prefix: Src) ---
    /// Existence flag for a registered oracle source (`true` when present).
    SrcActive(Address),
    /// The [`OracleSources`] registry (list of sources and their metadata).
    SrcRegistry,
    /// Unix timestamp of the last heartbeat submitted by a source.
    SrcHeartbeat(Address),
    /// Inactive flag for a source.
    SrcInactive(Address),

    // --- Asset registry (prefix: Asset) ---
    /// Existence flag for a registered asset (`true` when present).
    AssetRegistered(Address),
    /// Ordered list of all registered asset addresses.
    AssetRegistry,
    /// Optional [`AssetMetadata`] attached to a registered asset.
    AssetMetadata(Address),
    /// Optional minimum accepted price (`i128`) for a registered asset.
    AssetMinPrice(Address),
    /// Configurable maximum number of assets that can be registered.
    MaxAssets,

    /// Boolean flag indicating whether the contract is paused.
    PauseFlag,
    /// Monotonically incrementing counter used to assign IDs to pending operations.
    TlPendingOpCount,
    /// A [`PendingOperation`] awaiting timelock expiry before execution.
    PendingOp(u32),
    /// Number of ledgers that must pass between proposing and executing a timelock operation.
    TimelockDuration,
    PriceOverride(Address),
    /// Per-asset resolution override in seconds. When set, overrides the contract-wide resolution.
    AssetResolution(Address),
    /// Cooldown (in ledgers) between trigger_aggregation calls per asset.
    AggregationCooldown,
    /// Ledger of the last trigger_aggregation call per asset.
    LastAggregationTrigger(Address),
    /// Minimum submission interval enforcement (in ledgers) for sources.
    MinSubmissionInterval,
    /// Last submission ledger per (source, asset) pair — for compliance tracking.
    LastSubmissionLedger(Address, Address),
    /// Flag marking a source as non-compliant for a given asset.
    SourceNonCompliant(Address, Address),
    /// Counter and storage for pending batch operations.
    PendingBatchCount,
    /// A pending batch operation.
    PendingBatch(u32),
}

/// A price submission from a single oracle source for a specific asset.
///
/// Stored under [`DataKey::Submission`] keyed by `(asset, source)`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceEntry {
    /// Raw price value scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp (seconds) provided by the source at submission time.
    pub timestamp: u64,
    /// Address of the oracle source that submitted this entry.
    pub source: Address,
    /// Decimal precision in effect when this entry was stored.
    pub decimals: u32,
    /// Ledger sequence number when this entry was last written.
    pub last_updated: u32,
}

/// An aggregated price computed from multiple oracle sources for a specific asset.
///
/// Stored under [`DataKey::Aggregate`] and updated on every [`PriceEntry`] submission
/// that results in enough contributing sources to meet the minimum threshold.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AggregatePrice {
    /// Aggregated price value scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp of the most-recent contributing submission.
    pub timestamp: u64,
    /// Number of sources that contributed to this aggregate.
    pub num_sources: u32,
    /// Decimal precision applied to `price`.
    pub decimals: u32,
    pub is_override: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceOverrideEntry {
    pub price: i128,
    pub reason: String,
    pub expiry_ledger: u32,
    pub set_ledger: u32,
}

/// A snapshot of the aggregate price recorded at a particular ledger.
///
/// Stored in temporary storage under [`DataKey::PriceHistory`] keyed by `(asset, ledger)`.
/// Entries are pruned to the configured `max_history_length` on each new aggregation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceHistoryEntry {
    /// Aggregated price value scaled by the contract's decimal precision.
    pub price: i128,
    /// Unix timestamp of the most-recent contributing submission at snapshot time.
    pub timestamp: u64,
    /// Ledger sequence number when this snapshot was recorded.
    pub ledger: u32,
    /// Number of sources that contributed to this price.
    pub num_sources: u32,
}

/// Registry of all authorized oracle sources and their display names.
///
/// Stored under [`DataKey::OracleSources`] and updated by [`add_source`] /
/// [`remove_source`] operations.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct OracleSources {
    /// Ordered list of authorized source addresses.
    pub sources: Vec<Address>,
    /// Human-readable display name for each source, keyed by address.
    pub metadata: Map<Address, String>,
}

/// Represents a priced asset, following the SEP-40 oracle interface convention.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum Asset {
    /// A Stellar token identified by its contract address.
    Stellar(Address),
    /// A non-Stellar asset identified by a short symbol (e.g. `"USD"`, `"BTC"`).
    Other(Symbol),
}

/// Strategy used when combining multiple source prices into a single aggregate.
///
/// Stored as a `u32` discriminant under [`DataKey::AggregationMethod`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum AggregationMethod {
    /// Select the middle value after sorting; resistant to outliers. (default)
    Median = 0,
    /// Arithmetic mean of all submitted prices.
    Mean = 1,
    /// Arithmetic mean after removing the top and bottom 10 % of values.
    TrimmedMean = 2,
}

/// SEP-40 compatible price data returned by the standard oracle interface methods.
///
/// Used as the return type of [`lastprice`], [`price`], and [`prices`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceData {
    /// Aggregated price value scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp (seconds) of the price observation.
    pub timestamp: u64,
    /// Ledger sequence number when this data was last updated.
    pub last_updated: u32,
}

/// Discriminant for operations that require timelock protection before execution.
///
/// Used in [`PendingOperation`] and mapped to/from `u32` in the public API.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum OperationType {
    /// Upgrade the contract WASM hash.
    Upgrade = 0,
    /// Replace the administrator address.
    SetAdmin = 1,
    /// Change the minimum number of required sources.
    SetMinSources = 2,
    /// Change the maximum retained history length.
    SetMaxHistory = 3,
    /// Change the price resolution window.
    SetResolution = 4,
    /// Change the decimal precision.
    SetDecimals = 5,
    /// Update the contract description string.
    SetDescription = 6,
    /// Adjust the timestamp validity threshold.
    SetTimestampThreshold = 7,
}

/// A governance operation that has been proposed and is waiting for its timelock to expire.
///
/// Stored under [`DataKey::PendingOp`] keyed by `id`. Removed once executed or cancelled.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PendingOperation {
    /// Unique sequential identifier assigned at proposal time.
    pub id: u32,
    /// Kind of administrative change being proposed.
    pub op_type: OperationType,
    /// Address of the admin who proposed this operation.
    pub proposed_by: Address,
    /// Ledger sequence number when this operation was proposed.
    pub proposed_ledger: u32,
    /// Arbitrary encoded payload whose interpretation depends on `op_type`.
    pub data: Bytes,
}

/// Optional metadata that can be attached to a registered asset.
///
/// Stored under [`DataKey::AssetMetadata`] and managed via `set_asset_metadata`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AssetMetadata {
    /// Human-readable name of the asset (e.g. `"Wrapped Bitcoin"`).
    pub name: String,
    /// Trading symbol of the asset (e.g. `"WBTC"`).
    pub symbol: String,
    /// Optional override for the number of decimals used by this asset's token contract.
    /// When `None`, the contract-wide decimal setting applies.
    pub decimals: Option<u32>,
}

/// A single admin operation within a batch, identified by type and its encoded payload.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct BatchOperation {
    /// Numeric discriminant matching [`OperationType`] (0–7).
    pub op_type: u32,
    /// Encoded payload for the operation (same encoding as single [`PendingOperation`]).
    pub data: Bytes,
}

/// A pending batch of admin operations waiting for its timelock to expire.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PendingBatch {
    /// Unique sequential identifier assigned at proposal time.
    pub id: u32,
    /// Address of the admin who proposed the batch.
    pub proposed_by: Address,
    /// Ledger when the batch was proposed.
    pub proposed_ledger: u32,
    /// Ordered list of operations to execute atomically.
    pub operations: Vec<BatchOperation>,
}
