//! Issue #453 — Upgrade and storage-migration schema desync penetration test.
//!
//! # Layout inventory (keys touched by `migration.rs`)
//!
//! | Key | v1 | v2 | Written by migration |
//! |---|---|---|---|
//! | `StorageVersion` | absent (read as 1) | `2` | on completion |
//! | `MigrationState` | absent | absent | while in progress, removed on completion |
//! | `AssetList` | `Vec<Address>` | unchanged | read only (cursor source) |
//! | `Aggregate(asset)` | present only once priced | present for every registered asset | zero placeholder if missing; TTL bump |
//!
//! Every other key has the same layout in v1 and v2, so migration does not
//! touch it. `inventory_is_complete_after_migration` checks that every
//! registered asset ends with an `Aggregate` entry.
//!
//! # Aliasing
//!
//! `Aggregate(asset)` is the only field that changes meaning: v2 writes a
//! placeholder `AggregatePrice { price: 0, timestamp: 0, num_sources: 0, .. }`
//! for unpriced assets. A reader that trusts presence instead of `price > 0`
//! would return a zero price. SEP-40 `lastprice` did exactly that after
//! migration (a silent wrong price); it now returns `None` for a non-positive
//! aggregate. `placeholder_aggregates_are_not_served_as_prices` pins this for
//! `get_price` and `lastprice`.
//!
//! # Findings
//!
//! * Aborting after any batch leaves `StorageVersion` at 1, all public reads
//!   working, and migration resumable from the stored cursor.
//! * Re-running a completed migration is a no-op (idempotent).
//! * **No version guard.** Reads do not check `StorageVersion`, and a stored
//!   version newer than `CURRENT_VERSION` is silently relabelled as
//!   `CURRENT_VERSION` by `migrate_storage` (`gap_newer_version_is_downgraded`).
//! * **Cursor over a mutable list.** The cursor indexes `AssetList`; assets
//!   registered mid-migration are appended and still covered, but removals
//!   would shift the index.

use soroban_sdk::{Address, Env, Vec};

use crate::migration::CURRENT_VERSION;
use crate::test_helpers::{ledger_default, register_test_asset, setup_basic};
use crate::types::{AggregatePrice, Asset, DataKey};
use crate::PriceOracleContractClient;

const TS: u64 = 1_000_000;

fn setup(e: &Env, n: u32) -> (PriceOracleContractClient<'_>, Address, Vec<Address>) {
    e.mock_all_auths();
    ledger_default(e, 100, TS);
    let (client, _admin, source, first) = setup_basic(e);
    client.submit_price(&source, &first, &1_000i128, &TS);
    let mut assets = Vec::new(e);
    assets.push_back(first);
    for _ in 1..n {
        assets.push_back(register_test_asset(e, &client));
    }
    // Simulate a v1 deployment.
    e.as_contract(&client.address, || {
        e.storage().persistent().remove(&DataKey::StorageVersion)
    });
    (client, source, assets)
}

fn snapshot(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    assets: &Vec<Address>,
) -> Vec<Option<AggregatePrice>> {
    e.as_contract(&client.address, || {
        let mut out = Vec::new(e);
        for a in assets.iter() {
            out.push_back(e.storage().persistent().get(&DataKey::Aggregate(a)));
        }
        out
    })
}

fn assert_reads_coherent(client: &PriceOracleContractClient<'_>, assets: &Vec<Address>) {
    let priced = assets.get_unchecked(0);
    assert_eq!(client.get_price(&priced, &u64::MAX).unwrap().price, 1_000);
    assert_eq!(
        client.lastprice(&Asset::Stellar(priced)).unwrap().price,
        1_000
    );
    for i in 1..assets.len() {
        let a = assets.get_unchecked(i);
        assert!(client
            .lastprice(&Asset::Stellar(a))
            .is_none_or(|p| p.price > 0));
    }
}

#[test]
fn abort_after_every_step_is_coherent_and_resumable() {
    for stop_after in 1..4u32 {
        let e = Env::default();
        let (client, _source, assets) = setup(&e, 4);
        assert_eq!(client.get_storage_version(), 1);

        for _ in 0..stop_after {
            client.migrate_storage(&1u32);
        }
        // Aborted mid-way: still v1, cursor persisted, reads coherent.
        assert_eq!(client.get_storage_version(), 1);
        assert_eq!(client.get_migration_state().unwrap().cursor, stop_after);
        assert_reads_coherent(&client, &assets);

        // Resume to completion.
        while client.get_migration_state().is_some()
            || client.get_storage_version() < CURRENT_VERSION
        {
            client.migrate_storage(&1u32);
        }
        assert_eq!(client.get_storage_version(), CURRENT_VERSION);
        assert_reads_coherent(&client, &assets);
    }
}

#[test]
fn migration_is_idempotent() {
    let e = Env::default();
    let (client, _source, assets) = setup(&e, 3);
    client.migrate_storage(&0u32);
    let first = snapshot(&e, &client, &assets);
    let version = client.get_storage_version();

    client.migrate_storage(&0u32);
    assert_eq!(snapshot(&e, &client, &assets), first);
    assert_eq!(client.get_storage_version(), version);
    assert!(client.get_migration_state().is_none());
}

#[test]
fn inventory_is_complete_after_migration() {
    let e = Env::default();
    let (client, _source, assets) = setup(&e, 5);
    client.migrate_storage(&0u32);
    for entry in snapshot(&e, &client, &assets).iter() {
        assert!(entry.is_some());
    }
}

#[test]
fn placeholder_aggregates_are_not_served_as_prices() {
    let e = Env::default();
    let (client, _source, assets) = setup(&e, 2);
    client.migrate_storage(&0u32);
    let unpriced = assets.get_unchecked(1);
    assert!(client
        .lastprice(&Asset::Stellar(unpriced.clone()))
        .is_none_or(|p| p.price > 0));
    assert!(client
        .try_get_price(&unpriced, &u64::MAX)
        .map_or(true, |r| r.map_or(true, |p| p.is_none_or(|p| p.price > 0))));
}

#[test]
fn asset_registered_mid_migration_is_covered() {
    let e = Env::default();
    let (client, _source, mut assets) = setup(&e, 2);
    client.migrate_storage(&1u32);
    assets.push_back(register_test_asset(&e, &client));
    while client.get_migration_state().is_some() {
        client.migrate_storage(&1u32);
    }
    for entry in snapshot(&e, &client, &assets).iter() {
        assert!(entry.is_some());
    }
}

#[test]
fn gap_newer_version_is_downgraded() {
    let e = Env::default();
    let (client, _source, _assets) = setup(&e, 1);
    e.as_contract(&client.address, || {
        e.storage()
            .persistent()
            .set(&DataKey::StorageVersion, &(CURRENT_VERSION + 1))
    });
    client.migrate_storage(&0u32);
    // No guard: state written by a newer schema is relabelled as current.
    assert_eq!(client.get_storage_version(), CURRENT_VERSION);
}
