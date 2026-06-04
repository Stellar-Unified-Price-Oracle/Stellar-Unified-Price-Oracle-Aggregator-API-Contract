use soroban_sdk::{contracttype, contracterror, Address, Map, String, Symbol, Vec};

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub enum DataKey {
    Admin,
    Source(Address),
    AssetRegistered(Address),
    Submission(Address, Address),
    Aggregate(Address),
    PriceHistory(Address, u32),
    OracleSources,
    RegisteredAssets,
    MinSourcesRequired,
    MaxHistoryLength,
    Resolution,
    Decimals,
    Description,
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

#[contracterror]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ErrorCode {
    NotAuthorized = 0,
    AlreadyInitialized = 1,
    AssetNotRegistered = 2,
    AssetAlreadyRegistered = 3,
    SourceAlreadyExists = 4,
    SourceNotFound = 5,
    InsufficientSources = 6,
    InvalidPrice = 7,
    NoData = 8,
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
