use soroban_sdk::contracterror;

/// Error codes returned by contract invocations when a precondition is violated.
///
/// Each variant maps to a `u32` discriminant that is embedded in the Soroban
/// host error returned to the caller. Clients should match on these values to
/// present meaningful error messages.
///
/// **Discriminant registry** (never reuse a number, even after a variant is removed):
///
/// | Range  | Category               |
/// |--------|------------------------|
/// | 0–15   | Core / original        |
/// | 16–19  | Rate-limit & migration |
/// | 20–30  | Source management      |
/// | 31–39  | Commit-reveal (#187)   |
/// | 40–49  | Finality gadget (#188) |
/// | 50–61  | Relayer & misc         |
/// | 62–74  | Asset/price bounds     |
/// | 75–98  | Advanced features      |
/// | 99–101 | Signed submission (#216) |
/// | 102, 116–118 | Freeze/pagination/notify (#223,#229,#243) — 116–118 renumbered off the 99–101 collision |
/// | 103–107 | Relayer batch/bond/fee market (#264,#265,#266) |
/// | 200 | Severity-aware alerting |
#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    // ── 0–15: Core ───────────────────────────────────────────────────────────
    /// The caller is not authorized to perform the requested operation.
    NotAuthorized = 0,
    /// `initialize` was called on a contract that has already been set up.
    AlreadyInitialized = 1,
    /// The specified asset has not been registered with `register_asset`.
    AssetNotRegistered = 2,
    /// `register_asset` was called for an asset that is already registered.
    AssetAlreadyRegistered = 3,
    /// `add_source` was called for an address that is already a registered source.
    SourceAlreadyExists = 4,
    /// The referenced oracle source address is not registered.
    SourceNotFound = 5,
    /// Fewer sources have submitted prices than the configured `min_sources_required`.
    InsufficientSources = 6,
    /// A submitted price value is zero or negative.
    InvalidPrice = 7,
    /// No aggregate price data exists for the requested asset.
    NoData = 8,
    /// The submitted timestamp lies too far in the future relative to the current ledger time.
    InvalidTimestamp = 9,
    /// A configuration parameter is out of its valid range.
    InvalidConfiguration = 10,
    /// The description string exceeds the maximum allowed length of 256 characters.
    DescriptionTooLong = 11,
    /// The contract is currently paused; no price submissions or reads are allowed.
    ContractPaused = 12,
    /// A timelock operation cannot yet be executed because its delay period has not elapsed.
    TimelockNotReady = 13,
    /// No pending operation exists with the given ID.
    OperationNotFound = 14,
    /// The submitted price is below the asset's configured minimum price floor.
    PriceBelowMinimum = 15,

    // ── 16–19: Rate-limit, subscription & migration ──────────────────────────
    /// Rate limit exceeded for an operation.
    RateLimitExceeded = 16,
    /// The requested subscription plan duration does not exist.
    InvalidDuration = 17,
    /// The consumer's subscription has expired.
    SubscriptionExpired = 18,
    /// A migration is already in progress.
    MigrationInProgress = 19,

    // ── 20–30: Source management ─────────────────────────────────────────────
    /// No migration is currently in progress.
    NoMigrationInProgress = 20,
    /// The source name is empty.
    SourceNameEmpty = 21,
    /// The source name exceeds the maximum allowed length.
    SourceNameTooLong = 22,
    /// The maximum number of sources has been reached.
    MaxSourcesReached = 23,
    /// The `op_type` discriminant passed to `propose_operation` is not valid.
    InvalidOperationType = 24,
    /// A reentrancy attempt was detected.
    Reentrant = 25,
    /// Source is not pending removal (cancel / finalize called on non-pending source).
    SourceNotPendingRemoval = 26,
    /// The removal cooldown has not elapsed yet.
    CooldownNotElapsed = 27,
    /// Maximum number of registered assets reached.
    MaxAssetsReached = 28,
    /// A reason string is too long.
    ReasonTooLong = 29,
    /// Records limit exceeded for price history query.
    RecordsLimitExceeded = 30,

    // ── 31–39: Commit-reveal (#187) ──────────────────────────────────────────
    /// The revealed hash does not match the committed hash.
    CommitHashMismatch = 31,
    /// The commit has expired (reveal window has closed for this round).
    CommitExpired = 32,
    /// No commit was found for this (source, asset, round).
    CommitNotFound = 33,
    /// The reveal window is closed — too early or too late to reveal.
    RevealWindowClosed = 34,
    /// A commit already exists for this (source, asset, round); cannot double-commit.
    AlreadyCommitted = 35,
    /// The commit round ledger is invalid (e.g., in the future).
    InvalidCommitRound = 36,
    /// Commit-reveal scheme is required (BFT fault tolerance > 0) but was bypassed.
    CommitRevealRequired = 37,

    // ── 40–49: Finality gadget (#188) ────────────────────────────────────────
    /// The price has already been finalized and cannot be changed.
    AlreadyFinalized = 40,
    /// The price was retracted due to a detected reorg.
    PriceRetracted = 41,
    /// The price is still in the finality pending window.
    FinalityPending = 42,
    /// The requested price does not meet the caller's minimum finality requirement.
    InsufficientFinality = 43,
    /// A reorg was detected via ledger hash chain inconsistency.
    ReorgDetected = 44,

    // ── 50–61: Relayer & misc ────────────────────────────────────────────────
    /// The caller is not a registered and approved relayer.
    RelayerNotAuthorized = 50,
    /// `add_relayer` was called for an address that is already an approved relayer.
    RelayerAlreadyExists = 51,
    /// Source reputation is too high to be eligible for slashing.
    ReputationTooHighToSlash = 52,
    /// Maximum number of alert subscriptions has been reached.
    MaxSubscriptionsReached = 53,
    /// The oracle source has been suspended due to progressive disqualification.
    SourceSuspended = 62,
    /// Demerit configuration thresholds are invalid.
    InvalidDemeritThreshold = 63,
    /// Invalid source governance config.
    InvalidGovernanceConfig = 64,
    /// The specified source proposal does not exist.
    ProposalNotFound = 57,
    /// Approver has already approved the proposal.
    AlreadyApproved = 58,
    /// The source proposal was already executed.
    ProposalAlreadyExecuted = 59,
    /// The source has not deposited the required bond.
    InsufficientBond = 60,
    /// The staking/fee token contract has not been configured.
    StakeTokenNotConfigured = 61,

    // ── 62–74: Asset/price bounds & circuit breaker ───────────────────────────
    /// The submitted price falls outside the asset's configured price bounds.
    PriceOutOfBounds = 65,
    /// The asset is currently paused and cannot accept new submissions.
    AssetPaused = 66,
    /// The circuit breaker tripped for the asset and rejected the update.
    CircuitBreakerTripped = 67,

    // ── 75–99: Advanced features ──────────────────────────────────────────────
    /// Daily admin operation limit exceeded.
    OperationLimitExceeded = 75,
    /// An arithmetic overflow occurred in a calculation.
    ArithmeticOverflow = 76,
    /// AMM pool already exists for this asset pair.
    PoolAlreadyExists = 77,
    /// AMM pool not found for this asset pair.
    PoolNotFound = 78,
    /// AMM swap would exceed maximum allowed slippage.
    SlippageExceeded = 79,
    /// AMM price manipulation detected.
    AmmPriceManipulation = 80,
    /// Price challenge submission is outside the valid submission window.
    OutOfSubmissionWindow = 81,
    /// State channel not found for this source.
    ChannelNotFound = 82,
    /// State channel is already open for this source.
    ChannelAlreadyOpen = 83,
    /// Exotic asset cycle depth limit exceeded.
    ExoticCycleLimitExceeded = 84,
    /// Exotic cycle detected in asset dependency graph.
    ExoticCycleDetected = 85,
    /// Exotic asset pricing configuration not found.
    ExoticAssetNotConfigured = 86,
    /// Fee market submission is below the minimum priority fee.
    FeeMarketBelowMinimum = 87,
    /// Multi-sig: approval not found.
    ApprovalNotFound = 88,
    /// Multi-sig: not the queue head.
    MsNotQueueHead = 89,
    /// Multi-sig: quorum not reached.
    MsQuorumNotReached = 90,
    /// Optimistic oracle: bond is too small.
    BondTooSmall = 91,
    /// Optimistic oracle: proposal already disputed.
    ProposalAlreadyDisputed = 92,
    /// Optimistic oracle: proposal already resolved.
    ProposalAlreadyResolved = 93,
    /// Optimistic oracle: proposal has expired.
    ProposalExpired = 94,
    /// Optimistic oracle: proposal not disputed (cannot resolve).
    ProposalNotDisputed = 95,
    /// ZK proof is invalid.
    ZkProofInvalid = 96,
    /// ZK verifying key not set.
    ZkVkNotSet = 97,
    /// ZK invalid public signals.
    ZkInvalidPublicSignals = 98,

    // ── 99–101: Signed price submission (#216) ────────────────────────────────
    /// A pre-signed price proof's `expiration_ledger` has already passed.
    SignatureExpired = 99,
    /// The provided nonce does not exceed the source's last accepted nonce.
    InvalidNonce = 100,
    /// `source` has not registered an Ed25519 key for signed submissions.
    SigningKeyNotRegistered = 101,
    // ── 116–118: Freeze & pagination (renumbered off the 99–101 range, which
    // collided with the Signed price submission block above and would not
    // compile — see the discriminant registry note at the top of this file) ──
    /// The asset's price is currently frozen (#223).
    PriceFrozen = 116,
    /// The asset's price is not currently frozen (#223).
    PriceNotFrozen = 117,
    /// The requested pagination limit is `0` or exceeds the configured maximum (#229).
    InvalidPageSize = 118,
    /// A notification `channel` or `target` string exceeds the maximum allowed length (#243).
    NotificationConfigInvalid = 102,

    // ── 103–109: History export ───────────────────────────────────────────────
    /// The requested export limit is `0` or exceeds the configured maximum.
    ExportLimitExceeded = 103,
    /// No export snapshot was found for the given asset / range.
    ExportNotFound = 104,

    // ── 110–112: Timelock priority queues ────────────────────────────────────
    /// The supplied priority discriminant is not a valid `OperationPriority` value.
    InvalidPriority = 110,
    /// The priority-specific timelock delay has not elapsed for this operation.
    PriorityTimelockNotReady = 111,

    // ── 112–115: Issues #295–#298 ─────────────────────────────────────────────
    /// The submitted external proof failed format validation (#296).
    InvalidProof = 112,
    /// The proof type does not satisfy the asset's configured proof requirement (#296).
    ProofTypeMismatch = 113,
    /// The per-asset callback limit has been reached (#297).
    TooManyCallbacks = 114,
    /// No callback registration found for the given (consumer, asset) pair (#297).
    CallbackNotFound = 115,

    // ── 200: Severity-aware alerting ──────────────────────────────────────────
    /// Severity thresholds are not strictly increasing, or one of them is zero.
    InvalidSeverityThresholds = 200,
}
