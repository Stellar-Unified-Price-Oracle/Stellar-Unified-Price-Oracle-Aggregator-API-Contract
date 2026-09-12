#![cfg(test)]

//! # #186 — Adaptive Heartbeat Tests
//!
//! Tests for:
//! - `compute_adaptive_interval` formula correctness
//! - Healthy → Degraded → Inactive transition after 3 consecutive misses
//! - Source reactivation requires BOTH a heartbeat AND a price submission
//! - Only heartbeat does NOT reactivate
//! - `check_and_prune_inactive_sources` auto-removes stale sources
//! - Race-condition guard: never removes sources below min_sources_required
//! - `get_source_health` returns correct variant at each transition

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
};

use crate::sources::compute_adaptive_interval;
use crate::test_helpers::*;
use crate::{PriceOracleContract, PriceOracleContractClient, SourceHealthStatus};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn advance_time(e: &Env, seq: u32, timestamp: u64) {
    e.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
}

// ---------------------------------------------------------------------------
// Unit: compute_adaptive_interval
// ---------------------------------------------------------------------------

#[test]
fn test_adaptive_interval_zero_misses() {
    // base * (window + 0) / window = base * 1 = base
    assert_eq!(compute_adaptive_interval(3600, 0, 10), 3600);
}

#[test]
fn test_adaptive_interval_partial_misses() {
    // base * (10 + 5) / 10 = base * 1 (integer division)
    assert_eq!(compute_adaptive_interval(3600, 5, 10), 3600);
    // base * (10 + 10) / 10 = base * 2
    assert_eq!(compute_adaptive_interval(3600, 10, 10), 7200);
}

#[test]
fn test_adaptive_interval_capped_at_3x() {
    // Even with many misses, should never exceed 3×
    assert_eq!(compute_adaptive_interval(3600, 100, 10), 10800);
    assert_eq!(compute_adaptive_interval(3600, 1000, 10), 10800);
}

#[test]
fn test_adaptive_interval_zero_window_safe() {
    // window = 0 should be treated as 1 to avoid division by zero
    assert_eq!(compute_adaptive_interval(3600, 0, 0), 3600);
}

// ---------------------------------------------------------------------------
// Integration: health status transitions
// ---------------------------------------------------------------------------

#[test]
fn test_source_starts_healthy() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let source = register_test_source(&e, &client, "Oracle-A");
    // Source has just been added, no heartbeats missed yet
    advance_time(&e, 10, 100);
    // Immediately after registration, within the base interval → Healthy
    client.submit_heartbeat(&source);
    let health = client.get_source_health(&source);
    assert_eq!(health, SourceHealthStatus::Healthy);
}

#[test]
fn test_source_degraded_after_one_miss() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    // Use a short heartbeat interval for testing
    client.set_heartbeat_interval(&60u64); // 60 seconds
    client.set_heartbeat_window(&10u32);

    let source = register_test_source(&e, &client, "Oracle-A");
    // Submit initial heartbeat
    advance_time(&e, 10, 100);
    client.submit_heartbeat(&source);

    // Advance past 1× adaptive interval (base * 1 = 60s)
    // The adaptive check happens lazily on the next is_source_inactive call
    advance_time(&e, 20, 200); // +100 seconds → interval expired

    // Check inactivity (triggers missed-heartbeat increment → 1 miss = Degraded)
    let is_inactive = client.is_source_inactive(&source);
    // 1 miss < MISS_THRESHOLD(3) → not yet inactive
    assert!(!is_inactive);
    assert_eq!(client.get_missed_heartbeats(&source), 1u32);
}

#[test]
fn test_source_becomes_inactive_after_3_misses() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_heartbeat_interval(&60u64);
    client.set_heartbeat_window(&10u32);

    let source = register_test_source(&e, &client, "Oracle-A");
    advance_time(&e, 10, 100);
    client.submit_heartbeat(&source);

    // Each call to is_source_inactive at a timestamp beyond the adaptive interval
    // increments the miss counter. We need to trigger it 3 times.
    // After each check, the adaptive interval grows:
    //   miss=0: interval=60s  → advance 70s
    //   miss=1: interval=60s  → advance 70s (still 1×, since 1/10 < 1)
    //   miss=2: interval=60s  → advance 70s

    // Miss 1
    advance_time(&e, 20, 200);
    client.is_source_inactive(&source);
    assert_eq!(client.get_missed_heartbeats(&source), 1u32);

    // Miss 2 — need to be past base interval from last heartbeat check
    advance_time(&e, 30, 300);
    client.is_source_inactive(&source);
    assert_eq!(client.get_missed_heartbeats(&source), 2u32);

    // Miss 3 → crosses MISS_THRESHOLD(3) → Inactive
    advance_time(&e, 40, 400);
    let is_inactive = client.is_source_inactive(&source);
    assert!(is_inactive);
    assert_eq!(client.get_missed_heartbeats(&source), 3u32);
    assert_eq!(
        client.get_source_health(&source),
        SourceHealthStatus::Inactive
    );
}

#[test]
fn test_reactivation_requires_heartbeat_and_price() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_heartbeat_interval(&60u64);
    client.set_heartbeat_window(&10u32);

    let source = register_test_source(&e, &client, "Oracle-A");
    let asset = register_test_asset(&e, &client);

    // Initial heartbeat
    advance_time(&e, 10, 100);
    client.submit_heartbeat(&source);

    // Force 3 misses → Inactive
    advance_time(&e, 20, 200);
    client.is_source_inactive(&source);
    advance_time(&e, 30, 300);
    client.is_source_inactive(&source);
    advance_time(&e, 40, 400);
    client.is_source_inactive(&source);
    assert_eq!(
        client.get_source_health(&source),
        SourceHealthStatus::Inactive
    );

    // Heartbeat alone should NOT reactivate (price not yet submitted after reactivation attempt)
    advance_time(&e, 50, 500);
    client.submit_heartbeat(&source);
    // Still inactive because price hasn't been submitted
    assert_eq!(
        client.get_source_health(&source),
        SourceHealthStatus::Inactive
    );

    // Now submit a price in the same ledger window
    client.submit_price(&source, &asset, &1000i128, &500u64);

    // Now submit another heartbeat — both conditions met, should reactivate
    advance_time(&e, 51, 510);
    client.submit_heartbeat(&source);
    assert_eq!(
        client.get_source_health(&source),
        SourceHealthStatus::Healthy
    );
    assert_eq!(client.get_missed_heartbeats(&source), 0u32);
}

#[test]
fn test_heartbeat_alone_does_not_reactivate() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_heartbeat_interval(&60u64);

    let source = register_test_source(&e, &client, "Oracle-A");
    advance_time(&e, 10, 100);
    client.submit_heartbeat(&source);

    // Force inactive
    advance_time(&e, 20, 200);
    client.is_source_inactive(&source);
    advance_time(&e, 30, 300);
    client.is_source_inactive(&source);
    advance_time(&e, 40, 400);
    client.is_source_inactive(&source);
    assert_eq!(
        client.get_source_health(&source),
        SourceHealthStatus::Inactive
    );

    // Heartbeat-only does not reactivate
    advance_time(&e, 50, 500);
    client.submit_heartbeat(&source);
    assert_eq!(
        client.get_source_health(&source),
        SourceHealthStatus::Inactive
    );
}

// ---------------------------------------------------------------------------
// Integration: auto-removal
// ---------------------------------------------------------------------------

#[test]
fn test_auto_removal_after_max_inactive_ledgers() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    // 2 sources so removing 1 doesn't break the oracle (min=1)
    client.set_min_sources_required(&1u32);
    client.set_heartbeat_interval(&60u64);
    client.set_max_inactive_ledgers(&5u32); // short for testing

    let s1 = register_test_source(&e, &client, "Source-1");
    let s2 = register_test_source(&e, &client, "Source-2");

    // s2 submits heartbeats regularly; s1 goes silent
    advance_time(&e, 10, 100);
    client.submit_heartbeat(&s1);
    client.submit_heartbeat(&s2);

    // Force s1 inactive at ledger 10
    advance_time(&e, 20, 200);
    client.is_source_inactive(&s1);
    advance_time(&e, 30, 300);
    client.is_source_inactive(&s1);
    advance_time(&e, 40, 400);
    client.is_source_inactive(&s1); // Now inactive, recorded inactive_since_ledger = 40

    // Advance past max_inactive_ledgers from ledger 40 → ledger 46
    advance_time(&e, 46, 460);
    client.submit_heartbeat(&s2); // keep s2 active

    let removed = client.check_and_prune_inactive_sources();
    assert_eq!(removed, 1u32);

    // s1 should no longer be a registered source
    assert!(!client.is_source(&s1));
    // s2 should still be registered
    assert!(client.is_source(&s2));
}

#[test]
fn test_auto_removal_race_condition_guard() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    // Only 1 source, min_sources = 1 — removing would break the oracle
    client.set_min_sources_required(&1u32);
    client.set_heartbeat_interval(&60u64);
    client.set_max_inactive_ledgers(&5u32);

    let source = register_test_source(&e, &client, "Lone-Source");

    // Force inactive
    advance_time(&e, 10, 100);
    client.submit_heartbeat(&source);
    advance_time(&e, 20, 200);
    client.is_source_inactive(&source);
    advance_time(&e, 30, 300);
    client.is_source_inactive(&source);
    advance_time(&e, 40, 400);
    client.is_source_inactive(&source);

    // Advance well past max_inactive_ledgers
    advance_time(&e, 60, 600);
    // Guard: removing this source would leave 0 active < min_sources(1) → skip
    let removed = client.check_and_prune_inactive_sources();
    assert_eq!(removed, 0u32);
    assert!(client.is_source(&source));
}

#[test]
fn test_get_source_health_of_unknown_source() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let unknown = Address::generate(&e);
    // Unknown/removed source → AutoRemoved
    let health = client.get_source_health(&unknown);
    assert_eq!(health, SourceHealthStatus::AutoRemoved);
}
