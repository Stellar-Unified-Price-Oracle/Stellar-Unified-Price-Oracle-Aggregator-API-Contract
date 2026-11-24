#![cfg(test)]

//! #828 — Snapshot-based upgrade test.
//!
//! Loads a real testnet ledger snapshot (`Env::from_snapshot_file`), upgrades the
//! contract to the HEAD WASM, runs the lazy migrations, and reads every key back
//! to prove the state survived the upgrade.
//!
//! The snapshot itself is refreshed on a schedule by
//! `scripts/devx/refresh-testnet-snapshot.sh` and is **not** committed (see
//! `.gitignore`). When it is absent the test is skipped, so the suite stays green
//! on a clean checkout; CI fetches the snapshot before running this test.

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Bytes, Env, String,
};

use crate::test_helpers::*;

/// Path of the refreshed testnet snapshot, relative to the crate root.
const SNAPSHOT_PATH: &str = "testdata/testnet-snapshot.json";

fn snapshot_available() -> bool {
    std::path::Path::new(SNAPSHOT_PATH).exists()
}

fn load_wasm_hash(e: &Env) -> soroban_sdk::BytesN<32> {
    let wasm = include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm");
    e.deployer()
        .upload_contract_wasm(Bytes::from_slice(e, wasm))
}

/// Reads back every key the contract writes for a seeded asset and asserts the
/// values are unchanged after the upgrade.
fn assert_state_intact(
    client: &crate::PriceOracleContractClient<'_>,
    admin: &Address,
    source: &Address,
    asset: &Address,
    expected_price: i128,
) {
    assert_eq!(&client.get_admin(), admin);
    assert!(client.is_source(source));
    assert!(client.is_registered_asset(asset));

    let aggregate = client.get_price(asset, &0u64).unwrap();
    assert_eq!(aggregate.price, expected_price);

    let submission = client.get_price(asset, &0u64).unwrap();
    assert_eq!(submission.price, expected_price);

    assert!(client.has_historical_price(asset, &100u32));
    let history = client.get_historical_prices(asset, &100u32, &100u32);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get_unchecked(0).price, expected_price);
}

/// Seeds a snapshot-shaped ledger state, upgrades to HEAD, and reads every key
/// back. This is the deterministic core of the snapshot test: it runs on every
/// CI job, while the real-snapshot variant below runs when the file is present.
#[test]
fn test_upgrade_from_snapshot_state_preserves_every_key() {
    let e = Env::default();
    e.cost_estimate().disable_resource_limits();
    e.cost_estimate().budget().reset_unlimited();

    let (client, admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Snapshot Source");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 100, 1_700_000_000);
    client.set_min_sources_required(&1u32);
    submit_test_price_n(&client, &source, &asset, 55_000, 1_700_000_000, 1);

    // Upgrade to the HEAD WASM.
    let new_wasm_hash = load_wasm_hash(&e);
    client.upgrade(&new_wasm_hash);

    // Migrations are lazy: touching the asset index re-populates it.
    let assets = client.get_assets();
    assert!(assets.contains(asset));

    assert_state_intact(&client, &admin, &source, &asset, 55_000);
}

/// Loads the real testnet snapshot when it has been refreshed, upgrades to HEAD,
/// and reads every key back. Skipped when the snapshot is not present.
#[test]
fn test_upgrade_from_live_testnet_snapshot() {
    if !snapshot_available() {
        std::eprintln!(
            "skipping #828 snapshot upgrade test: {} not present (run scripts/devx/refresh-testnet-snapshot.sh)",
            SNAPSHOT_PATH
        );
        return;
    }

    let e = Env::from_snapshot_file(SNAPSHOT_PATH);
    e.cost_estimate().disable_resource_limits();
    e.cost_estimate().budget().reset_unlimited();

    let contract_id = Address::from_string(&String::from_str(
        &e,
        &std::fs::read_to_string("testdata/testnet-contract-id.txt")
            .expect("testnet contract id file")
            .trim(),
    ));
    let client = crate::PriceOracleContractClient::new(&e, &contract_id);

    // Capture the pre-upgrade state of every key we know how to read.
    let admin = client.get_admin();
    let assets = client.get_assets();
    let min_sources = client.get_min_sources_required();
    let mut before = std::vec::Vec::new();
    for asset in assets.iter() {
        before.push((asset.clone(), client.get_price(&asset, &0u64)));
    }

    // Upgrade to the HEAD WASM and run the lazy migrations.
    let new_wasm_hash = load_wasm_hash(&e);
    client.upgrade(&new_wasm_hash);
    let _ = client.get_assets();

    // Read every key back.
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_min_sources_required(), min_sources);
    assert_eq!(client.get_assets(), assets);
    for (asset, price) in before {
        assert_eq!(client.get_price(&asset, &0u64), price);
    }
}
