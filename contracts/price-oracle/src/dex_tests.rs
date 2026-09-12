#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

use crate::test_helpers::*;

#[test]
fn test_register_and_read_dex_pool() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(&admin, &1u32, &50u32, &18u32, &String::from_str(&e, "DEX"));

    let asset_a = Address::generate(&e);
    let asset_b = Address::generate(&e);
    client.register_asset(&asset_a);
    client.register_asset(&asset_b);

    client.dex_register_pool(&asset_a, &asset_b, &1000i128, &2000i128);

    let price = client.get_dex_price(&asset_a);
    assert!(price.is_some());
    assert!(price.unwrap().price > 0);
}

#[test]
fn test_dex_price_none_when_unregistered() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(&admin, &1u32, &50u32, &18u32, &String::from_str(&e, "DEX"));

    let asset = Address::generate(&e);
    let price = client.get_dex_price(&asset);
    assert!(price.is_none());
}
