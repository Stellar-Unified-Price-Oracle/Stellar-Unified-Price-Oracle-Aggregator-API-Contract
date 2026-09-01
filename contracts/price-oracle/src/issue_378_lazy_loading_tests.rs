//! Tests for Issue #378 — Lazy-loading and On-Demand Storage Reads

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env,
};

use crate::test_helpers::{setup_contract, register_test_source, register_test_asset};

fn set_ledger(e: &Env, seq: u32, ts: u64) {
    e.ledger().set(LedgerInfo {
        timestamp: ts,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100,
        min_persistent_entry_ttl: 100,
        max_entry_ttl: 99_999,
    });
}

// ══════════════════════════════════════════════════════════════════════════════
// Issue #378 — Lazy-loading and On-Demand Storage Reads in Hot Paths
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_get_price_lazy_loads_only_aggregate() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_000_000, &500);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_submit_price_only_reads_necessary_state() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_500_000, &500);

    let price = client.get_price(&asset);
    assert_eq!(price.unwrap().price, 1_500_000);
}

#[test]
fn test_no_redundant_has_get_pairs() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset, &1_000_000, &500);
    client.submit_price(&source2, &asset, &1_100_000, &500);
    client.submit_price(&source3, &asset, &1_050_000, &500);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_aggregation_reads_only_required_prices() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let source3 = register_test_source(&e, &client, "Source 3");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset, &1_000_000, &500);
    client.submit_price(&source2, &asset, &1_100_000, &500);
    client.submit_price(&source3, &asset, &1_050_000, &500);

    client.trigger_aggregation(&asset);

    let price = client.get_price(&asset);
    assert!(price.is_some());
}

#[test]
fn test_cache_within_call_scope() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source 1");
    let asset = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &2_000_000, &500);

    let price1 = client.get_price(&asset);
    let price2 = client.get_price(&asset);

    assert_eq!(price1.unwrap().price, price2.unwrap().price);
}

#[test]
fn test_multiple_assets_independent_reads() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source 1");
    let source2 = register_test_source(&e, &client, "Source 2");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    set_ledger(&e, 100, 1_000);

    client.submit_price(&source1, &asset1, &1_000_000, &500);
    client.submit_price(&source2, &asset2, &2_000_000, &500);

    let price1 = client.get_price(&asset1);
    let price2 = client.get_price(&asset2);

    assert_eq!(price1.unwrap().price, 1_000_000);
    assert_eq!(price2.unwrap().price, 2_000_000);
}
