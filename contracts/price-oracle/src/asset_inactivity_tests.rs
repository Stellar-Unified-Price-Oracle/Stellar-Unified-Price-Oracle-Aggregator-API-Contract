#![cfg(test)]
use crate::test_helpers::*;
use soroban_sdk::Env;

#[test]
fn test_set_get_inactivity_timeout() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = setup_basic(&e);
    client.set_inactivity_timeout(&17280u32);
    assert_eq!(client.get_inactivity_timeout(), 17280u32);
}

#[test]
fn test_set_get_asset_inactivity_timeout() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = setup_basic(&e);
    client.set_asset_inactivity_timeout(&asset, &5000u32);
    assert_eq!(client.get_asset_inactivity_timeout(&asset), 5000u32);
}

#[test]
fn test_asset_inactivity_timeout_falls_back_to_global() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = setup_basic(&e);
    client.set_inactivity_timeout(&1000u32);
    // No per-asset timeout set — should return global
    assert_eq!(client.get_asset_inactivity_timeout(&asset), 1000u32);
}

#[test]
fn test_submit_price_records_activity() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = setup_basic(&e);
    client.submit_price(
        &source,
        &asset,
        &1_000_000i128,
        &(e.ledger().timestamp()),
        &1u64,
    );
    // After submit, last activity should be recorded (not None in storage)
    // We verify indirectly: is_asset_inactive with a small timeout should be false immediately
    client.set_inactivity_timeout(&100u32);
    assert!(!client.is_asset_inactive(&asset));
}

#[test]
fn test_is_asset_inactive_false_when_disabled() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = setup_basic(&e);
    // No timeout set (0 = disabled)
    assert!(!client.is_asset_inactive(&asset));
}

#[test]
fn test_clear_per_asset_timeout() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = setup_basic(&e);
    client.set_asset_inactivity_timeout(&asset, &5000u32);
    client.set_asset_inactivity_timeout(&asset, &0u32); // clear
    assert_eq!(client.get_asset_inactivity_timeout(&asset), 0u32); // falls back to global (0)
}
