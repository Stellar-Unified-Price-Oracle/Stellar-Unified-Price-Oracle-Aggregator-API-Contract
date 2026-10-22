#![cfg(test)]

//! Tests for issues #247, #251, #252, #253
//! - #252: Versioned aggregate price
//! - #247: History compaction
//! - #251: History sharding
//! - #253: Storage budget calculator

use soroban_sdk::{testutils::Ledger, Address, Env};

use crate::test_helpers::*;

// ─────────────────────────────────────────────────────────────────────────────
// #252 — Versioned Aggregate Price
// ─────────────────────────────────────────────────────────────────────────────

/// Submitting a new distinct price increments the version.
#[test]
fn test_version_increments_on_price_change() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    // First submission — version should be 0 (first aggregation, starts from default 0 → change from 0).
    client.submit_price(&source, &asset, &1_000_000i128, &1000u64);
    let v1 = client.get_aggregate_with_version(&asset);
    assert_eq!(v1.aggregate.price, 1_000_000i128);
    // version should be 1 because price changed from the default 0.
    assert_eq!(v1.version, 1u32);
    assert_eq!(v1.aggregate.version, v1.version);

    // Second submission with same price — version must NOT change.
    e.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 1100,
        protocol_version: 26,
        sequence_number: 2,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
    client.submit_price(&source, &asset, &1_000_000i128, &1100u64);
    let v2 = client.get_aggregate_with_version(&asset);
    assert_eq!(v2.version, 1u32, "version must not change when price is unchanged");

    // Third submission with different price — version must increment.
    e.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 1200,
        protocol_version: 26,
        sequence_number: 3,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
    client.submit_price(&source, &asset, &2_000_000i128, &1200u64);
    let v3 = client.get_aggregate_with_version(&asset);
    assert_eq!(v3.aggregate.price, 2_000_000i128);
    assert_eq!(v3.version, 2u32, "version must increment when price changes");
}

/// Version is consistent between get_aggregate_with_version and get_price.
#[test]
fn test_version_consistent_with_get_price() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    client.submit_price(&source, &asset, &5_000_000i128, &500u64);
    let versioned = client.get_aggregate_with_version(&asset);
    let direct = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(versioned.aggregate.price, direct.price);
    assert_eq!(versioned.aggregate.version, direct.version);
}

/// get_aggregate_with_version panics when asset has no price data.
#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_get_aggregate_with_version_no_data() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    client.get_aggregate_with_version(&asset);
}

/// Version increments monotonically across many price changes.
#[test]
fn test_version_monotonic_across_multiple_changes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    let prices: [i128; 5] = [100, 200, 300, 400, 500];
    let mut last_version: u32 = 0;
    for (i, &p) in prices.iter().enumerate() {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 100,
            protocol_version: 26,
            sequence_number: (i + 1) as u32,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset, &p, &(1000u64 + i as u64 * 100));
        let v = client.get_aggregate_with_version(&asset);
        assert!(v.version > last_version, "version must increase at step {}", i);
        last_version = v.version;
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// #247 — History Compaction
// ─────────────────────────────────────────────────────────────────────────────

/// compact_history returns correct metadata when threshold is 0 (no-op).
#[test]
fn test_compact_history_disabled_noop() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    // Submit several distinct prices to build history.
    for i in 0..5u32 {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 50,
            protocol_version: 26,
            sequence_number: i + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset, &((i + 1) as i128 * 1_000_000), &(1000 + i as u64 * 50));
    }

    // Threshold is 0 (default) → compact_history is a no-op.
    let meta = client.compact_history(&asset);
    assert_eq!(meta.original_count, meta.compacted_count);
    assert_eq!(meta.threshold_bps, 0u32);
}

/// compact_history removes stable-price entries within threshold.
#[test]
fn test_compact_history_removes_stable_entries() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    // Set threshold to 1% (100 bps).
    client.set_compaction_threshold_bps(&100u32);
    assert_eq!(client.get_compaction_threshold_bps(), 100u32);

    // Submit 5 prices: first 3 are very close together (~0.1% spread),
    // then a large jump, then another stable entry.
    let submissions: [(i128, u64); 5] = [
        (1_000_000, 1000),
        (1_000_500, 1100),  // +0.05% from prev — within 1%
        (1_001_000, 1200),  // +0.05% from prev — within 1%
        (2_000_000, 1300),  // +100% from prev — outside threshold
        (2_000_100, 1400),  // +0.005% from prev — within 1%
    ];

    for (i, (price, ts)) in submissions.iter().enumerate() {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: *ts,
            protocol_version: 26,
            sequence_number: (i + 1) as u32,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset, price, ts);
    }

    let meta = client.compact_history(&asset);
    // original 5 entries; the stable middle entries (index 1, 2) and the last stable
    // one (index 4) may be compacted. At minimum the count must have decreased.
    assert_eq!(meta.original_count, 5u32);
    assert!(
        meta.compacted_count < meta.original_count,
        "compacted_count={} must be < original={}",
        meta.compacted_count,
        meta.original_count
    );
    // First and last entries are always retained.
    assert!(meta.compacted_count >= 2u32);
}

/// set_compaction_threshold_bps requires admin auth.
#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_compaction_threshold_unauthorized() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    clear_auth(&e);
    client.set_compaction_threshold_bps(&100u32);
}

/// compact_history requires admin auth.
#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_compact_history_unauthorized() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    client.submit_price(&source, &asset, &1_000_000i128, &1000u64);
    clear_auth(&e);
    client.compact_history(&asset);
}

/// get_compaction_metadata returns None before any compaction.
#[test]
fn test_get_compaction_metadata_initial_none() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let meta = client.get_compaction_metadata(&asset);
    assert!(meta.is_none());
}

/// Metadata is stored after compact_history runs.
#[test]
fn test_compact_history_metadata_stored() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    client.submit_price(&source, &asset, &100i128, &1000u64);

    let meta_after = client.compact_history(&asset);
    let stored = client.get_compaction_metadata(&asset).unwrap();
    assert_eq!(stored.original_count, meta_after.original_count);
    assert_eq!(stored.compacted_count, meta_after.compacted_count);
}

// ─────────────────────────────────────────────────────────────────────────────
// #251 — History Sharding
// ─────────────────────────────────────────────────────────────────────────────

/// migrate_history_to_shards migrates all existing entries.
#[test]
fn test_migrate_history_to_shards() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    // Write 3 distinct history entries.
    for i in 0..3u32 {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 100,
            protocol_version: 26,
            sequence_number: i + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset, &((i + 1) as i128 * 100), &(1000 + i as u64 * 100));
    }

    let migrated = client.migrate_history_to_shards(&asset);
    assert_eq!(migrated, 3u32);
}

/// migrate_history_to_shards requires admin auth.
#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_migrate_history_unauthorized() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    clear_auth(&e);
    client.migrate_history_to_shards(&asset);
}

/// get_bucket_entries returns entries after migration.
#[test]
fn test_get_bucket_entries_after_migration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    // All three ledgers land in bucket 0 (ledger < 120_960).
    for i in 0..3u32 {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 100,
            protocol_version: 26,
            sequence_number: i + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset, &((i + 1) as i128 * 100), &(1000 + i as u64 * 100));
    }

    client.migrate_history_to_shards(&asset);

    // Query bucket containing ledger 1 (bucket 0).
    let entries = client.get_bucket_entries(&asset, &1u32);
    assert_eq!(entries.len(), 3u32, "bucket should have 3 migrated entries");
}

/// migrate_history_to_shards is idempotent — running twice doesn't duplicate.
#[test]
fn test_migrate_history_idempotent() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    for i in 0..2u32 {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 100,
            protocol_version: 26,
            sequence_number: i + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset, &((i + 1) as i128 * 500), &(1000 + i as u64 * 100));
    }

    client.migrate_history_to_shards(&asset);
    client.migrate_history_to_shards(&asset); // second call — must not duplicate

    let entries = client.get_bucket_entries(&asset, &1u32);
    assert_eq!(entries.len(), 2u32, "entries must not be duplicated after 2 migrations");
}

// ─────────────────────────────────────────────────────────────────────────────
// #253 — Storage Budget Calculator
// ─────────────────────────────────────────────────────────────────────────────

/// get_storage_budget returns zero for an asset with no history.
#[test]
fn test_storage_budget_empty() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    let budget = client.get_storage_budget(&asset);
    assert_eq!(budget.entry_count, 0u32);
    assert_eq!(budget.estimated_bytes, 0u32);
    assert_eq!(budget.estimated_ttl_costs, 0i128);
    assert_eq!(budget.projected_monthly_cost, 0i128);
}

/// get_storage_budget reflects accumulated entries.
#[test]
fn test_storage_budget_grows_with_entries() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    let n: u32 = 3;
    for i in 0..n {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 100,
            protocol_version: 26,
            sequence_number: i + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset, &((i + 1) as i128 * 100_000), &(1000 + i as u64 * 100));
    }

    let budget = client.get_storage_budget(&asset);
    assert_eq!(budget.entry_count, n, "entry_count must match submitted count");
    assert!(budget.estimated_bytes > 0u32);
    assert!(budget.estimated_ttl_costs >= 0i128);
    assert!(budget.projected_monthly_cost >= budget.estimated_ttl_costs);
}

/// get_total_storage_budget aggregates across assets.
#[test]
fn test_total_storage_budget_aggregates() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    // 2 entries for asset1, 1 for asset2.
    for i in 0..2u32 {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 100,
            protocol_version: 26,
            sequence_number: i + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset1, &((i + 1) as i128 * 100), &(1000 + i as u64 * 100));
    }
    e.ledger().set(soroban_sdk::testutils::LedgerInfo {
        timestamp: 2000,
        protocol_version: 26,
        sequence_number: 10,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
    client.submit_price(&source, &asset2, &999i128, &2000u64);

    let total = client.get_total_storage_budget();
    assert_eq!(total.asset_count, 2u32);
    assert_eq!(total.total_entry_count, 3u32, "total entries must be 2+1=3");
    assert!(total.total_estimated_bytes > 0u32);
}

/// get_storage_budget panics for unregistered asset.
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_storage_budget_unregistered_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let fake_asset = Address::generate(&e);
    client.get_storage_budget(&fake_asset);
}

/// Individual asset budgets sum to the total.
#[test]
fn test_total_budget_equals_sum_of_individuals() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    for i in 0..2u32 {
        e.ledger().set(soroban_sdk::testutils::LedgerInfo {
            timestamp: 1000 + i as u64 * 100,
            protocol_version: 26,
            sequence_number: i + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 6_312_000,
        });
        client.submit_price(&source, &asset1, &((i + 1) as i128 * 1000), &(1000 + i as u64 * 100));
        client.submit_price(&source, &asset2, &((i + 1) as i128 * 500), &(1000 + i as u64 * 100));
    }

    let b1 = client.get_storage_budget(&asset1);
    let b2 = client.get_storage_budget(&asset2);
    let total = client.get_total_storage_budget();

    assert_eq!(
        total.total_entry_count,
        b1.entry_count + b2.entry_count
    );
    assert_eq!(
        total.total_estimated_bytes,
        b1.estimated_bytes + b2.estimated_bytes
    );
}
