//! Error types for the Rust SDK

use std::fmt;

/// SDK-level errors mapped to contract ErrorCode
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SdkError {
    /// Operation not found in timelock
    OperationNotFound,
    /// Timelock not ready for execution
    TimelockNotReady,
    /// Configuration is invalid
    InvalidConfiguration,
    /// Unauthorized operation
    Unauthorized,
    /// Asset not registered
    AssetNotRegistered,
    /// Price out of bounds
    PriceOutOfBounds,
    /// Not enough sources for consensus
    InsufficientSources,
    /// Generic error with message
    Other(String),
}

/// Error codes matching the contract
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCode {
    AlreadyInitialized = 0,
    Unauthorized = 1,
    OperationNotFound = 2,
    InvalidConfiguration = 3,
    TimelockNotReady = 4,
    AssetNotRegistered = 5,
    PriceOutOfBounds = 6,
    InsufficientSources = 7,
    DescriptionTooLong = 8,
    PriorityTimelockNotReady = 9,
}

impl fmt::Display for SdkError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SdkError::OperationNotFound => write!(f, "Operation not found in timelock"),
            SdkError::TimelockNotReady => write!(f, "Timelock period not elapsed"),
            SdkError::InvalidConfiguration => write!(f, "Configuration value out of bounds"),
            SdkError::Unauthorized => write!(f, "Unauthorized operation"),
            SdkError::AssetNotRegistered => write!(f, "Asset is not registered"),
            SdkError::PriceOutOfBounds => write!(f, "Price exceeds bounds"),
            SdkError::InsufficientSources => write!(f, "Not enough price sources"),
            SdkError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for SdkError {}

pub type Result<T> = std::result::Result<T, SdkError>;
