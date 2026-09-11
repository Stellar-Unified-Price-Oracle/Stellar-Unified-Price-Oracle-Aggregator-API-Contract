#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::{test_helpers::setup_contract, PriceOracleContractClient};

fn setup_price_bounds_case<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address, Address) {
    let (client, _admin) = setup_contract(e);
    let source = Address::generate(e);
    let asset = Address::generate(e);
    client.add_source(&source, &String::from_str(e, "Source"));
    client.register_asset(&asset);
    client.set_min_sources_required(&1u32);
    client.set_price_bounds(&asset, &100i128, &1000i128, &100u32);
    (client, source, asset)
}

#[test]
fn rejects_out_of_bounds_submissions() {
    let e = Env::default();
    let (client, source, asset) = setup_price_bounds_case(&e);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.submit_price(&source, &asset, &1500i128, &1000u64);
    }));
    assert!(result.is_err());
}

#[test]
fn trips_circuit_breaker_for_large_price_move() {
    let e = Env::default();
    let (client, source, asset) = setup_price_bounds_case(&e);
    client.submit_price(&source, &asset, &100i128, &1000u64);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.submit_price(&source, &asset, &1000i128, &1001u64);
    }));
    assert!(result.is_err());
    assert!(client.is_asset_paused(&asset));
    assert!(client.is_circuit_breaker_tripped(&asset));
}
