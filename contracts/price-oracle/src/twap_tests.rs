#![cfg(test)]

use soroban_sdk::Env;

use crate::test_helpers::{
    ledger_default, register_test_asset, register_test_source, setup_contract, submit_test_price,
};
use crate::TwapMethod;

#[test]
fn test_get_twap_arithmetic_and_geometric() {
    let env = Env::default();
    ledger_default(&env, 10, 1_000);

    let (client, _admin) = setup_contract(&env);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&env, &client, "Source");
    let asset = register_test_asset(&env, &client);

    submit_test_price(&client, &source, &asset, 100, 1_000);

    let first_twap = client.get_twap(&asset, &1u32, &TwapMethod::Arithmetic);
    assert!(first_twap.is_some());
    let first_data = first_twap.unwrap();
    assert_eq!(first_data.price, 100);
    assert_eq!(first_data.timestamp, 1_000);
    assert_eq!(first_data.last_updated, 10);

    ledger_default(&env, 11, 1_001);
    submit_test_price(&client, &source, &asset, 400, 1_001);

    let arithmetic = client
        .get_twap(&asset, &2u32, &TwapMethod::Arithmetic)
        .unwrap();
    assert_eq!(arithmetic.price, 250);
    assert_eq!(arithmetic.timestamp, 1_001);
    assert_eq!(arithmetic.last_updated, 11);

    let geometric = client
        .get_twap(&asset, &2u32, &TwapMethod::Geometric)
        .unwrap();
    assert_eq!(geometric.price, 200);
    assert_eq!(geometric.timestamp, 1_001);
    assert_eq!(geometric.last_updated, 11);
}
