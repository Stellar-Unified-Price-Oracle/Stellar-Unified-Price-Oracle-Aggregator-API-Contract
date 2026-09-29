//! Comprehensive tests for the Rust SDK

#[cfg(test)]
mod tests {
    use crate::{client::PriceOracleClient, error::SdkError, types::*};

    fn setup() -> PriceOracleClient {
        PriceOracleClient::new()
    }

    mod query_operations {
        use super::*;

        #[test]
        fn test_get_current_price() {
            let client = setup();
            let result = client.get_price("EURUSD");
            assert!(result.is_ok());
            let price = result.unwrap();
            assert_eq!(price.asset, "EURUSD");
            assert!(price.price > 0);
        }

        #[test]
        fn test_get_price_history() {
            let client = setup();
            let result = client.get_price_history("EURUSD", 10);
            assert!(result.is_ok());
            let history = result.unwrap();
            assert_eq!(history.len(), 10);
        }

        #[test]
        fn test_get_multiple_prices() {
            let client = setup();
            let assets = vec!["EURUSD", "GBPUSD", "JPYUSD"];
            let result = client.get_prices(&assets);
            assert!(result.is_ok());
            let prices = result.unwrap();
            assert_eq!(prices.len(), 3);
        }

        #[test]
        fn test_get_config() {
            let client = setup();
            let result = client.get_config();
            assert!(result.is_ok());
            let config = result.unwrap();
            assert_eq!(config.min_sources, 3);
            assert_eq!(config.decimals, 8);
        }

        #[test]
        fn test_validate_asset_id() {
            let client = setup();
            assert!(client.validate_asset_id("EURUSD").is_ok());
            assert!(client.validate_asset_id("").is_err());
        }
    }

    mod submission_operations {
        use super::*;

        #[test]
        fn test_submit_price() {
            let client = setup();
            let result = client.submit_price("EURUSD", 150_000_000);
            assert!(result.is_ok());
            let tx_result = result.unwrap();
            assert!(tx_result.success);
        }

        #[test]
        fn test_batch_submit_prices() {
            let client = setup();
            let submissions = vec![
                PriceSubmission {
                    asset: "EURUSD".to_string(),
                    price: 150_000_000,
                    timestamp: 1234567890,
                },
                PriceSubmission {
                    asset: "GBPUSD".to_string(),
                    price: 127_000_000,
                    timestamp: 1234567890,
                },
            ];
            let result = client.batch_submit_prices(&submissions);
            assert!(result.is_ok());
        }

        #[test]
        fn test_submit_price_validation() {
            let client = setup();
            let result = client.submit_price("", 150_000_000);
            assert!(result.is_err());
        }

        #[test]
        fn test_submit_negative_price() {
            let client = setup();
            let result = client.submit_price("EURUSD", -100);
            assert!(result.is_err());
        }

        #[test]
        fn test_validate_price() {
            let client = setup();
            assert!(client.validate_price(150_000_000).is_ok());
            assert!(client.validate_price(-1).is_err());
            assert!(client.validate_price(0).is_err());
        }
    }

    mod governance_operations {
        use super::*;

        #[test]
        fn test_propose_admin_action_urgent() {
            let client = setup();
            let result = client.propose_admin_action("setMinSources", OperationPriority::Urgent);
            assert!(result.is_ok());
            let proposal = result.unwrap();
            assert_eq!(proposal.priority_delay, 1);
        }

        #[test]
        fn test_propose_admin_action_normal() {
            let client = setup();
            let result = client.propose_admin_action("setMinSources", OperationPriority::Normal);
            assert!(result.is_ok());
            let proposal = result.unwrap();
            assert_eq!(proposal.priority_delay, 10);
        }

        #[test]
        fn test_propose_admin_action_long_term() {
            let client = setup();
            let result = client.propose_admin_action("setMinSources", OperationPriority::LongTerm);
            assert!(result.is_ok());
            let proposal = result.unwrap();
            assert_eq!(proposal.priority_delay, 100);
        }

        #[test]
        fn test_execute_admin_action() {
            let client = setup();
            let result = client.execute_admin_action(1);
            assert!(result.is_ok());
            let tx = result.unwrap();
            assert!(tx.success);
        }

        #[test]
        fn test_execute_nonexistent_action() {
            let client = setup();
            let result = client.execute_admin_action(0);
            assert!(result.is_err());
            assert_eq!(result.unwrap_err(), SdkError::OperationNotFound);
        }

        #[test]
        fn test_cancel_admin_action() {
            let client = setup();
            let result = client.cancel_admin_action(1);
            assert!(result.is_ok());
        }

        #[test]
        fn test_get_pending_actions() {
            let client = setup();
            let result = client.get_pending_actions();
            assert!(result.is_ok());
            let actions = result.unwrap();
            assert!(!actions.is_empty());
        }
    }

    mod timelock_operations {
        use super::*;

        #[test]
        fn test_validate_timelock_duration_positive() {
            let client = setup();
            assert!(client.validate_timelock_duration(100).is_ok());
        }

        #[test]
        fn test_validate_timelock_duration_zero() {
            let client = setup();
            let result = client.validate_timelock_duration(0);
            assert!(result.is_err());
        }
    }

    mod helper_functions {
        use super::*;

        #[test]
        fn test_format_price() {
            let client = setup();
            let formatted = client.format_price(150_000_000, 8);
            assert!(formatted.contains("1.5"));
        }

        #[test]
        fn test_parse_price() {
            let client = setup();
            let result = client.parse_price("1.50", 8);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 150_000_000);
        }

        #[test]
        fn test_parse_price_no_fraction() {
            let client = setup();
            let result = client.parse_price("1", 8);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 100_000_000);
        }

        #[test]
        fn test_parse_price_full_fraction() {
            let client = setup();
            let result = client.parse_price("1.12345678", 8);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 112_345_678);
        }

        #[test]
        fn test_calculate_median_odd_length() {
            let client = setup();
            let prices = vec![100, 150, 120, 110, 130];
            let result = client.calculate_median(&prices);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 120);
        }

        #[test]
        fn test_calculate_median_even_length() {
            let client = setup();
            let prices = vec![100, 150, 120, 110];
            let result = client.calculate_median(&prices);
            assert!(result.is_ok());
            let median = result.unwrap();
            assert_eq!(median, 115);
        }

        #[test]
        fn test_calculate_median_single_value() {
            let client = setup();
            let prices = vec![100];
            let result = client.calculate_median(&prices);
            assert!(result.is_ok());
            assert_eq!(result.unwrap(), 100);
        }

        #[test]
        fn test_calculate_median_empty() {
            let client = setup();
            let prices = vec![];
            let result = client.calculate_median(&prices);
            assert!(result.is_err());
        }
    }

    mod error_handling {
        use super::*;

        #[test]
        fn test_invalid_asset_error() {
            let client = setup();
            let result = client.get_price("");
            assert!(result.is_err());
        }

        #[test]
        fn test_operation_not_found() {
            let client = setup();
            let result = client.execute_admin_action(0);
            assert_eq!(result.unwrap_err(), SdkError::OperationNotFound);
        }

        #[test]
        fn test_insufficient_sources() {
            let client = setup();
            let result = client.calculate_median(&[]);
            assert_eq!(result.unwrap_err(), SdkError::InsufficientSources);
        }
    }

    mod type_definitions {
        use super::*;

        #[test]
        fn test_operation_priority_enum() {
            assert_eq!(OperationPriority::Urgent as u32, 0);
            assert_eq!(OperationPriority::Normal as u32, 1);
            assert_eq!(OperationPriority::LongTerm as u32, 2);
        }

        #[test]
        fn test_operation_type_enum() {
            assert_eq!(OperationType::Upgrade as u32, 0);
            assert_eq!(OperationType::SetAdmin as u32, 1);
            assert_eq!(OperationType::SetMinSources as u32, 2);
        }

        #[test]
        fn test_config_struct() {
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
        fn test_price_struct() {
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

    mod integration_scenarios {
        use super::*;

        #[test]
        fn test_complete_workflow() {
            let client = setup();

            // Get current price
            let price = client.get_price("EURUSD").unwrap();
            assert_eq!(price.asset, "EURUSD");

            // Get configuration
            let config = client.get_config().unwrap();
            assert!(config.min_sources > 0);

            // Propose governance action
            let proposal = client
                .propose_admin_action("setMinSources", OperationPriority::Normal)
                .unwrap();
            assert!(proposal.proposal_id > 0);
        }

        #[test]
        fn test_price_submission_workflow() {
            let client = setup();

            // Submit single price
            let result = client.submit_price("EURUSD", 150_000_000).unwrap();
            assert!(result.success);

            // Submit batch
            let submissions = vec![
                PriceSubmission {
                    asset: "EURUSD".to_string(),
                    price: 150_000_000,
                    timestamp: 1234567890,
                },
            ];
            let batch_result = client.batch_submit_prices(&submissions).unwrap();
            assert!(batch_result.success);
        }
    }
}
