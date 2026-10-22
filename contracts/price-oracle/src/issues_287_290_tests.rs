//! Tests for Issues #287, #288, #289, #290
//!
//! #287 — SEP-40 price verification helpers
//! #288 — Timestamp-based price pruning
//! #289 — Subscription auto-renewal
//! #290 — Price submission scheduling

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env,
};

use crate::test_helpers::{setup_basic, setup_contract};
use crate::PriceOracleContractClient;

// ─── Helpers ────────────────────────────────────────────────────────────────

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

fn make_client(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address, Address) {
    setup_basic(e)
}

// ══════════════════════════════════════════════════════════════════════════════
// Issue #287 — SEP-40 Price Verification Helpers
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_verify_price_freshness_fresh() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Price is 0 seconds old — should be fresh with max_age = 60
    let is_fresh = client.verify_price_freshness(&asset, &60u64);
    assert!(is_fresh);
}

#[test]
fn test_verify_price_freshness_stale() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Advance time by 120 seconds — price is now 120 s old, max_age = 60 → stale
    set_ledger(&e, 200, 1_120);
    let is_fresh = client.verify_price_freshness(&asset, &60u64);
    assert!(!is_fresh);
}

#[test]
fn test_verify_price_freshness_zero_max_age_always_passes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = make_client(&e);

    // No price submitted at all — but max_age = 0 should always return true
    let is_fresh = client.verify_price_freshness(&asset, &0u64);
    assert!(is_fresh);
}

#[test]
fn test_verify_price_freshness_no_price_returns_false() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    // No price submitted — should return false (not fresh)
    let is_fresh = client.verify_price_freshness(&asset, &60u64);
    assert!(!is_fresh);
}

#[test]
fn test_verify_price_deviation_within_tolerance() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    // 100 vs 101 → deviation = 100/101 * 10000 ≈ 99 bp < 200 bp
    let within = client.verify_price_deviation(&100_i128, &101_i128, &200u32);
    assert!(within);
}

#[test]
fn test_verify_price_deviation_outside_tolerance() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    // 100 vs 120 → deviation = 20/120 * 10000 = 1666 bp > 500 bp
    let within = client.verify_price_deviation(&100_i128, &120_i128, &500u32);
    assert!(!within);
}

#[test]
fn test_verify_price_deviation_equal_prices() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    let within = client.verify_price_deviation(&500_i128, &500_i128, &0u32);
    assert!(within); // 0 deviation, 0 tolerance → within
}

#[test]
fn test_verify_cross_oracle_within_tolerance() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Reference oracle has 1_001_000 — deviation ~100 bp < 200 bp
    let within = client.verify_cross_oracle(&asset, &1_001_000_i128, &200u32);
    assert!(within);
}

#[test]
fn test_verify_cross_oracle_outside_tolerance() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Reference = 1_100_000 → 10% deviation > 5% (500 bp) tolerance
    let within = client.verify_cross_oracle(&asset, &1_100_000_i128, &500u32);
    assert!(!within);
}

#[test]
fn test_verify_cross_oracle_no_aggregate_returns_false() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = make_client(&e);

    let within = client.verify_cross_oracle(&asset, &1_000_000_i128, &500u32);
    assert!(!within);
}

#[test]
fn test_get_oracle_decimals() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    assert_eq!(client.get_oracle_decimals(), 18u32);
}

// ══════════════════════════════════════════════════════════════════════════════
// Issue #288 — Timestamp-Based Price Pruning
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_set_and_get_retention_window() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = make_client(&e);

    client.set_asset_retention_window(&asset, &3600u64); // 1 hour
    assert_eq!(client.get_asset_retention_window(&asset), 3600u64);
}

#[test]
fn test_retention_window_defaults_to_zero() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = make_client(&e);

    assert_eq!(client.get_asset_retention_window(&asset), 0u64);
}

#[test]
fn test_prune_by_timestamp_removes_stale_entries() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    // Submit price at timestamp 1000
    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Set retention window of 30 seconds
    client.set_asset_retention_window(&asset, &30u64);

    // Advance time, but stay within ledger TTL window (seq 105, ts 1100)
    // now = 1100, cutoff = 1070, entry ts = 1000 < 1070 → should prune
    set_ledger(&e, 105, 1_100);
    let pruned = client.prune_history_by_timestamp(&asset);
    assert_eq!(pruned, 1u32);
}

#[test]
fn test_prune_by_timestamp_keeps_fresh_entries() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Set retention window of 3600 seconds (1 hour)
    client.set_asset_retention_window(&asset, &3600u64);

    // Advance only 10 seconds — price is still within window
    set_ledger(&e, 101, 1_010);
    let pruned = client.prune_history_by_timestamp(&asset);
    assert_eq!(pruned, 0u32);
}

#[test]
fn test_prune_no_window_configured_is_noop() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // No retention window set — should prune 0 entries
    set_ledger(&e, 200, 9_999_999);
    let pruned = client.prune_history_by_timestamp(&asset);
    assert_eq!(pruned, 0u32);
}

#[test]
fn test_remove_retention_window() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, asset) = make_client(&e);

    client.set_asset_retention_window(&asset, &3600u64);
    assert_eq!(client.get_asset_retention_window(&asset), 3600u64);

    client.remove_asset_retention_window(&asset);
    assert_eq!(client.get_asset_retention_window(&asset), 0u64);
}

#[test]
fn test_combined_pruning_respects_both_ledger_and_timestamp() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    // Set max history to 10 (plenty of room) and timestamp retention of 50 seconds
    client.set_max_history_length(&10u32);
    client.set_asset_retention_window(&asset, &50u64);

    // Submit two prices with different timestamps
    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000_000, &1_000); // entry ts=1000

    set_ledger(&e, 101, 1_010);
    client.submit_price(&source, &asset, &1_010_000, &1_010); // entry ts=1010

    // At seq=103, ts=1060: cutoff = 1060-50 = 1010
    // Entry at ts=1000 < 1010 → stale → should be pruned
    // Entry at ts=1010 is NOT < 1010 → kept
    set_ledger(&e, 103, 1_060);
    let pruned = client.prune_history_by_timestamp(&asset);
    assert_eq!(pruned, 1u32);
}

// ══════════════════════════════════════════════════════════════════════════════
// Issue #289 — Subscription Auto-Renewal
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_subscribe_basic() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);
    client.subscribe(&subscriber, &3600u64); // 1-hour subscription

    let record = client.get_subscription(&subscriber);
    assert!(record.is_some());
    let r = record.unwrap();
    assert_eq!(r.expires_at, 4_600u64); // 1000 + 3600
    assert_eq!(r.period_seconds, 3600u64);
}

#[test]
fn test_is_subscription_active() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);
    client.subscribe(&subscriber, &3600u64);

    assert!(client.is_subscription_active(&subscriber));

    // Advance time past expiry
    set_ledger(&e, 200, 5_000);
    assert!(!client.is_subscription_active(&subscriber));
}

#[test]
fn test_subscribe_extends_existing() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);

    // First subscription: expires at 2000
    client.subscribe(&subscriber, &1_000u64);
    // Second subscription while still active: extends by another 1000 s → expires at 3000
    client.subscribe(&subscriber, &1_000u64);

    let r = client.get_subscription(&subscriber).unwrap();
    assert_eq!(r.expires_at, 3_000u64);
}

#[test]
fn test_approve_and_revoke_renewal() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);
    client.subscribe(&subscriber, &3600u64);

    client.approve_renewal(&subscriber, &5u32, &300u64);
    let approval = client.get_renewal_approval(&subscriber);
    assert!(approval.is_some());
    let a = approval.unwrap();
    assert_eq!(a.max_renewals, 5u32);
    assert_eq!(a.renewal_threshold_seconds, 300u64);

    client.revoke_renewal(&subscriber);
    assert!(client.get_renewal_approval(&subscriber).is_none());
}

#[test]
#[should_panic]
fn test_approve_renewal_no_subscription_panics() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    let subscriber = Address::generate(&e);
    // Should panic — no subscription exists
    client.approve_renewal(&subscriber, &5u32, &300u64);
}

#[test]
fn test_check_and_renew_valid_subscription() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);
    client.subscribe(&subscriber, &3600u64);

    // Still valid — check_and_renew should return true without renewing
    let result = client.check_and_renew(&subscriber);
    assert!(result);
}

#[test]
fn test_check_and_renew_triggers_renewal_near_expiry() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);
    client.subscribe(&subscriber, &3600u64); // expires at 4600

    // Approve auto-renewal with threshold of 500 seconds
    client.approve_renewal(&subscriber, &3u32, &500u64);

    // Advance to 4200 — only 400 seconds left, within 500s threshold → should renew
    set_ledger(&e, 200, 4_200);
    let result = client.check_and_renew(&subscriber);
    assert!(result);

    // Subscription should now be extended
    let r = client.get_subscription(&subscriber).unwrap();
    assert!(r.expires_at > 4_600u64); // extended
}

#[test]
fn test_check_and_renew_no_subscription_returns_false() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    let subscriber = Address::generate(&e);
    let result = client.check_and_renew(&subscriber);
    assert!(!result);
}

#[test]
fn test_auto_renewal_budget_exhausted() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);
    client.subscribe(&subscriber, &100u64); // expires at 1100

    // Only 1 auto-renewal allowed
    client.approve_renewal(&subscriber, &1u32, &50u64);

    // Advance to 1060 — within threshold → renews once (expires 1200)
    set_ledger(&e, 200, 1_060);
    let r1 = client.check_and_renew(&subscriber);
    assert!(r1);
    let after_renewal = client.get_subscription(&subscriber).unwrap();
    assert!(after_renewal.expires_at > 1_100u64);

    // Advance to near expiry again — budget exhausted → no more renewals
    set_ledger(&e, 300, after_renewal.expires_at - 10);
    let r2 = client.check_and_renew(&subscriber);
    // Still valid (not yet expired), but no renewal triggered — returns true
    assert!(r2);
    let still_same = client.get_subscription(&subscriber).unwrap();
    assert_eq!(still_same.expires_at, after_renewal.expires_at); // unchanged
}

#[test]
fn test_admin_remove_expired_subscription() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, _source, _asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    let subscriber = Address::generate(&e);
    client.subscribe(&subscriber, &100u64); // expires at 1100

    // Advance past expiry
    set_ledger(&e, 200, 1_200);
    client.admin_remove_subscription(&subscriber);

    assert!(client.get_subscription(&subscriber).is_none());
}

// ══════════════════════════════════════════════════════════════════════════════
// Issue #290 — Price Submission Scheduling
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn test_register_and_get_schedule_ledger_based() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    // kind=0 = every-N-ledgers, interval=50, multiplier=2
    client.register_schedule(&source, &asset, &0u32, &50u64, &2u32);

    let sched = client.get_schedule(&source, &asset);
    assert!(sched.is_some());
    let s = sched.unwrap();
    assert_eq!(s.interval, 50u64);
    assert_eq!(s.deadline_multiplier, 2u32);
}

#[test]
fn test_register_and_get_schedule_seconds_based() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.register_schedule(&source, &asset, &1u32, &300u64, &2u32);

    let sched = client.get_schedule(&source, &asset).unwrap();
    assert_eq!(sched.interval, 300u64);
}

#[test]
fn test_schedule_on_time_submission() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.register_schedule(&source, &asset, &0u32, &50u64, &2u32);

    // First submission — always on time
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Check liveness — first submission, no previous record, should be on time
    set_ledger(&e, 140, 1_040);
    let live = client.check_source_liveness(&source, &asset);
    assert!(live); // gap = 40 ledgers ≤ 50*2 = 100 → on time
}

#[test]
fn test_schedule_violation_detected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.register_schedule(&source, &asset, &0u32, &50u64, &2u32); // deadline = 100 ledgers
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // Advance 200 ledgers — gap (200) > deadline (100) → violation
    set_ledger(&e, 300, 2_000);
    let live = client.check_source_liveness(&source, &asset);
    assert!(!live);
}

#[test]
fn test_schedule_no_schedule_always_live() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    // No schedule registered — always live
    let live = client.check_source_liveness(&source, &asset);
    assert!(live);
}

#[test]
fn test_remove_schedule_by_source() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.register_schedule(&source, &asset, &1u32, &300u64, &1u32);
    assert!(client.get_schedule(&source, &asset).is_some());

    client.remove_schedule(&source, &asset);
    assert!(client.get_schedule(&source, &asset).is_none());
}

#[test]
fn test_admin_remove_schedule() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.register_schedule(&source, &asset, &1u32, &300u64, &1u32);
    client.admin_remove_schedule(&source, &asset);
    assert!(client.get_schedule(&source, &asset).is_none());
}

#[test]
fn test_get_last_submission_record() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    client.register_schedule(&source, &asset, &0u32, &50u64, &2u32);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    let rec = client.get_last_submission(&source, &asset);
    assert!(rec.is_some());
    let r = rec.unwrap();
    assert_eq!(r.ledger, 100u32);
    assert_eq!(r.timestamp, 1_000u64);
}

#[test]
fn test_schedule_seconds_based_violation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    set_ledger(&e, 100, 1_000);
    // every 60 seconds, deadline multiplier 2 → deadline = 120 s
    client.register_schedule(&source, &asset, &1u32, &60u64, &2u32);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    // 200 seconds later — gap = 200 > 120 → violation
    set_ledger(&e, 300, 1_200);
    let live = client.check_source_liveness(&source, &asset);
    assert!(!live);
}

#[test]
#[should_panic]
fn test_schedule_invalid_config_zero_interval_panics() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = make_client(&e);

    // Should panic — interval of 0 is invalid
    client.register_schedule(&source, &asset, &0u32, &0u64, &1u32);
}
