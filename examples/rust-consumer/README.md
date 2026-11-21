# Rust Consumer Contract Quickstart (5-Minute Guide)

This quickstart demonstrates how to consume prices from the Stellar Unified Price Oracle in your Soroban smart contracts using SEP-40.

## Contract Snippet

```rust
#![no_std]
use soroban_sdk::{contract, contractimpl, Address, Env};
use sep40_guard::{Asset, PriceData, Sep40Client};

#[contract]
pub struct OracleConsumerContract;

#[contractimpl]
impl OracleConsumerContract {
    pub fn read_asset_price(env: Env, oracle: Address, asset: Address) -> Option<PriceData> {
        let client = Sep40Client::new(&env, &oracle);
        let sep40_asset = Asset::Stellar(asset);
        client.lastprice(&sep40_asset)
    }
}
```

## Running Tests

```bash
cargo test --manifest-path examples/rust-consumer/Cargo.toml
```
