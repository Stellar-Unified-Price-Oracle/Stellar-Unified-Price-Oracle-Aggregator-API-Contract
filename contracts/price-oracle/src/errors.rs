use soroban_sdk::contracterror;

/// Error codes returned by contract invocations when a precondition is violated.
///
/// Each variant maps to a `u32` discriminant that is embedded in the Soroban
/// host error returned to the caller. Clients should match on these values to
/// present meaningful error messages.
#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    /// The caller is not authorized to perform the requested operation.
    /// Returned when `require_auth()` fails or a non-admin attempts an admin action.
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
    /// A configuration parameter is out of its valid range (e.g. `min_sources = 0`,
    /// `deviation_basis_points > 100_000`, or `heartbeat_interval = 0`).
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
    /// Rate limit exceeded for an operation (e.g., too many price submissions).
    RateLimitExceeded = 16,
    /// The requested subscription plan duration does not exist.
    InvalidDuration = 17,
    /// The consumer's subscription has expired.
    SubscriptionExpired = 18,
    Reentrant = 16,
    /// The `op_type` discriminant passed to `propose_operation` is not in `[0, 7]`.
    InvalidOperationType = 17,
    /// A migration is already in progress; complete or resume it before starting another.
    MigrationInProgress = 18,
    /// No migration is currently in progress.
    NoMigrationInProgress = 19,
}
