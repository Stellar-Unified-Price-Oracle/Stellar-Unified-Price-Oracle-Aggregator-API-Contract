use soroban_sdk::{contracttype, Address, Bytes, Map, String, Symbol, Vec};

pub use crate::errors::ErrorCode;

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum DataKey {
    Admin,
    Source(Address),
    AssetRegistered(Address),
    Submission(Address, Address),
    SubmissionLedger(Address, Address),
    Aggregate(Address),
    PriceHistory(Address, u32),
    PriceHistoryLedgers(Address),
    OracleSources,
    RegisteredAssets,
    MinSourcesRequired,
    MaxHistoryLength,
    Resolution,
    Decimals,
    Description,
    TimestampThreshold,
    MaxPriceDeviation,
    SubmissionDeviant(Address, Address),
    SourceHeartbeat(Address),
    HeartbeatInterval,
    InactiveSource(Address),
    MaxInvalidSubmissions,
    AggregationMethod,
    AssetMetadata(Address),
    AssetMinPrice(Address),
    PauseFlag,
    PendingOpCount,
    PendingOp(u32),
    TimelockDuration,
    ReentrancyGuard,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceEntry {
    pub price: i128,
    pub timestamp: u64,
    pub source: Address,
    pub decimals: u32,
    pub last_updated: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AggregatePrice {
    pub price: i128,
    pub timestamp: u64,
    pub num_sources: u32,
    pub decimals: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceHistoryEntry {
    pub price: i128,
    pub timestamp: u64,
    pub ledger: u32,
    pub num_sources: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct OracleSources {
    pub sources: Vec<Address>,
    pub metadata: Map<Address, String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum Asset {
    Stellar(Address),
    Other(Symbol),
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum AggregationMethod {
    Median = 0,
    Mean = 1,
    TrimmedMean = 2,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
    pub last_updated: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum OperationType {
    Upgrade = 0,
    SetAdmin = 1,
    SetMinSources = 2,
    SetMaxHistory = 3,
    SetResolution = 4,
    SetDecimals = 5,
    SetDescription = 6,
    SetTimestampThreshold = 7,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PendingOperation {
    pub id: u32,
    pub op_type: OperationType,
    pub proposed_by: Address,
    pub proposed_ledger: u32,
    pub data: Bytes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct AssetMetadata {
    pub name: String,
    pub symbol: String,
    pub decimals: Option<u32>,
}
