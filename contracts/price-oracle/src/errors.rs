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
/// | 116–118 | Cross-chain asset registry |
/// | 119–121 | Axelar GMP integration |
/// | 122–125 | LayerZero integration |
/// | 156–157 | #484 bound tiers / #485 deferral |
/// | 159–175 | Storage tiers, auto-renewal, consensus rounds, derived feeds (#246,#289,#397,#478) |
/// | 180–184 | #486 price corrections (renumbered off the 159–162 collision, see below) |
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
    InvalidConfiguration = 9,
    SourceEvidenceMissing = 10,
}
