//! Differential test between `contracts/minimal-oracle` and the core contract (#830).
//!
//! A single shared submission trace is replayed against both contracts under
//! their default configuration, and the SEP-40 `lastprice` / `price` results
//! must agree at every step.
//!
//! The trace is a fixed, deterministic list of `(source_index, price)` pairs
//! applied to one asset. Both contracts are driven through their public
//! entrypoints only, so the comparison is at the interface level.
//!
//! ```sh
//! cargo test -p price-oracle --lib minimal_oracle_diff_tests
//! ```

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

use crate::types::Asset;
use crate::PriceOracleContractClient;

/// Minimal oracle contract, compiled from `contracts/minimal-oracle`.
///
/// The crate is a separate package, so the test drives it through the same
/// generated client shape it exposes on-chain.
mod minimal {
    use soroban_sdk::{contractclient, Address, Env};

    #[contractclient(name = "MinimalOracleClient")]
    pub trait MinimalOracleInterface {
        fn initialize(env: Env, admin: Address, decimals: u32);
        fn add_source(env: Env, source: Address);
        fn submit_price(env: Env, source: Address, asset: Address, price: i128);
        fn get_price(env: Env, asset: Address) -> i128;
    }
}

use minimal::MinimalOracleClient;

/// Number of sources registered on both contracts.
const SOURCE_COUNT: u32 = 3;
/// Decimals configured on both contracts.
const DECIMALS: u32 = 7;
/// Base ledger timestamp for the trace.
const BASE_TIMESTAMP: u64 = 1_700_000_000;

/// Shared submission trace: `(source_index, price)` applied in order.
const TRACE: &[(u32, i128)] = &[
    (0, 100_000_000),
    (1, 102_000_000),
    (2, 98_000_000),
    (0, 101_000_000),
    (1, 99_000_000),
    (2, 103_000_000),
    (0, 97_000_000),
    (1, 105_000_000),
    (2, 100_000_000),
    (0, 100_000_000),
];

struct Fixture<'a> {
    env: Env,
    core: PriceOracleContractClient<'a>,
    minimal: MinimalOracleClient<'a>,
    sources: soroban_sdk::Vec<Address>,
    asset: Address,
}

fn setup() -> Fixture<'static> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(BASE_TIMESTAMP);

    let admin = Address::generate(&env);

    // Core contract, default configuration: median aggregation, no quorum
    // beyond a single source, no resolution window.
    let core_id = env.register(crate::PriceOracleContract, ());
    let core = PriceOracleContractClient::new(&env, &core_id);
    core.initialize(
        &admin,
        &1,
        &100,
        &DECIMALS,
        &String::from_str(&env, "Differential Oracle"),
    );

    // Minimal oracle, default configuration.
    let minimal_id = env.register(crate::minimal_oracle_diff_tests::minimal::MinimalOracle, ());
    let minimal = MinimalOracleClient::new(&env, &minimal_id);
    minimal.initialize(&admin, &DECIMALS);

    let mut sources = soroban_sdk::Vec::new(&env);
    for i in 0..SOURCE_COUNT {
        let source = Address::generate(&env);
        core.add_source(&source, &String::from_str(&env, "src"));
        minimal.add_source(&source);
        sources.push_back(source);
        let _ = i;
    }

    let asset = Address::generate(&env);
    core.register_asset(&asset, &String::from_str(&env, "DIFF"));

    Fixture {
        env,
        core,
        minimal,
        sources,
        asset,
    }
}

/// Reads the SEP-40 `lastprice` from the core contract.
fn core_lastprice(f: &Fixture<'_>) -> Option<i128> {
    f.core
        .lastprice(&Asset::Stellar(f.asset.clone()))
        .map(|p| p.price)
}

#[test]
fn differential_trace_matches_minimal_oracle() {
    let f = setup();

    for (step, (source_index, price)) in TRACE.iter().enumerate() {
        let source = f.sources.get_unchecked(*source_index);
        let timestamp = BASE_TIMESTAMP + step as u64;

        f.env.ledger().set_timestamp(timestamp);
        f.core
            .submit_price(&source, &f.asset, price, &timestamp);
        f.minimal
            .submit_price(&source, &f.asset, price);

        let core_price = core_lastprice(&f);
        let minimal_price = f.minimal.get_price(&f.asset);

        assert_eq!(
            core_price,
            Some(minimal_price),
            "divergence at trace step {step} (source {source_index}, price {price})"
        );
    }
}

#[test]
fn differential_trace_matches_before_any_submission() {
    let f = setup();

    // No submissions yet: the core has no aggregate, the minimal oracle has no
    // data. Both must report "no price" rather than a placeholder.
    assert_eq!(core_lastprice(&f), None);
    assert!(f.minimal.try_get_price(&f.asset).is_err());
}
