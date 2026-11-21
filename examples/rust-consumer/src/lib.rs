#![no_std]
//! # Quickstart: Rust Consumer Contract
//!
//! A 5-minute copy-paste example showing how a Soroban smart contract
//! consumes prices from the Stellar Unified Price Oracle via SEP-40.

use soroban_sdk::{contract, contractimpl, Address, Env};
use sep40_guard::{Asset, PriceData, Sep40Client};

#[contract]
pub struct OracleConsumerContract;

#[contractimpl]
impl OracleConsumerContract {
    /// Read the latest price for an asset directly from the oracle contract.
    /// Returns the raw price (scaled by 10^decimals) and the observation timestamp.
    pub fn read_asset_price(env: Env, oracle: Address, asset: Address) -> Option<PriceData> {
        let client = Sep40Client::new(&env, &oracle);
        let sep40_asset = Asset::Stellar(asset);
        client.lastprice(&sep40_asset)
    }

    /// Read price and normalize against decimal precision.
    pub fn read_price_and_decimals(env: Env, oracle: Address, asset: Address) -> (Option<PriceData>, u32) {
        let client = Sep40Client::new(&env, &oracle);
        let decimals = client.decimals();
        let sep40_asset = Asset::Stellar(asset);
        (client.lastprice(&sep40_asset), decimals)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl,
        testutils::Address as _,
        Symbol,
    };

    // Minimal mock SEP-40 oracle contract for testing
    #[contract]
    pub struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn lastprice(_env: Env, _asset: Asset) -> Option<PriceData> {
            Some(PriceData {
                price: 65_000_0000000, // $65,000 with 7 decimals
                timestamp: 1_700_000_000,
                last_updated: 100,
            })
        }

        pub fn decimals(_env: Env) -> u32 {
            7
        }
    }

    #[test]
    fn test_consumer_reads_mock_oracle() {
        let env = Env::default();
        let oracle_id = env.register(MockOracle, ());
        let consumer_id = env.register(OracleConsumerContract, ());
        let client = OracleConsumerContractClient::new(&env, &consumer_id);

        let test_asset = Address::generate(&env);
        let price_data = client.read_asset_price(&oracle_id, &test_asset);
        assert!(price_data.is_some());
        let price = price_data.unwrap();
        assert_eq!(price.price, 65_000_0000000);
        assert_eq!(price.timestamp, 1_700_000_000);

        let (data, dec) = client.read_price_and_decimals(&oracle_id, &test_asset);
        assert_eq!(dec, 7);
        assert_eq!(data.unwrap().price, 65_000_0000000);
    }
}
