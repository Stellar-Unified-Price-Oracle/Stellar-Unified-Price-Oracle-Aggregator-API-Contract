use soroban_sdk::{contracttype, Address, Map, String, Symbol, Vec};

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
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PriceEntry {
    pub price: i128,
    pub timestamp: u64,
    pub source: Address,
    pub decimals: u32,
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
pub struct PriceData {
    pub price: i128,
    pub timestamp: u64,
}
