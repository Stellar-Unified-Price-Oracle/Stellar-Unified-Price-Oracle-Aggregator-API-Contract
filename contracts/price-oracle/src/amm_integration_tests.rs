#![cfg(test)]

use soroban_sdk::{Address, Env, String};

use crate::test_helpers::*;

#[test]
fn test_soroswap_pool_lifecycle() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(&admin, &1u32, &50u32, &18u32, &String::from_str(&e, "AMM"));

    let asset_a = Address::generate(&e);
    let asset_b = Address::generate(&e);

    client.soroswap_register_pool(&asset_a, &asset_b, &1000i128, &2000i128, &30u32);
    let pool = client.soroswap_get_pool(&asset_a, &asset_b);
    assert!(pool.is_some());
    assert_eq!(pool.unwrap().fee_bps, 30u32);

    client.soroswap_set_pool_status(&asset_a, &asset_b, false);
    let disabled = client.soroswap_get_pool(&asset_a, &asset_b);
    assert!(disabled.is_some());
    assert!(!disabled.unwrap().enabled);
}

#[test]
fn test_amm_weight_config() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(
        &admin,
        &1u32,
        &50u32,
        &18u32,
        &String::from_str(&e, "Weight"),
    );

    let asset = Address::generate(&e);
    client.amm_set_weight(&asset, &500u32, true);

    let cfg = client.amm_get_weight(&asset);
    assert!(cfg.is_some());
    assert_eq!(cfg.unwrap().weight_bps, 500u32);
    assert!(cfg.unwrap().enabled);
}

#[test]
fn test_get_soroswap_price() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(
        &admin,
        &1u32,
        &50u32,
        &18u32,
        &String::from_str(&e, "Price"),
    );

    let asset_a = Address::generate(&e);
    let asset_b = Address::generate(&e);

    client.soroswap_register_pool(&asset_a, &asset_b, &1000i128, &2000i128, &30u32);
    let price = client.get_soroswap_price(&asset_a, &asset_b);
    assert!(price.is_some());
    assert!(price.unwrap() > 0);
}
