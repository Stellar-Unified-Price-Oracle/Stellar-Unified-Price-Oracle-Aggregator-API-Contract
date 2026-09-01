//! Type definitions for the Rust SDK

use std::fmt;

/// Configuration for the Price Oracle
#[derive(Debug, Clone)]
pub struct Config {
    pub min_sources: u32,
    pub max_history: u32,
    pub decimals: u32,
    pub resolution: u32,
    pub timestamp_threshold: u64,
}

/// A price submission
#[derive(Debug, Clone)]
pub struct PriceSubmission {
    pub asset: String,
    pub price: i128,
    pub timestamp: u64,
}

/// A price result
#[derive(Debug, Clone)]
pub struct Price {
    pub asset: String,
    pub price: i128,
    pub timestamp: u64,
    pub decimals: u32,
}

/// Admin action proposal
#[derive(Debug, Clone)]
pub struct AdminActionProposal {
    pub proposal_id: u32,
    pub action: String,
    pub proposed_ledger: u32,
    pub priority_delay: u32,
}

/// Transaction result
#[derive(Debug, Clone)]
pub struct TransactionResult {
    pub transaction_hash: String,
    pub ledger: u32,
    pub success: bool,
}

/// Event types from the contract
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventType {
    PriceUpdated,
    ConfigChanged,
    AdminActionProposed,
    AdminActionExecuted,
    AdminActionCancelled,
}

/// Contract event
#[derive(Debug, Clone)]
pub struct ContractEvent {
    pub event_type: EventType,
    pub ledger: u32,
    pub data: String,
}

/// Aggregation method for prices
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregationMethod {
    Mean,
    Median,
    Mode,
}

/// Operation priority for timelock
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationPriority {
    Urgent = 0,
    Normal = 1,
    LongTerm = 2,
}

impl fmt::Display for OperationPriority {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationPriority::Urgent => write!(f, "Urgent"),
            OperationPriority::Normal => write!(f, "Normal"),
            OperationPriority::LongTerm => write!(f, "LongTerm"),
        }
    }
}

/// Operation type for timelock
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

impl fmt::Display for OperationType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OperationType::Upgrade => write!(f, "Upgrade"),
            OperationType::SetAdmin => write!(f, "SetAdmin"),
            OperationType::SetMinSources => write!(f, "SetMinSources"),
            OperationType::SetMaxHistory => write!(f, "SetMaxHistory"),
            OperationType::SetResolution => write!(f, "SetResolution"),
            OperationType::SetDecimals => write!(f, "SetDecimals"),
            OperationType::SetDescription => write!(f, "SetDescription"),
            OperationType::SetTimestampThreshold => write!(f, "SetTimestampThreshold"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_debug_display() {
        let config = Config {
            min_sources: 3,
            max_history: 1000,
            decimals: 8,
            resolution: 60,
            timestamp_threshold: 300,
        };
        assert_eq!(config.min_sources, 3);
    }

    #[test]
    fn price_creation() {
        let price = Price {
            asset: "EURUSD".to_string(),
            price: 150_000_000,
            timestamp: 1234567890,
            decimals: 8,
        };
        assert_eq!(price.asset, "EURUSD");
        assert_eq!(price.decimals, 8);
    }
}
