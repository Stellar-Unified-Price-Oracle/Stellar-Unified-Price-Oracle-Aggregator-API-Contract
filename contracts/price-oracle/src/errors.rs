use soroban_sdk::contracterror;

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
