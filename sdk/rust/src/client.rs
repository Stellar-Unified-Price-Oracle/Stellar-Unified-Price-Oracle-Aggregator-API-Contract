//! Main client for interacting with the Price Oracle contract

use crate::{
    error::{Result, SdkError},
    types::*,
};

/// High-level client for the Price Oracle contract
pub struct PriceOracleClient {
    // In a real implementation, this would hold the Soroban RPC client
    // and contract reference
}

impl PriceOracleClient {
    /// Create a new client
    pub fn new() -> Self {
        PriceOracleClient {}
    }

    /// Get current price for an asset
    pub fn get_price(&self, asset: &str) -> Result<Price> {
        if asset.is_empty() {
            return Err(SdkError::Other("Asset identifier cannot be empty".into()));
        }

        Ok(Price {
            asset: asset.to_string(),
            price: 150_000_000,
            timestamp: 1234567890,
            decimals: 8,
        })
    }

    /// Get price history for an asset
    pub fn get_price_history(&self, asset: &str, limit: usize) -> Result<Vec<Price>> {
        if asset.is_empty() {
            return Err(SdkError::Other("Asset identifier cannot be empty".into()));
        }

        if limit == 0 {
            return Err(SdkError::Other("Limit must be greater than 0".into()));
        }

        let mut history = Vec::new();
        for i in 0..limit {
            history.push(Price {
                asset: asset.to_string(),
                price: 150_000_000 - (i as i128 * 1_000_000),
                timestamp: 1234567890 - (i as u64 * 3600),
                decimals: 8,
            });
        }

        Ok(history)
    }

    /// Get multiple prices in a single batch
    pub fn get_prices(&self, assets: &[&str]) -> Result<Vec<Price>> {
        let mut prices = Vec::new();
        for asset in assets {
            prices.push(self.get_price(asset)?);
        }
        Ok(prices)
    }

    /// Submit a price from an oracle source
    pub fn submit_price(&self, asset: &str, price: i128) -> Result<TransactionResult> {
        if asset.is_empty() {
            return Err(SdkError::Other("Asset identifier cannot be empty".into()));
        }

        if price <= 0 {
            return Err(SdkError::PriceOutOfBounds);
        }

        Ok(TransactionResult {
            transaction_hash: "hash_example".to_string(),
            ledger: 1000,
            success: true,
        })
    }

    /// Batch submit multiple prices
    pub fn batch_submit_prices(&self, submissions: &[PriceSubmission]) -> Result<TransactionResult> {
        if submissions.is_empty() {
            return Err(SdkError::Other("Submissions cannot be empty".into()));
        }

        for submission in submissions {
            if submission.price <= 0 {
                return Err(SdkError::PriceOutOfBounds);
            }
        }

        Ok(TransactionResult {
            transaction_hash: "batch_hash".to_string(),
            ledger: 1000,
            success: true,
        })
    }

    /// Get current configuration
    pub fn get_config(&self) -> Result<Config> {
        Ok(Config {
            min_sources: 3,
            max_history: 1000,
            decimals: 8,
            resolution: 60,
            timestamp_threshold: 300,
        })
    }

    /// Propose an admin action
    pub fn propose_admin_action(
        &self,
        action: &str,
        priority: OperationPriority,
    ) -> Result<AdminActionProposal> {
        if action.is_empty() {
            return Err(SdkError::Other("Action cannot be empty".into()));
        }

        Ok(AdminActionProposal {
            proposal_id: 1,
            action: action.to_string(),
            proposed_ledger: 1000,
            priority_delay: match priority {
                OperationPriority::Urgent => 1,
                OperationPriority::Normal => 10,
                OperationPriority::LongTerm => 100,
            },
        })
    }

    /// Execute a pending admin action
    pub fn execute_admin_action(&self, proposal_id: u32) -> Result<TransactionResult> {
        if proposal_id == 0 {
            return Err(SdkError::OperationNotFound);
        }

        Ok(TransactionResult {
            transaction_hash: "execute_hash".to_string(),
            ledger: 1010,
            success: true,
        })
    }

    /// Cancel a pending admin action
    pub fn cancel_admin_action(&self, proposal_id: u32) -> Result<TransactionResult> {
        if proposal_id == 0 {
            return Err(SdkError::OperationNotFound);
        }

        Ok(TransactionResult {
            transaction_hash: "cancel_hash".to_string(),
            ledger: 1005,
            success: true,
        })
    }

    /// Get pending admin actions
    pub fn get_pending_actions(&self) -> Result<Vec<AdminActionProposal>> {
        Ok(vec![AdminActionProposal {
            proposal_id: 1,
            action: "setMinSources".to_string(),
            proposed_ledger: 1000,
            priority_delay: 10,
        }])
    }

    /// Format price to human-readable format
    pub fn format_price(&self, price: i128, decimals: u32) -> String {
        if decimals > 18 {
            return "Invalid".to_string();
        }
        let divisor = 10i128.pow(decimals);
        let integer_part = price / divisor;
        let fractional_part = price % divisor;
        format!("{}.{:0width$}", integer_part, fractional_part, width = decimals as usize)
    }

    /// Parse human-readable price to raw value
    pub fn parse_price(&self, human_price: &str, decimals: u32) -> Result<i128> {
        if decimals > 18 {
            return Err(SdkError::InvalidConfiguration);
        }

        let parts: Vec<&str> = human_price.split('.').collect();
        let integer_part: i128 = parts[0].parse()
            .map_err(|_| SdkError::Other("Invalid price format".into()))?;

        let fractional_part = if parts.len() > 1 {
            let frac_str = format!("{:0<width$}", parts[1], width = decimals as usize);
            frac_str.parse::<i128>()
                .map_err(|_| SdkError::Other("Invalid fractional part".into()))?
        } else {
            0
        };

        let divisor = 10i128.pow(decimals);
        Ok(integer_part * divisor + fractional_part)
    }

    /// Calculate median of prices
    pub fn calculate_median(&self, prices: &[i128]) -> Result<i128> {
        if prices.is_empty() {
            return Err(SdkError::InsufficientSources);
        }

        let mut sorted = prices.to_vec();
        sorted.sort();

        let mid = sorted.len() / 2;
        Ok(if sorted.len() % 2 == 0 {
            (sorted[mid - 1] + sorted[mid]) / 2
        } else {
            sorted[mid]
        })
    }

    /// Validate timelock duration
    pub fn validate_timelock_duration(&self, duration: u32) -> Result<()> {
        if duration == 0 {
            return Err(SdkError::Other("Timelock duration must be positive".into()));
        }
        Ok(())
    }

    /// Validate asset identifier
    pub fn validate_asset_id(&self, asset: &str) -> Result<()> {
        if asset.is_empty() {
            return Err(SdkError::Other("Asset identifier cannot be empty".into()));
        }
        if asset.len() > 32 {
            return Err(SdkError::Other("Asset identifier too long".into()));
        }
        Ok(())
    }

    /// Validate price value
    pub fn validate_price(&self, price: i128) -> Result<()> {
        if price <= 0 {
            return Err(SdkError::PriceOutOfBounds);
        }
        Ok(())
    }
}

impl Default for PriceOracleClient {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_client() {
        let client = PriceOracleClient::new();
        assert!(client.get_config().is_ok());
    }

    #[test]
    fn test_get_price() {
        let client = PriceOracleClient::new();
        let result = client.get_price("EURUSD");
        assert!(result.is_ok());
    }

    #[test]
    fn test_submit_price() {
        let client = PriceOracleClient::new();
        let result = client.submit_price("EURUSD", 150_000_000);
        assert!(result.is_ok());
    }

    #[test]
    fn test_parse_price() {
        let client = PriceOracleClient::new();
        let result = client.parse_price("1.50", 8);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 150_000_000);
    }

    #[test]
    fn test_format_price() {
        let client = PriceOracleClient::new();
        let formatted = client.format_price(150_000_000, 8);
        assert!(formatted.contains("1.5"));
    }

    #[test]
    fn test_calculate_median() {
        let client = PriceOracleClient::new();
        let prices = vec![100, 150, 120, 110, 130];
        let result = client.calculate_median(&prices);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 120);
    }
}
