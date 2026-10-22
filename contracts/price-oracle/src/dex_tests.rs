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

// -----------------------------------------------------------------------------
// Adversarial DEX/AMM tests (#448) — see docs/dex-amm-manipulation.md
// -----------------------------------------------------------------------------

#[test]
fn test_attacker_cannot_create_and_seed_pool() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    client.initialize(&admin, &1u32, &50u32, &18u32, &String::from_str(&e, "DEX"));

    let asset_a = Address::generate(&e);
    let asset_b = Address::generate(&e);
    client.register_asset(&asset_a);
    client.register_asset(&asset_b);

    // Drop all mocked auths: an attacker seeding a 1:1_000_000 pool must fail.
    e.set_auths(&[]);
    assert!(client
        .try_dex_register_pool(&asset_a, &asset_b, &1i128, &1_000_000i128)
        .is_err());
    assert!(client.get_dex_price(&asset_a).is_none());
}

#[test]
fn test_route_selection_ignores_more_favourable_thin_pool() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    client.initialize(&admin, &1u32, &50u32, &18u32, &String::from_str(&e, "DEX"));

    let asset = Address::generate(&e);
    let quote = Address::generate(&e);
    let thin = Address::generate(&e);
    client.register_asset(&asset);
    client.register_asset(&quote);
    client.register_asset(&thin);

    // Deep canonical pool: price 2.0.
    client.dex_register_pool(&asset, &quote, &1_000_000i128, &2_000_000i128);
    let canonical = client.get_dex_price(&asset).unwrap().price;

    // Thin pool quoting 1000x higher must not be selected as the route.
    client.dex_register_pool(&asset, &thin, &1i128, &2_000i128);
    let routed = client.get_dex_price(&asset).unwrap();
    assert_eq!(routed.price, canonical);
    assert_eq!(routed.reserve_x, 1_000_000);
}
