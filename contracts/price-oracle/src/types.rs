use soroban_sdk::{contracttype, Address, Bytes, BytesN, Map, String, Symbol, Vec};

pub use crate::errors::ErrorCode;

pub type SubscriptionPlans = Map<u32, i128>;

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
    /// Per-source deviation tolerance in basis points. Overrides global when set.
    SrcDeviationTolerance(Address),

    // --- Asset registry (prefix: Asset) ---
    /// Existence flag for a registered asset (`true` when present).
    AssetRegistered(Address),
    /// Ordered list of all registered asset addresses (used for enumeration).
    AssetRegistry,

    /// O(1) membership index: `true` when an asset address is registered.
    ///
    /// Kept separate from `AssetRegistry` so we can provide efficient
    /// `is_asset_registered` / `check_registered_asset` lookups while
    /// preserving the historical ordering exposed by `assets()`.
    AssetRegistryIndex(Address),

    /// Optional [`AssetMetadata`] attached to a registered asset.
    AssetMetadata(Address),
    /// Optional minimum accepted price (`i128`) for a registered asset.
    AssetMinPrice(Address),
    /// Per-asset price bounds applied to new submissions.
    AssetPriceBounds(Address),
    /// Whether an asset has been explicitly paused by the admin.
    AssetPauseFlag(Address),
    /// Whether the circuit breaker has tripped for an asset.
    AssetCircuitBreakerTripped(Address),
    /// Sequence counter for circuit-breaker event log entries.
    AssetCircuitBreakerLogCount(Address),
    /// Append-only circuit-breaker event log entry.
    AssetCircuitBreakerLog(Address, u32),
    /// Configurable maximum number of assets that can be registered.
    MaxAssets,

    // -------------------------------------------------------------------------
    // #301: Automatic asset deregistration on inactivity
    // -------------------------------------------------------------------------
    /// Last ledger sequence number when a price was submitted for this asset.
    AssetLastActivity(Address),
    /// Inactivity timeout in ledgers for a specific asset (0 = use global default).
    AssetInactivityTimeout(Address),
    /// Global default inactivity timeout in ledgers (0 = disabled).
    CfgInactivityTimeout,

    /// Boolean flag indicating whether the contract is paused.
    PauseFlag,
    /// Monotonically incrementing counter used to assign IDs to pending operations.
    TlPendingOpCount,
    /// A [`PendingOperation`] awaiting timelock expiry before execution.
    TlPendingOp(u32),
    /// Number of ledgers that must pass between proposing and executing a timelock operation.
    TimelockDuration,
    /// Tracks the number of queries made by a consumer for a specific ledger.
    QueryCount(Address, u32),
    /// Maximum number of queries allowed per ledger for rate limiting.
    QueryRateLimit,
    /// Expiry timestamp for a consumer's subscription.
    SubscriptionExpiry(Address),
    /// Available subscription plans mapped by duration (seconds) to amount (stroops).
    SubscriptionPlans,
    PriceOverride(Address),
    /// Per-asset resolution override in seconds. When set, overrides the contract-wide resolution.
    AssetResolution(Address),
    /// Number of optimistic price proposals created so far.
    OptimisticProposalCount,
    /// An optimistic price proposal keyed by proposal id.
    OptimisticProposal(u32),
    /// Configurable dispute window in ledgers for optimistic proposals.
    CfgOptimisticDisputeWindow,
    /// Minimum bond amount required to submit an optimistic proposal.
    CfgOptimisticMinBond,
    /// Bond balance tracked for an address after proposal/dispute settlement.
    OptimisticBondBalance(Address),
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
    /// Current storage schema version (u32). Absent means version 1.
    StorageVersion,
    /// Active migration state, if a migration is in progress.
    MigrationState,

    // --- #66: Phased removal (used in sources.rs but missing from original enum) ---
    /// Ledger at which a source becomes eligible for finalization after mark_source_for_removal.
    SourcePendingRemoval(Address),
    /// Decay factor for source reputation scores (u32, out of 100).
    ReputationDecayFactor,
    /// Reputation score for a source (i128, 0–100).
    SourceReputation(Address),
    /// Cooldown in ledgers between mark_source_for_removal and finalize_source_removal.
    RemovalCooldown,
    /// Maximum price deviation key used in admin.rs (mirrors CfgMaxDeviation for set path).
    MaxPriceDeviation,
    /// Timestamp threshold key used in admin.rs set path (mirrors CfgTimestampThreshold).
    TimestampThreshold,
    /// Minimum sources key used in timelock.rs batch execution (mirrors CfgMinSources).
    MinSourcesRequired,
    /// Max history length key used in timelock.rs batch execution (mirrors CfgMaxHistory).
    MaxHistoryLength,
    /// Resolution key used in timelock.rs batch execution (mirrors CfgResolution).
    Resolution,
    /// Decimals key used in timelock.rs batch execution (mirrors CfgDecimals).
    Decimals,

    // -------------------------------------------------------------------------
    // #92/#93/#94: event spam protection, max aggregation sources, per-asset history cap
    // -------------------------------------------------------------------------
    /// Per-asset maximum number of price entries before the oldest is pruned (issue #94).
    MaxHistoryPerAsset,
    /// Maximum number of events that may be emitted in a single call (issue #92).
    MaxEventsPerCall,
    /// Maximum number of sources used for aggregation; excess sources are randomly
    /// sub-sampled using the ledger hash (issue #93).
    MaxAggregationSources,
    /// Maximum number of oracle sources that may be registered. `0` = unlimited.
    MaxSources,
    /// Whether linear interpolation is used for historical price lookups that miss
    /// an exact ledger match.
    InterpolationEnabled,

    // -------------------------------------------------------------------------
    // #186: Adaptive heartbeat / liveness detection
    // -------------------------------------------------------------------------
    /// Number of consecutive missed heartbeats for a source (u32).
    SrcMissedHeartbeats(Address),
    /// Ledger sequence of the last price submission from a source (u32).
    SrcLastPriceLedger(Address),
    /// Flag: source has submitted a price since its last heartbeat reactivation (bool).
    SrcPriceSubmitAfterReactivation(Address),
    /// Ledger sequence at which a source first became inactive — for auto-removal timer (u32).
    SrcInactiveSinceLedger(Address),
    /// Maximum ledgers a source may remain inactive before automatic removal (u32).
    CfgMaxInactiveLedgers,
    /// Window size (number of heartbeat periods) used when computing the adaptive interval (u32).
    CfgHeartbeatWindow,

    // -------------------------------------------------------------------------
    // #187: Commit-reveal MEV resistance
    // -------------------------------------------------------------------------
    /// A pending price commit: stores PriceCommit under (asset, source, round_ledger).
    PriceCommit(Address, Address, u32),
    /// Number of ledgers that the commit phase lasts (sources must commit within this window).
    CfgCommitWindow,
    /// Number of ledgers after the commit deadline during which sources may reveal.
    CfgRevealWindow,
    /// Number of faulty sources the BFT aggregator is configured to tolerate.
    CfgBftFaultTolerance,
    /// Aggregation method used by the BFT path.
    CfgBftAggregationMethod,

    // -------------------------------------------------------------------------
    // #188: Economic finality gadget
    // -------------------------------------------------------------------------
    /// A pending finality entry for an asset at a given ledger (PendingFinalityEntry).
    PendingFinality(Address, u32),
    /// Number of ledgers to wait before an aggregated price is considered finalized (u32).
    CfgFinalityLedgers,
    /// Recorded ledger hash for reorg detection, keyed by ledger sequence number (BytesN<32>).
    LedgerHashChain(u32),
    /// The finalized aggregate price for an asset (FinalizedPrice).
    FinalizedPrice(Address),

    // -------------------------------------------------------------------------
    // #171: Source Reputation & Slashing
    // -------------------------------------------------------------------------
    /// Staking record for an oracle source (SourceStakeRecord).
    SourceStake(Address),
    /// Address of the XLM/oracle token contract used for staking and fees.
    StakeTokenContract,
    /// Percentage of stake to slash (u32, 0–100) per slash event.
    SlashPercent,
    /// Threshold below which a source's reputation triggers slash eligibility (u32, 0–100).
    SlashReputationThreshold,
    /// Total slashed funds held in contract treasury (i128 stroops).
    TreasuryBalance,

    // -------------------------------------------------------------------------
    // #172: Cross-Asset Correlation
    // -------------------------------------------------------------------------
    /// Min/max ratio band for a (base_asset, quote_asset) pair.
    CorrelationBand(Address, Address),
    /// Ordered list of (base, quote) correlation pairs registered.
    CorrelationPairList,
    /// Flag marking a (source, asset) price submission as correlation-violating.
    /// Flagged submissions are excluded from aggregation.
    CorrelationFlagged(Address, Address),

    // -------------------------------------------------------------------------
    // #173: Tiered Consumer Whitelisting
    // -------------------------------------------------------------------------
    /// Consumer tier and quota info for an address.
    ConsumerInfo(Address),
    /// Pricing for each tier (ConsumerTier discriminant → price in stroops per ledger cycle).
    TierPricing(u32),
    /// Per-ledger query counter for a tiered consumer.
    TierQueryCount(Address, u32),
    /// Treasury address for XLM fee sweeps.
    WhitelistTreasury,
    /// Address of the XLM token contract used for subscription fee collection.
    XlmTokenContract,

    // -------------------------------------------------------------------------
    // Gas metering
    // -------------------------------------------------------------------------
    /// Storage key for the last recorded gas usage (submit/aggregate)
    LastGasRecord,

    // -------------------------------------------------------------------------
    // #174: Price Deviation Alerts
    // -------------------------------------------------------------------------
    /// Alert subscription record for a (consumer, asset) pair.
    AlertSubscription(Address, Address),
    /// Ordered list of (consumer, asset) pairs that have active alert subscriptions.
    AlertSubscriptionList,
    /// Maximum number of alert subscriptions allowed globally.
    MaxAlertSubscriptions,
    /// TTL in ledgers for alert subscriptions before auto-expiry.
    AlertSubscriptionTtl,
    /// Last aggregate price recorded per asset for deviation comparison.
    AlertLastPrice(Address),

    // -------------------------------------------------------------------------
    // Off-chain relayer network integration
    // -------------------------------------------------------------------------
    /// Metadata for an approved relayer address.
    ApprovedRelayer(Address),
    /// Fee (in stroops) credited to a relayer per successful relayed price submission.
    RelayerFeePerSubmission,
    /// Accumulated fee balance (in stroops) owed to a relayer.
    RelayerFeeBalance(Address),
    /// Running count of successful price submissions made by a relayer.
    RelayerSubmissionCount(Address),

    // -------------------------------------------------------------------------
    // Cross-reference oracle checks
    // -------------------------------------------------------------------------
    /// Stored [`ReferenceOracleEntry`] for a registered external reference oracle.
    ReferenceOracle(Address),
    /// Ordered list of registered reference oracle contract addresses.
    ReferenceOracleList,
    /// Allowed deviation in basis points before a cross-reference alert is emitted.
    CrossRefDeviationThreshold,
    /// Demerit and disqualification state for an oracle source.
    SourceDemerits(Address),
    /// Configured thresholds for progressive disqualification.
    DemeritConfig,
    /// Configured multi-sig source governance settings.
    SourceGovConfig,
    /// Total number of source proposals created.
    SourceProposalCount,
    /// A pending source proposal details.
    SourceProposal(u32),
    /// Geolocation metadata for a registered oracle source.
    SourceGeo(Address),
    /// Configured liveness bond amount required for sources.
    SourceBondAmount,
    /// Deposited bond amount for a registered oracle source.
    SourceBond(Address),

    // -------------------------------------------------------------------------
    // Additional keys for feature modules
    // -------------------------------------------------------------------------
    /// Per-asset minimum submission interval override.
    AssetMinSubmissionInterval(Address),

    /// Active source set for an asset rotation schedule (#206).
    AssetActiveSourceSet(Address),
    /// Standby source set for an asset rotation schedule (#206).
    AssetStandbySourceSet(Address),
    /// Rotation schedule for an asset (#206).
    AssetRotationSchedule(Address),
    /// Next rotation ledger for an asset (#206).
    AssetNextRotationLedger(Address),

    /// Ledger when an asset's TTL was last extended (#203).
    AssetLastTtlExtended(Address),

    /// Per-day operation count for an admin op type (daily limit).
    AdminOpDailyCount(u32, u32),
    /// Per-day operation limit configuration for an admin op type.
    AdminOpDailyLimit(u32),

    /// AMM pool data for an asset (#180).
    AmmPool(Address),
    /// AMM maximum deviation basis points for an asset.
    AmmMaxDeviationBps(Address),
    /// AMM weight configuration for aggregation inclusion.
    AmmWeight(Address),
    /// Stellar DEX pool reserves for an asset pair.
    DexPool(Address, Address),
    /// Soroswap pool configuration for an asset pair.
    SoroswapPool(Address, Address),

    /// Challenge entries keyed by ID (#235).
    Challenge(u32),
    /// Total challenge count (#235).
    ChallengeCount,
    /// Challenger rewards balance (#235).
    ChallengerRewards(Address),

    /// Cross-chain relay configuration (#182).
    CrossChainRelayConfig,

    /// Submission deadline for an asset (#202).
    SubmissionDeadline(Address),
    /// Rebate amount available for a source/asset pair (#202).
    SubmissionRebate(Address, Address),
    /// Total rebate balance available for distribution (#202).
    RebateBalance,

    /// Event type registry for structured event indexing (#201).
    EventTypeRegistry,

    /// Exotic asset pricing configuration (#177).
    ExoticAssetConfig(Address),

    /// Fee market pending queue (#176).
    FmPendingQueue,
    /// Fee market fee pool balance (#176).
    FmFeePool,
    /// Per-source fee balance in the fee market (#176).
    FmSourceFeeBalance(Address),
    /// Fee market treasury address (#176).
    FmTreasury,
    /// Total treasury balance in fee market (#176).
    FmTreasuryBalance,
    /// Minimum priority fee setting (#176).
    FmMinPriorityFee,
    /// Fee distribution ratio (bps to sources vs treasury) (#176).
    FmFeeDistributionRatio,

    /// Multi-sig governors list (#178).
    MsGovernors,
    /// Multi-sig required approval count (#178).
    MsRequiredApprovals,
    /// A multi-sig operation by ID (#178).
    MsOp(u32),
    /// Multi-sig queue head pointer (#178).
    MsQueueHead,
    /// Multi-sig queue tail pointer (#178).
    MsQueueTail,
    /// Total multi-sig operation count (#178).
    MsOpCount,

    /// State channel per source (#179).
    StateChannel(Address),

    /// Current aggregation round metadata.
    CurrentAggregationRound,

    /// VDF sampling size configuration.
    VdfSamplingSize,

    /// ZK verifying key storage (#175).
    ZkVerifyingKey,

    /// Per-asset decimal configuration (#227).
    AssetDecimals(Address),

    /// Delegated role: (holder, role_discriminant) → bool.
    DelegatedRole(Address, u32),
    /// Role holders list keyed by role discriminant.
    RoleHolders(u32),

    /// Emergency pause entry (#240).
    EmergencyPauseEntry,
    /// Whether emergency pause is currently active (#240).
    EmergencyPauseActive,
    /// Reason for emergency pause (#240).
    EmergencyPauseReason,

    /// Per-source fee credit balance (for source reward schemes).
    SourceFeeBalance(Address),
    /// Total submission count across all sources.
    TotalSubmissionCount,
    /// Per-source submission count.
    SourceSubmissionCount(Address),

    /// Circuit breaker threshold in basis points.
    CircuitBreakerThreshold,

    /// Audit log entry by ID (#239).
    AuditEntry(u32),
    /// Total audit log entry count (#239).
    AuditEntryCount,
    /// Current audit log chain head hash (#239).
    AuditLogHead,

    // -------------------------------------------------------------------------
    // Pre-existing keys referenced by storage.rs/sources.rs/cross_chain_verify.rs
    // but missing from this enum (build-blocking gap, restored here).
    // -------------------------------------------------------------------------
    /// Assets a source is authorized to submit prices for (#226 support).
    SourceAssets(Address),
    /// Ledger sequence of a source's last key rotation (#226 support).
    SourceRotationLedger(Address),
    /// Verification metadata for a source (#226 support).
    SourceVerification(Address),
    /// Whether cross-chain price verification is globally enabled (#226).
    CrossChainVerificationEnabled,
    /// Maximum allowed deviation (bps) between this chain and a reference chain (#226).
    CrossChainDeviationThreshold,
    /// Stored cross-chain price observation for (asset, oracle_chain) (#226).
    CrossChainPrice(Address, Address),

    // -------------------------------------------------------------------------
    // #216: Off-chain signature-verified price submission
    // -------------------------------------------------------------------------
    /// Ed25519 public key registered by a source for pre-signed submissions.
    SignedSubmitPubKey(Address),
    /// Last accepted (strictly increasing) nonce for a source's signed submissions.
    SignedSubmitNonce(Address),

    // -------------------------------------------------------------------------
    // #218: Configurable aggregation triggers
    // -------------------------------------------------------------------------
    /// Minimum seconds between time-triggered aggregations for an asset (0 = disabled).
    TriggerTimeInterval(Address),
    /// Number of new submissions required to auto-trigger aggregation (0 = disabled).
    TriggerSubmissionThreshold(Address),
    /// Submissions accumulated since the last trigger-driven aggregation for an asset.
    TriggerSubmissionCount(Address),
    /// Deviation in basis points that auto-triggers aggregation (0 = disabled).
    TriggerDeviationBps(Address),
    /// Unix timestamp of the last trigger-driven aggregation for an asset.
    TriggerLastAggTime(Address),
    // #223: Price freeze mechanism for market emergencies
    // -------------------------------------------------------------------------
    /// Frozen price snapshot for an asset, present only while frozen (#223).
    FrozenPrice(Address),

    // -------------------------------------------------------------------------
    // #229: Cursor-paginated historical price queries
    // -------------------------------------------------------------------------
    // (no additional storage keys — pagination reuses `PriceHistoryLedgers`)

    // -------------------------------------------------------------------------
    // #243: Admin notification preference system
    // -------------------------------------------------------------------------
    /// Notification preferences registered for a given event-type discriminant (#243).
    NotificationPrefs(u32),
    /// Every event-type discriminant that currently has at least one preference (#243).
    NotificationEventTypes,

    // -------------------------------------------------------------------------
    // History export / timelock priority
    // -------------------------------------------------------------------------
    /// Delay (ledgers) for Urgent priority operations.
    TlUrgentDelay,
    /// Delay (ledgers) for Normal priority operations (mirrors CfgTimelockDuration).
    TlNormalDelay,
    /// Delay (ledgers) for LongTerm priority operations.
    TlLongTermDelay,

    // -------------------------------------------------------------------------
    // #283: Stellar DID Integration
    // -------------------------------------------------------------------------
    /// Stored DID document for a decentralized identity address.
    DidDocument(Address),
    /// Source address mapped to a DID address for identity verification.
    SourceDid(Address),

    // -------------------------------------------------------------------------
    // #282: Bridge Oracle for Non-Stellar Assets
    // -------------------------------------------------------------------------
    /// Bridge oracle configuration for a (source_asset, target_asset) pair.
    BridgeOracle(Address, Address),
    /// Latest bridged price observation for an asset pair.
    BridgedPrice(Address, Address),

    // -------------------------------------------------------------------------
    // #285: Ecosystem Metadata Registration
    // -------------------------------------------------------------------------
    /// Stellar ecosystem metadata registry entry.
    EcosystemMetadata,

    // -------------------------------------------------------------------------
    // Severity-aware alerting
    // -------------------------------------------------------------------------
    /// Global default severity thresholds (basis points) used when no per-asset
    /// override is configured.
    CfgSeverityThresholds,
    /// Per-asset severity threshold override.
    AssetSeverityThresholds(Address),
    /// Most recent severity classification emitted for an asset.
    LastAlertSeverity(Address),
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
    pub ledger_timestamp: u64,
    /// Optional liquidity/volume weight used by VWAP aggregation.
    pub volume: Option<i128>,
}

/// Aggregated price bounds applied to an asset before a submission is accepted.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceBounds {
    /// Minimum accepted price for the asset.
    pub min_price: i128,
    /// Maximum accepted price for the asset.
    pub max_price: i128,
    /// Maximum allowed percentage change (in basis points) between the previous aggregate
    /// and the candidate aggregate in a single ledger.
    pub max_change_bps_per_ledger: u32,
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
    /// Monotonically-incrementing version counter. Starts at 0 and increments by 1
    /// each time the aggregate price changes. Allows consumers to detect price
    /// changes without comparing i128 values. Persists across contract upgrades.
    pub version: u32,
}

/// Result type for `get_aggregate_with_version`: the aggregate price plus its version number.
///
/// Returned by [`get_aggregate_with_version`] so callers can poll for changes using only
/// the lightweight `version` field rather than comparing full `i128` prices.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct VersionedAggregatePrice {
    /// The full aggregate price record.
    pub aggregate: AggregatePrice,
    /// The current version counter — mirrors `aggregate.version` for ergonomic access.
    pub version: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CircuitBreakerEventEntry {
    /// Address of the asset affected by the breaker trip.
    pub asset: Address,
    /// Previous aggregate price before the candidate update.
    pub previous_price: i128,
    /// Candidate aggregate price that would have been published.
    pub candidate_price: i128,
    /// Percentage change in basis points that triggered the breaker.
    pub change_bps: u32,
    /// Maximum allowed change in basis points per ledger.
    pub max_change_bps: u32,
    /// Ledger where the breaker tripped.
    pub ledger: u32,
    /// Unix timestamp of the breaker trip.
    pub timestamp: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceOverrideEntry {
    pub price: i128,
    pub reason: String,
    pub expiry_ledger: u32,
    pub set_ledger: u32,
}

/// Subscription plan configuration.
///
/// Stored under [`DataKey::SubscriptionPlan`] or similar key.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SubscriptionPlan {
    /// Duration of the subscription plan in seconds.
    pub duration: u32,
    /// Cost amount in stroops (i128 for precision).
    pub amount: i128,
}

/// Subscription expiry record for a consumer address.
///
/// Stored under [`DataKey::SubscriptionExpiry`] keyed by consumer address.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SubscriptionExpiry {
    /// Unix timestamp (seconds) at which the subscription expires.
    pub expiry_timestamp: u64,
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
    /// `true` when this entry was produced by linear interpolation rather than a
    /// real submission. Consumers should treat interpolated values as estimates.
    pub is_interpolated: bool,
}

/// A frozen aggregate price snapshot, recorded when an admin freezes an asset
/// during a market emergency (#223). While present, it takes priority over the
/// live aggregate for `get_price` and blocks new submissions.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct FrozenPrice {
    /// The aggregate price at the moment of freezing, scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp of the aggregate at the moment of freezing.
    pub timestamp: u64,
    /// Decimal precision in effect when the price was frozen.
    pub decimals: u32,
    /// Admin-supplied human-readable reason for the freeze.
    pub reason: String,
    /// Ledger sequence number at which the freeze was triggered.
    pub frozen_at_ledger: u32,
}

/// An admin-configured off-chain notification target for a given event type (#243).
///
/// Dispatch happens off-chain: an external relayer service watches contract events
/// and forwards them to the configured `channel`/`target` pairs registered here.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct NotificationPreference {
    /// Event-type discriminant this preference applies to.
    pub event_type: u32,
    /// Notification channel kind (e.g. "webhook", "email").
    pub channel: String,
    /// Channel-specific target (URL, email address, etc).
    pub target: String,
}

/// Gas usage record for the most-recent submit/aggregate operation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct GasRecord {
    /// Human-readable method name (e.g. "submit_price", "aggregate").
    pub method: String,
    /// CPU instructions consumed during the recorded call.
    pub cpu_instructions: u64,
    /// Memory bytes consumed during the recorded call.
    pub memory_bytes: u64,
    /// Ledger sequence when the recorded call occurred.
    pub ledger: u32,
    /// Unix timestamp when the recorded call occurred.
    pub timestamp: u64,
}

/// Report entry describing a storage key and (where available) remaining TTL.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StorageTtlEntry {
    /// Descriptive key name.
    pub key: String,
    /// Whether the key currently exists.
    pub exists: bool,
    /// Remaining TTL in ledgers (0 if unavailable/unknown).
    pub remaining_ttl: u32,
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
    /// Optional proof-of-identity verification metadata for each source.
    pub verification: Map<Address, SourceVerification>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SourceVerification {
    pub verified: bool,
    pub verification_method: String,
    pub verifier: Address,
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
#[allow(clippy::upper_case_acronyms)]
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum AggregationMethod {
    /// Select the middle value after sorting; resistant to outliers. (default)
    Median = 0,
    /// Arithmetic mean of all submitted prices.
    Mean = 1,
    /// Arithmetic mean after removing the top and bottom 10 % of values.
    TrimmedMean = 2,
    /// Volume-weighted average price using submitted positive volumes.
    VWAP = 3,
}

/// Aggregation modes available inside the BFT path.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum BftAggregationMethod {
    /// Use the median of the consensus set after removing outliers.
    Median = 0,
    /// Use the mean of the consensus set after removing outliers.
    Mean = 1,
    /// Use a trimmed mean of the consensus set after removing outliers.
    TrimmedMean = 2,
}

/// TWAP aggregation variant.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum TwapMethod {
    /// Standard arithmetic TWAP using time-weighted average.
    Arithmetic = 0,
    /// Geometric TWAP using a time-weighted geometric mean.
    Geometric = 1,
}

/// Lifecycle state for an optimistic proposal.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum OptimisticProposalStatus {
    Pending = 0,
    Finalized = 1,
    Disputed = 2,
    Resolved = 3,
}

/// An optimistic price proposal that can be disputed before finalization.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct OptimisticProposal {
    pub id: u32,
    pub asset: Address,
    pub proposer: Address,
    pub price: i128,
    pub timestamp: u64,
    pub bond_amount: i128,
    pub dispute_window: u32,
    pub expires_at_ledger: u32,
    pub status: u32,
    pub disputed: bool,
    pub resolved: bool,
    pub resolution: u32,
    pub disputer: Option<Address>,
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
    /// Priority tier that determines the required delay before execution.
    pub priority: OperationPriority,
}

/// A snapshot of the oracle's overall health, returned by `health_check()`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct HealthReport {
    /// Total number of registered oracle sources.
    pub total_sources: u32,
    /// Number of sources that are currently active (not inactive/suspended).
    pub active_sources: u32,
    /// Total number of registered assets.
    pub total_assets: u32,
    /// Number of assets that have at least one aggregate price recorded.
    pub assets_with_prices: u32,
    /// Whether the contract is currently paused (aggregation suspended).
    pub is_aggregation_paused: bool,
    /// Ledger sequence number of the most recent price aggregation, or 0 if none.
    pub last_aggregation_ledger: u32,
    /// Number of assets whose latest price is stale (older than `resolution` seconds).
    /// Always 0 when `resolution` is 0 (staleness checking disabled).
    pub stale_price_count: u32,
    /// Number of sources currently marked as suspended or inactive.
    pub suspended_source_count: u32,
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
    /// Logo URI of the asset.
    pub logo_uri: String,
}

/// Helper struct for batch asset metadata updates.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AssetMetadataUpdate {
    pub asset: Address,
    pub name: String,
    pub symbol: String,
    pub decimals: Option<u32>,
    pub logo_uri: String,
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

/// Status of an in-progress storage migration.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum MigrationStatus {
    /// Migration has been started but not yet completed.
    InProgress = 0,
    /// Migration completed successfully.
    Completed = 1,
}

/// Tracks progress of a storage migration from one version to the next.
///
/// Stored under [`DataKey::MigrationState`] while a migration is running.
/// Removed once the migration completes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct MigrationState {
    /// Storage version being migrated from.
    pub from_version: u32,
    /// Storage version being migrated to.
    pub to_version: u32,
    /// Index of the next item to process (supports pause/resume).
    pub cursor: u32,
    /// Ledger when the migration was started.
    pub started_ledger: u32,
    /// Current migration status.
    pub status: MigrationStatus,
}

// =============================================================================
// #186 — Adaptive Heartbeat / Liveness Detection
// =============================================================================

/// Liveness health status of an oracle source.
///
/// Returned by `get_source_health(source)`. Each variant encodes progressively
/// worse liveness signals.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum SourceHealthStatus {
    /// Source is submitting heartbeats and prices within the adaptive interval.
    Healthy = 0,
    /// Source has missed one or more heartbeats but is still within the allowed window.
    /// The inner `u32` is the consecutive-miss count.
    Degraded = 1,
    /// Source has exceeded the consecutive-miss threshold and is marked inactive.
    Inactive = 2,
    /// Source was automatically removed after exceeding `max_inactive_ledgers`.
    AutoRemoved = 3,
}

// =============================================================================
// #187 — Commit-Reveal MEV Resistance
// =============================================================================

/// A committed (but not yet revealed) price submission.
///
/// Stored in temporary storage under [`DataKey::PriceCommit`] keyed by
/// `(asset, source, round_ledger)`. Expires after `commit_window + reveal_window`
/// ledgers to bound storage growth.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceCommit {
    /// sha256(price_bytes || salt || round_ledger_bytes) as a 32-byte hash.
    pub hash: BytesN<32>,
    /// Ledger sequence number when this commit was submitted (= the round's start ledger).
    pub committed_ledger: u32,
    /// Address of the source that made this commit.
    pub source: Address,
    /// Address of the asset this commit is for.
    pub asset: Address,
    /// Whether this commit has been revealed (prevents double-reveal).
    pub revealed: bool,
}

// =============================================================================
// #188 — Economic Finality Gadget
// =============================================================================

/// Finality status of an aggregated price.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum FinalityStatus {
    /// Price is awaiting the finality window. Inner `u32` = ledger it becomes final.
    Pending = 0,
    /// Price has passed the finality window without retraction — immutable.
    Finalized = 1,
    /// Price was retracted by admin before finalization (reorg protection).
    Retracted = 2,
}

/// A price aggregation result that is in the pending-finality window.
///
/// Stored under [`DataKey::PendingFinality`] keyed by `(asset, committed_ledger)`.
/// After `finality_ledgers` pass, it transitions to [`DataKey::FinalizedPrice`].
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PendingFinalityEntry {
    /// The aggregated price value (scaled by `10^decimals`).
    pub price: i128,
    /// Unix timestamp of the aggregation.
    pub timestamp: u64,
    /// Number of sources that contributed.
    pub num_sources: u32,
    /// Decimal precision applied to `price`.
    pub decimals: u32,
    /// Ledger at which this price was first aggregated.
    pub committed_ledger: u32,
    /// Ledger after which this price is considered finalized (= committed_ledger + finality_ledgers).
    pub finality_ledger: u32,
    /// Current status.
    pub status: FinalityStatus,
    /// Ledger hash recorded at `committed_ledger` — used for reorg detection.
    pub ledger_hash: BytesN<32>,
}

/// An immutable, finalized price record.
///
/// Written to [`DataKey::FinalizedPrice`] when a [`PendingFinalityEntry`] passes
/// `finality_ledger` without retraction.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct FinalizedPrice {
    /// The finalized aggregated price.
    pub price: i128,
    /// Unix timestamp of the finalized aggregation.
    pub timestamp: u64,
    /// Number of contributing sources.
    pub num_sources: u32,
    /// Decimal precision.
    pub decimals: u32,
    /// Ledger at which this price was originally aggregated.
    pub committed_ledger: u32,
    /// Ledger at which finality was confirmed.
    pub finalized_ledger: u32,
}

// =============================================================================
// #171 — Source Reputation & Slashing
// =============================================================================

/// Staking record for an oracle source.
///
/// Stored under [`DataKey::SourceStake`] keyed by source address.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SourceStakeRecord {
    /// Locked stake amount in stroops.
    pub amount: i128,
    /// Ledger at which the stake was last updated.
    pub last_updated_ledger: u32,
}

// =============================================================================
// #172 — Cross-Asset Correlation
// =============================================================================

/// Acceptable ratio band between two correlated assets.
///
/// The ratio is computed as `price_base * RATIO_PRECISION / price_quote` and must
/// fall within `[min_ratio, max_ratio]`. All values are scaled by `RATIO_PRECISION`
/// (10^7) to avoid floating-point arithmetic.
///
/// Stored under [`DataKey::CorrelationBand`] keyed by `(base_asset, quote_asset)`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CorrelationBand {
    /// Minimum acceptable ratio (scaled by 10^7).
    pub min_ratio: u128,
    /// Maximum acceptable ratio (scaled by 10^7).
    pub max_ratio: u128,
    /// Whether this correlation check is currently active.
    pub enabled: bool,
}

/// A registered correlation pair for enumeration purposes.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CorrelationPair {
    pub base_asset: Address,
    pub quote_asset: Address,
}

// =============================================================================
// #173 — Tiered Consumer Access
// =============================================================================

/// Consumer access tier.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum ConsumerTier {
    /// Free tier: 10 queries/ledger, data up to 1-hour stale.
    Free = 0,
    /// Basic tier: 100 queries/ledger, max 30-second fresh data.
    Basic = 1,
    /// Premium tier: unlimited queries/ledger, real-time data.
    Premium = 2,
}

/// Per-consumer tier, quota, and subscription state.
///
/// Stored under [`DataKey::ConsumerInfo`] keyed by consumer address.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct ConsumerInfo {
    /// Current access tier.
    pub tier: ConsumerTier,
    /// Ledger-based subscription expiration. 0 = no active subscription.
    pub subscription_expiry_ledger: u32,
    /// Unix timestamp-based subscription expiration. 0 = no active subscription.
    pub subscription_expiry_ts: u64,
    /// Number of queries consumed in the current ledger.
    pub queries_this_ledger: u32,
    /// Ledger sequence for which `queries_this_ledger` was last reset.
    pub quota_reset_ledger: u32,
}

// =============================================================================
// #174 — On-Chain Alert Subscriptions
// =============================================================================

/// An alert subscription record.
///
/// Stored under [`DataKey::AlertSubscription`] keyed by `(consumer, asset)`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AlertSubscription {
    /// Address of the subscriber (consumer contract or account).
    pub consumer: Address,
    /// Asset being monitored.
    pub asset: Address,
    /// Price movement threshold in basis points (100 bps = 1%) that triggers an alert.
    pub threshold_bps: u32,
    /// Contract address to invoke when the threshold is breached.
    pub callback_contract: Address,
    /// Function selector (Symbol) on the callback contract to call.
    pub callback_fn: Symbol,
    /// Ledger at which this subscription was created or last renewed.
    pub created_ledger: u32,
    /// TTL in ledgers; subscription expires at `created_ledger + ttl`.
    pub ttl_ledgers: u32,
}

// =============================================================================
// Off-chain relayer network integration
// =============================================================================

/// Metadata stored for each admin-approved relayer.
///
/// Stored under [`DataKey::ApprovedRelayer`] keyed by relayer address.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RelayerInfo {
    /// Human-readable display name for this relayer.
    pub name: String,
    /// Ledger sequence number when the relayer was approved by the admin.
    pub approved_at_ledger: u32,
}

/// A single (source, asset, price, timestamp) leg of a batch relayed submission (#264).
///
/// Each leg is independently authorized by its `source` — the relayer bundles one
/// pre-signed authorization entry per leg alongside its own signature. `priority_fee`
/// implements the relayer fee market (#266): legs must be ordered by non-increasing
/// `priority_fee` within the batch, and the source's signature covers this exact value,
/// so a relayer cannot alter it after the fact without invalidating the signature.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RelayedSubmission {
    /// Address of the oracle source whose price is being relayed.
    pub source: Address,
    /// Contract address of the asset being priced.
    pub asset: Address,
    /// Raw price value scaled by `10^decimals`. Must be > 0.
    pub price: i128,
    /// Unix timestamp (seconds) of the price observation.
    pub timestamp: u64,
    /// Priority fee (in stroops) the source is willing to pay for prioritized processing.
    pub priority_fee: u128,
}

// =============================================================================
// #265 — Relayer Performance Bonds
// =============================================================================

/// Reasons a relayer failure incident may be recorded, used as slash grounds (#265).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum RelayerFailureReason {
    /// Relayer submitted a price on behalf of a source without valid authorization.
    UnauthorizedPrice = 0,
    /// Relayer submitted an otherwise invalid/rejected price.
    InvalidSubmission = 1,
    /// Any other operator-attested misbehavior.
    Other = 2,
}

// =============================================================================
// #267 — Relayer Dashboard
// =============================================================================

/// Per-asset submission breakdown for a relayer, part of [`RelayerDashboard`] (#267).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RelayerAssetStat {
    /// Asset contract address.
    pub asset: Address,
    /// Number of successful submissions relayed for this asset.
    pub submissions: u64,
}

/// Aggregated operational dashboard for a relayer (#267).
///
/// Returned by `get_relayer_dashboard`. Combines volume, accuracy, latency, fee/reward
/// earnings, and a comparative percentile rank against every other approved relayer.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RelayerDashboard {
    /// The relayer this dashboard describes.
    pub relayer: Address,
    /// Total successful relayed submissions (single + batch legs).
    pub total_submissions: u64,
    /// Total reported failure incidents (see [`RelayerFailureReason`]).
    pub failed_submissions: u32,
    /// Success rate in basis points: `10_000 * successful / (successful + failed)`.
    pub success_rate_bps: u32,
    /// Estimated submissions per day, averaged over the relayer's approved lifetime.
    pub submissions_per_day: u64,
    /// Average latency in seconds between observation timestamp and ledger close time.
    pub avg_latency_seconds: u64,
    /// Accumulated flat + priority fee earnings (in stroops).
    pub fee_earnings: i128,
    /// Accumulated accuracy-weighted reward earnings (in stroops).
    pub reward_earnings: i128,
    /// Currently deposited performance bond (in stroops).
    pub bond_deposited: i128,
    /// Percentile rank (0-100) of `total_submissions` among all approved relayers.
    pub percentile_rank: u32,
    /// Per-asset submission breakdown.
    pub per_asset: Vec<RelayerAssetStat>,
}

// =============================================================================
// Cross-reference oracle checks
// =============================================================================

/// A registered external oracle contract used for cross-reference price checks.
///
/// Stored under [`DataKey::ReferenceOracle`] keyed by the oracle's contract address.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct ReferenceOracleEntry {
    /// Contract address of the external oracle.
    pub contract_id: Address,
    /// Maps our asset addresses to the corresponding asset addresses used by the reference oracle.
    pub asset_mapping: Map<Address, Address>,
}

/// The result of a cross-reference price comparison for a single asset.
///
/// Returned by `get_cross_reference`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CrossReferenceResult {
    /// Our current aggregated price for the asset.
    pub our_price: i128,
    /// Price reported by the reference oracle for the same asset.
    pub ref_price: i128,
    /// Absolute deviation between the two prices expressed in basis points (1 % = 100 bps).
    pub deviation_bps: u32,
    /// Contract address of the reference oracle that provided `ref_price`.
    pub ref_contract: Address,
}

// =============================================================================
// #210 — Progressive Disqualification / Demerits System
// =============================================================================

/// Progressive disqualification status of an oracle source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum DisqualificationStatus {
    Active = 0,
    Warning = 1,
    Probation = 2,
    Disqualified = 3,
}

/// State tracking demerits and progressive disqualification status for a source.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SourceDemeritState {
    pub demerits: u32,
    pub status: DisqualificationStatus,
    pub status_updated_ledger: u32,
}

/// Configurations for progressive disqualification thresholds.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct DemeritConfig {
    pub warning_threshold: u32,
    pub probation_threshold: u32,
    pub disqualified_threshold: u32,
    pub cooldown_ledgers: u32,
}

// =============================================================================
// #207 — Multi-sig Source Registration Governance
// =============================================================================

/// Configuration for source registration multi-sig governance.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SourceGovernance {
    pub approvers: Vec<Address>,
    pub threshold: u32,
}

/// A proposal to register a new source.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SourceProposal {
    pub id: u32,
    pub source: Address,
    pub name: String,
    pub approvals: Vec<Address>,
    pub executed: bool,
}

// =============================================================================
// #208 — Source Geolocation & Decentralization Metrics
// =============================================================================

/// Geolocation and provider tags for an oracle source.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SourceGeoMetadata {
    pub region: String,
    pub provider: String,
    pub jurisdiction: String,
}

/// Decentralization and concentration report for registered sources.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct DecentralizationReport {
    pub region_hhi: u32,
    pub provider_hhi: u32,
    pub jurisdiction_hhi: u32,
    pub overall_score: u32,
}

// =============================================================================
// Missing types required by feature modules
// =============================================================================

/// Admin operation type discriminant for operation limiting (#238).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum AdminOperationType {
    AddSource = 0,
    RemoveSource = 1,
    RegisterAsset = 2,
    UnregisterAsset = 3,
    SetDecimals = 4,
    SetResolution = 5,
}

/// Per-operation-type daily limit configuration (#238).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AdminOpLimit {
    pub daily_limit: u32,
    /// Ledger when this limit config was set.
    pub set_ledger: u32,
}

/// AMM pool data for the constant-product oracle AMM (#180).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AmmPool {
    /// Address of token X in the pool.
    pub asset_x: Address,
    /// Address of token Y in the pool.
    pub asset_y: Address,
    /// Reserve of token X in the pool (scaled).
    pub reserve_x: u128,
    /// Reserve of token Y in the pool (scaled).
    pub reserve_y: u128,
    /// Constant product k = reserve_x * reserve_y.
    pub k: u128,
    /// Whether the pool is currently accepting swaps.
    pub enabled: bool,
    /// Fee in basis points applied to each swap (e.g., 30 = 0.3%).
    pub fee_bps: u32,
}

/// A price challenge record (#235).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Challenge {
    /// Unique challenge identifier.
    pub id: u32,
    /// Asset being challenged.
    pub asset: Address,
    /// Address of the challenger.
    pub challenger: Address,
    /// Challenger's claimed correct price.
    pub expected_price: i128,
    /// Arbitrary proof bytes.
    pub proof_data: Bytes,
    /// Ledger when the challenge was submitted.
    pub challenged_ledger: u32,
    /// Whether the challenge has been resolved.
    pub is_resolved: bool,
    /// Whether the challenge was deemed valid upon resolution.
    pub is_valid: bool,
    /// Reward amount credited on valid resolution.
    pub reward_amount: i128,
}

/// Price event payload for cross-chain relay (#182).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceEventPayload {
    /// Aggregated price.
    pub price: i128,
    /// Unix timestamp of the price.
    pub timestamp: u64,
    /// Ledger sequence at which the event was recorded.
    pub ledger_sequence: u32,
}

/// Minimal Stellar ledger header fields needed for cross-chain light-client verification (#182).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StellarHeader {
    /// Ledger sequence number.
    pub ledger_sequence: u32,
    /// 32-byte hash of the transaction set.
    pub tx_set_hash: BytesN<32>,
    /// 32-byte hash of the bucket list.
    pub bucket_list_hash: BytesN<32>,
    /// Expected header digest for consistency check.
    pub expected_hash: BytesN<32>,
}

/// Asset pricing type for exotic assets (#177).
/// Each variant carries the data needed to compute the asset's fair value.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum AssetType {
    /// Directly priced asset (standard oracle feed).
    Direct,
    /// LP token: (reserve0, reserve1, total_supply).
    LPToken(u128, u128, u128),
    /// Basket/index: (component addresses, weights).
    Index(Vec<Address>, Vec<u32>),
    /// Options contract: (underlying asset, strike, expiry_timestamp, is_call).
    Option(Address, u128, u64, bool),
}

/// Configuration for an exotic asset's fair-value pricing (#177).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AssetPricingConfig {
    /// Category of the exotic asset.
    pub asset_type: AssetType,
    /// Whether this configuration is enabled.
    pub enabled: bool,
    /// Volatility in basis points (used for option pricing).
    pub volatility_bps: u32,
}

/// Fee market submission entry (#176).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct FeeMarketSubmission {
    /// Submitting oracle source address.
    pub source: Address,
    /// Asset being priced.
    pub asset: Address,
    /// Submitted price.
    pub price: i128,
    /// Submitted timestamp.
    pub timestamp: u64,
    /// Priority fee attached (in stroops-equivalent).
    pub priority_fee: u128,
    /// Ledger when submitted.
    pub submitted_ledger: u32,
}

/// Ordered pending-submission queue for the fee market (#176).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PendingFeeSubmissions {
    /// Submissions ordered by (priority_fee DESC, timestamp ASC).
    pub submissions: Vec<FeeMarketSubmission>,
}

/// Multi-sig governance operation (#178).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct MultiSigOperation {
    /// Unique sequential ID.
    pub id: u32,
    /// Operation type (symbol or discriminant).
    pub op_type: soroban_sdk::Symbol,
    /// Encoded payload.
    pub data: Bytes,
    /// Addresses that have approved this operation.
    pub approvals: Vec<Address>,
    /// Number of approvals required to reach quorum.
    pub required_approvals: u32,
    /// Ledger when the operation was first proposed.
    pub proposed_ledger: u32,
    /// Address of the proposer.
    pub proposed_by: Address,
    /// Whether the operation has been executed.
    pub executed: bool,
    /// Ledger when quorum was reached and timelock started (0 = not yet).
    pub timelock_start_ledger: u32,
    /// Linked-list next pointer (0 = tail).
    pub next_op_id: u32,
}

/// A guardian-initiated admin key recovery process (#245).
///
/// Stored under [`DataKey::PendingRecovery`] while a recovery is in flight.
/// Removed once cancelled by the admin or executed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct GuardianRecovery {
    /// Address that guardians are proposing to install as the new admin.
    pub new_admin: Address,
    /// Guardians that have approved this recovery so far.
    pub approvals: Vec<Address>,
    /// Ledger sequence number when the initiating guardian first proposed this recovery.
    pub initiated_ledger: u32,
    /// Ledger sequence number when the N-of-M guardian threshold was reached and the
    /// cancellation-window delay started. `0` means the threshold has not been reached yet.
    pub ready_ledger: u32,
}

/// Per-asset decimal precision configuration (#227).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AssetDecimalConfig {
    /// Number of decimal places for this asset.
    pub decimals: u32,
    /// Whether this configuration is active.
    pub enabled: bool,
    /// Ledger when this config was set.
    pub set_ledger: u32,
}

/// Source rotation schedule for an asset (#206).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SourceRotationSchedule {
    /// Ledger interval between rotations.
    pub rotation_interval: u32,
    /// Ledger of the next scheduled rotation.
    pub next_rotation_ledger: u32,
    /// Overlap period in ledgers (old+new sources both active during transition).
    pub overlap_period: u32,
    /// Whether rotation is currently enabled.
    pub enabled: bool,
}

/// Off-chain state channel for high-frequency updates (#179).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StateChannel {
    /// Address of the oracle source that opened the channel.
    pub source: Address,
    /// Deposit amount locked in the channel (stroops).
    pub deposit: i128,
    /// Current nonce (highest processed batch sequence number).
    pub nonce: u64,
    /// Last confirmed price via the channel.
    pub last_price: u128,
    /// Last confirmed timestamp via the channel.
    pub last_timestamp: u64,
    /// Unix timestamp after which a dispute can be raised.
    pub dispute_timeout: u64,
    /// Whether the channel is closed.
    pub is_closed: bool,
    /// XLM/token contract address used for the deposit.
    pub token_contract: Address,
}

/// Aggregation round metadata for VDF sampler (#VDF).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AggregationRound {
    /// Round sequence number.
    pub round_id: u32,
    /// Ledger when this round started.
    pub start_ledger: u32,
    /// Ledger when this round ended (0 if in progress).
    pub end_ledger: u32,
    /// Number of submissions in this round.
    pub submission_count: u32,
    /// Aggregated price for this round.
    pub aggregate_price: i128,
}

/// Groth16 verifying key for ZK proof verification (#175).
/// Fields are stored as flat byte arrays to avoid BN254 point type complexity.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Groth16VerifyingKey {
    /// Number of IC points (= public_inputs + 1).
    pub ic_len: u32,
    /// Flat concatenation of IC points (64 bytes each, x-coord || y-coord).
    pub ic_bytes: Bytes,
    /// Pre-computed pairing bytes for the verification equation.
    pub pairing_precomp: Bytes,
}

/// Groth16 ZK proof for off-chain price attestation (#175).
/// Points stored as flat byte arrays: 64 bytes each for A/C, 128 bytes for B.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Groth16Proof {
    /// A point (64 bytes).
    pub a: Bytes,
    /// B point (128 bytes).
    pub b: Bytes,
    /// C point (64 bytes).
    pub c: Bytes,
    /// Fiat-Shamir verification tag (32 bytes).
    pub fs_check: BytesN<32>,
}

/// ZK-verified price attestation (#175).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct ZkPriceAttestation {
    /// Asset address.
    pub asset: Address,
    /// Attested price.
    pub price: i128,
    /// Unix timestamp.
    pub timestamp: u64,
    /// Public signals for the proof.
    pub public_signals: Vec<BytesN<32>>,
    /// The Groth16 proof.
    pub proof: Groth16Proof,
}

/// Snapshot of core global oracle configuration parameters.
///
/// Captured before each successful core-parameter mutation so an admin can
/// roll back to a known-good state via `rollback_config`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct ConfigSnapshot {
    /// Monotonically increasing snapshot version (never reused).
    pub version: u32,
    /// Ledger sequence when this snapshot was taken.
    pub ledger: u32,
    /// Unix timestamp when this snapshot was taken.
    pub timestamp: u64,
    /// Minimum contributing sources required for aggregation.
    pub min_sources_required: u32,
    /// Maximum retained price-history entries per asset.
    pub max_history_length: u32,
    /// SEP-40 resolution window in seconds.
    pub resolution: u32,
    /// Global decimal precision.
    pub decimals: u32,
    /// Human-readable oracle description.
    pub description: String,
    /// Aggregation method discriminant.
    pub aggregation_method: u32,
    /// Maximum allowed submission timestamp skew in seconds.
    pub timestamp_threshold: u64,
    /// Maximum allowed price deviation in basis points.
    pub max_price_deviation: u32,
    /// Circuit-breaker trip threshold in basis points.
    pub circuit_breaker_threshold: u32,
    /// Heartbeat interval in seconds.
    pub heartbeat_interval: u64,
    /// Per-asset history cap.
    pub max_history_per_asset: u32,
    /// Maximum events emitted per call.
    pub max_events_per_call: u32,
    /// Maximum sources used during aggregation (`0` = unlimited).
    pub max_aggregation_sources: u32,
    /// Aggregation cooldown in ledgers.
    pub aggregation_cooldown: u32,
    /// Minimum submission interval in ledgers.
    pub min_submission_interval: u32,
    /// Whether historical price interpolation is enabled.
    pub interpolation_enabled: bool,
    /// Maximum registered oracle sources (`0` = unlimited).
    pub max_sources: u32,
    /// Query rate limit per ledger.
    pub query_rate_limit: u32,
    /// Maximum registered assets.
    pub max_assets: u32,
    /// Whether the contract is paused.
    pub paused: bool,
    /// Timelock delay in ledgers.
    pub timelock_duration: u32,
}

/// Audit log entry for admin actions (#239).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AuditEntry {
    /// Sequential log entry ID.
    pub id: u32,
    /// Short action symbol (e.g., "add_src").
    pub action: Symbol,
    /// Admin address that performed the action.
    pub admin: Address,
    /// Arbitrary action data.
    pub data: Bytes,
    /// Ledger sequence number of this entry.
    pub ledger: u32,
    /// Unix timestamp of this entry.
    pub timestamp: u64,
    /// SHA-256 hash of the previous entry (hash chain).
    pub previous_hash: Bytes,
    /// SHA-256 hash of this entry (for chain validation).
    pub current_hash: Bytes,
}

/// RBAC role discriminant (#241).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum Role {
    /// Manage oracle sources.
    SourceManager = 0,
    /// Register/unregister assets.
    AssetManager = 1,
    /// Submit and override prices.
    PriceUpdater = 2,
    /// Modify configuration settings.
    ConfigManager = 3,
    /// Perform upgrades and admin transfers.
    UpgradeManager = 4,
}

/// Emergency pause state (#240).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct EmergencyPause {
    /// Admin who triggered the pause.
    pub initiated_by: Address,
    /// Reason for the emergency pause.
    pub reason: String,
    /// Ledger when the pause was triggered.
    pub initiated_ledger: u32,
    /// Ledger after which the pause automatically expires (0 = manual only).
    pub auto_unpause_ledger: u32,
}

/// Configuration for cross-chain relay (#182).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CrossChainRelayConfig {
    /// Whether relay is enabled.
    pub enabled: bool,
    /// Quorum threshold percentage (e.g., 67 means 2/3 of validators).
    pub quorum_threshold_pct: u32,
    /// Bit-vector encoding the Merkle path direction for proof verification.
    pub merkle_path_bits: u32,
}

/// A price observation for an asset fetched from the same oracle on another chain (#226).
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct CrossChainPriceEntry {
    /// Raw price value scaled by `10^decimals`.
    pub price: i128,
    /// Decimal precision of `price`.
    pub decimals: u32,
    /// Identifier of the source chain (e.g. `"ethereum"`).
    pub chain_id: String,
    /// Local ledger sequence number when this observation was recorded.
    pub ledger: u32,
    /// Unix timestamp of the observation on the source chain.
    pub timestamp: u64,
}

/// A batch item for state channel high-frequency updates.
/// Each item carries a price update with strict nonce ordering.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct BatchItem {
    /// Monotonically increasing nonce (must exceed channel's current nonce).
    pub nonce: u64,
    /// Price value (scaled by decimals).
    pub price: u128,
    /// Unix timestamp of the price observation.
    pub timestamp: u64,
}

// =============================================================================
// History Export (#export-history)
// =============================================================================

/// A single exported price history entry for off-chain archiving.
///
/// Mirrors [`PriceHistoryEntry`] but includes the asset address so the export
/// bundle is self-contained.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct ExportedEntry {
    /// Asset address.
    pub asset: Address,
    /// Aggregated price scaled by `10^decimals`.
    pub price: i128,
    /// Unix timestamp of the snapshot.
    pub timestamp: u64,
    /// Ledger sequence number of the snapshot.
    pub ledger: u32,
    /// Number of sources that contributed.
    pub num_sources: u32,
    /// Whether this entry was produced by interpolation.
    pub is_interpolated: bool,
}

/// Result returned by `export_history`.
///
/// Contains the page of entries, a simple XOR-based data hash over all included
/// entries (for quick integrity verification off-chain), and pagination state.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct ExportedHistorySnapshot {
    /// The exported price history entries (up to `limit` entries).
    pub entries: Vec<ExportedEntry>,
    /// Simple XOR-fold hash of all entry prices — a lightweight integrity token.
    /// Off-chain archivers can recompute this from the entries and compare.
    pub data_hash: u64,
    /// Ledger of the first entry in this page (0 when empty).
    pub from_ledger: u32,
    /// Ledger of the last entry in this page (0 when empty).
    pub to_ledger: u32,
    /// Total number of recorded ledgers available for this asset.
    pub total_available: u32,
    /// Cursor to pass as `from_ledger` to fetch the next page (`0` when no more pages).
    pub next_cursor: u32,
}

// =============================================================================
// Timelock Priority Queues
// =============================================================================

/// Priority tier for a timelock operation.
///
/// Each tier has its own configurable delay (in ledgers).  Lower discriminant
/// values represent more urgent tiers so that numeric comparisons are intuitive.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum OperationPriority {
    /// Immediate-ish execution with multi-sig co-sign requirement.
    /// Default delay: 1 ledger.
    Urgent = 0,
    /// Standard governance delay.
    /// Default delay: 10 ledgers (matches the pre-existing default).
    Normal = 1,
    /// Extended delay for critical, protocol-level changes.
    /// Default delay: 100 ledgers.
    LongTerm = 2,
}

// =============================================================================
// Batch Dry-Run Simulation
// =============================================================================

/// Warning flags that may be set on a simulated operation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum SimulationWarning {
    /// No warnings.
    None = 0,
    /// The operation would change a parameter to an extreme value.
    ExtremeValue = 1,
    /// The operation would set min_sources below 2, weakening security.
    LowMinSources = 2,
    /// The operation would set max_history to a very large value.
    LargeHistory = 3,
    /// The operation type is unrecognised in the simulator.
    UnknownOpType = 4,
    /// The operation data is too short to decode.
    InvalidData = 5,
}

/// Result for a single operation in a simulated batch.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct OperationSimulationResult {
    /// Zero-based index of this operation in the batch.
    pub index: u32,
    /// Numeric `op_type` discriminant (mirrors [`OperationType`]).
    pub op_type: u32,
    /// Human-readable description of what the operation would change.
    pub description: String,
    /// `true` when the operation would succeed given current contract state.
    pub would_succeed: bool,
    /// Any warnings raised by the simulation.
    pub warning: SimulationWarning,
}

/// Aggregate result of `simulate_batch`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct BatchSimulationResult {
    /// Per-operation simulation results.
    pub results: Vec<OperationSimulationResult>,
    /// Total number of operations in the batch.
    pub total_ops: u32,
    /// Number of operations that would succeed.
    pub would_succeed_count: u32,
    /// Number of operations that raised at least one warning.
    pub warning_count: u32,
    /// `true` when *all* operations would succeed (the batch is safe to submit).
    pub all_succeed: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum OperationStatus {
    Pending,
    Executed,
    Cancelled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct Operation {
    pub id: String,
    pub status: OperationStatus,
    pub depends_on: Vec<String>,
}

// ---- Pending operation types ----

/// The kind of administrative action captured in a pending operation.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum OperationKind {
    AddSource,
    RemoveSource,
    RegisterAsset,
    UnregisterAsset,
    SetMinSources,
    SetMaxHistory,
    SetDecimals,
    SetDescription,
}

/// A pending operation waiting to be executed or expired.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PendingOperation {
    /// Unique monotonic id (ledger sequence at creation).
    pub id: u64,
    pub kind: OperationKind,
    /// JSON-style serialized args stored as a String for simplicity.
    pub args: String,
    /// Ledger sequence at which this operation was created.
    pub created_at_ledger: u32,
    /// Ledger sequence after which this operation is expired and unexecutable.
    pub expires_at_ledger: u32,
    /// Whether this operation has been executed already.
    pub executed: bool,
}

// ---- Template registry types ----

/// A single parameterized step inside a template.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct TemplateStep {
    pub kind: OperationKind,
    /// Human-readable description of this step.
    pub description: String,
}

/// A named, reusable sequence of operation steps.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct OperationTemplate {
    pub name: Symbol,
    pub description: String,
    pub steps: Vec<TemplateStep>,
    pub created_at_ledger: u32,
}

// =============================================================================
// #278 — Contract State Introspection
// =============================================================================

/// Serializable contract configuration snapshot for `oracle-state-dump`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StateDump {
    pub admin: Address,
    pub description: String,
    pub min_sources_required: u32,
    pub max_history_length: u32,
    pub decimals: u32,
    pub resolution: u32,
    pub timestamp_threshold: u64,
    pub max_deviation_bps: u32,
    pub heartbeat_interval: u64,
}

/// Statistics computed from live contract state for `oracle-state-analyze`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StateAnalysis {
    pub admin: Address,
    pub decimals: u32,
    pub min_sources_required: u32,
    pub max_history_length: u32,
    pub registered_assets: u32,
    pub registered_sources: u32,
    pub aggregate_count: u32,
    pub history_depth_avg: u32,
}

/// Field-level diff between two contract snapshots.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StateDiffEntry {
    pub field: String,
    pub left: String,
    pub right: String,
}

/// Top-level diff container returned by `oracle-state-diff`.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StateDiff {
    pub contract_a: String,
    pub contract_b: String,
    pub entries: Vec<StateDiffEntry>,
}

// =============================================================================
// #280 — Stellar DEX Integration
// =============================================================================

/// A price observation read from a Stellar DEX liquidity pool.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct DexPrice {
    pub asset: Address,
    pub price: i128,
    pub reserve_x: i128,
    pub reserve_y: i128,
    pub timestamp: u64,
}

// =============================================================================
// #281 — Soroswap / AMM Integration
// =============================================================================

/// AMM pool weight configuration for aggregation inclusion.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AmmWeightConfig {
    pub asset: Address,
    pub weight_bps: u32,
    pub enabled: bool,
}

/// Soroswap pool metadata used to derive a price feed.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SoroswapPool {
    pub asset_a: Address,
    pub asset_b: Address,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub fee_bps: u32,
    pub enabled: bool,
}

// =============================================================================
// Severity-aware alerting
// =============================================================================

/// Anomaly severity classification for a price-deviation alert.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum AlertSeverity {
    /// Movement below the warning threshold — informational only.
    Info = 0,
    /// Movement exceeds the warning threshold — routed to the dashboard.
    Warning = 1,
    /// Movement exceeds the critical threshold — routed to the paging channel.
    Critical = 2,
    /// Movement exceeds the emergency threshold — routed to the paging channel
    /// with the highest urgency.
    Emergency = 3,
}

/// Notification channel an alert is routed to based on its classified severity.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub enum AlertChannel {
    /// Non-urgent — surfaced on the monitoring dashboard only.
    Dashboard = 0,
    /// Urgent — routed to an on-call paging channel.
    Page = 1,
}

/// Basis-point movement thresholds used to classify an anomaly's severity.
///
/// Must satisfy `warning_bps < critical_bps < emergency_bps`; all three greater
/// than zero.
#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct SeverityThresholds {
    /// Movement at/above this level (bps) is at least `Warning`.
    pub warning_bps: u32,
    /// Movement at/above this level (bps) is at least `Critical`.
    pub critical_bps: u32,
    /// Movement at/above this level (bps) is `Emergency`.
    pub emergency_bps: u32,
}

