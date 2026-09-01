//! Stellar Unified Price Oracle Rust SDK
//!
//! A high-level, user-friendly wrapper around the `PriceOracleContractClient`
//! that provides convenient methods for querying prices, submitting data, and
//! managing governance operations.

pub mod client;
pub mod error;
pub mod types;

pub use client::PriceOracleClient;
pub use error::{ErrorCode, SdkError};
pub use types::*;

#[cfg(test)]
mod tests;
