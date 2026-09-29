#![cfg(test)]

//! # #410 — Chaos engineering test suite
//!
//! Each fault (reorg, partition, delayed submission, ledger-time skew, storage
//! pressure, TTL eviction) is paired with a hostile variant: an adversary who
//! tries to exploit the disorder.  The invariants held here are listed, with
//! the fault each one survives, in `docs/chaos-invariants.md`.

use soroban_sdk::{
    testutils::{Ledger, LedgerInfo},
    Env,
};

use crate::test_helpers::*;
use crate::ErrorCode;

fn set_ledger(e: &Env, seq: u32, timestamp: u64) {
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

fn advance_ledger(e: &Env, seq: u32) {
    set_ledger(e, seq, (seq as u64) * 5);
}

fn err(code: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(code as u32)
}

// ---------------------------------------------------------------------------
// INV-1: no double finalization, and a reorg-retracted price never finalizes
// ---------------------------------------------------------------------------

#[test]
fn chaos_reorg_retracted_price_never_finalizes() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    client.set_finality_ledgers(&10u32);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);
    client.mark_price_pending(&asset);

    // Reorg observed off-chain: admin retracts before finality.
    advance_ledger(&e, 12);
    client.retract_price(&asset, &10u32);

    // Past the window the retracted entry still cannot be finalized.
    advance_ledger(&e, 40);
    assert_eq!(
        client.try_try_finalize_price(&asset, &10u32),
        Err(Ok(err(ErrorCode::PriceRetracted)))
    );
    assert_eq!(
        client.try_get_finalized_price(&asset, &0u32),
        Err(Ok(err(ErrorCode::NoData)))
    );
}

#[test]
fn chaos_reorg_hostile_resurrection_of_retracted_price_rejected() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    client.set_finality_ledgers(&10u32);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);
    client.mark_price_pending(&asset);
    client.retract_price(&asset, &10u32);

    // Adversary: `mark_price_pending` is permissionless; calling it again in
    // the same ledger must not reset the retracted entry to Pending.
    assert_eq!(
        client.try_mark_price_pending(&asset),
        Err(Ok(err(ErrorCode::PriceRetracted)))
    );
    advance_ledger(&e, 40);
    assert!(client.try_try_finalize_price(&asset, &10u32).is_err());
}

#[test]
fn chaos_reorg_hostile_non_admin_cannot_retract_finalized_or_pending() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    client.set_finality_ledgers(&10u32);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);
    client.mark_price_pending(&asset);

    // Adversary fakes a reorg signal without admin authorization.
    clear_auth(&e);
    assert!(client.try_retract_price(&asset, &10u32).is_err());

    // Finalization is permissionless and happens exactly once.
    advance_ledger(&e, 21);
    assert!(client.try_finalize_price(&asset, &10u32));
    assert_eq!(
        client.try_try_finalize_price(&asset, &10u32),
        Err(Ok(err(ErrorCode::AlreadyFinalized)))
    );
    let fp = client.get_finalized_price(&asset, &0u32);
    assert_eq!((fp.price, fp.committed_ledger), (1_000i128, 10u32));
}

// ---------------------------------------------------------------------------
// INV-2: out-of-order finalization never rolls the finalized price back
// ---------------------------------------------------------------------------

#[test]
fn chaos_out_of_order_finalization_keeps_newest_price() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    client.set_finality_ledgers(&10u32);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);
    client.mark_price_pending(&asset);

    advance_ledger(&e, 15);
    client.submit_price(&source, &asset, &2_000i128, &75u64);
    client.mark_price_pending(&asset);

    // Hostile/delayed finalizer processes the newer ledger first, then the
    // older one, hoping to roll consumers back to the stale price.
    advance_ledger(&e, 30);
    assert!(client.try_finalize_price(&asset, &15u32));
    assert!(client.try_finalize_price(&asset, &10u32));

    let fp = client.get_finalized_price(&asset, &0u32);
    assert_eq!((fp.price, fp.committed_ledger), (2_000i128, 15u32));
}

// ---------------------------------------------------------------------------
// INV-3: a delayed submission never silently overwrites a newer one
// ---------------------------------------------------------------------------

#[test]
fn chaos_delayed_submission_rejected_loudly() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    set_ledger(&e, 100, 1_000);

    client.submit_price(&source, &asset, &1_000i128, &900u64);
    // Network delay: an older message (ts 800) lands after the newer one.
    assert_eq!(
        client.try_submit_price(&source, &asset, &5i128, &800u64),
        Err(Ok(err(ErrorCode::InvalidTimestamp)))
    );
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 1_000i128);
}

#[test]
fn chaos_delayed_submission_hostile_replay_with_explicit_nonce_rejected() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    set_ledger(&e, 100, 1_000);

    submit_test_price_n(&client, &source, &asset, 1_000, 900, 10);
    // Adversary re-orders a captured older message under a fresh nonce.
    assert!(client
        .try_submit_price_with_nonce(&source, &asset, &1i128, &800u64, &11u64)
        .is_err());
    // And a replay of the original nonce is refused as well.
    assert!(client
        .try_submit_price_with_nonce(&source, &asset, &1i128, &900u64, &10u64)
        .is_err());
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 1_000i128);
}

// ---------------------------------------------------------------------------
// INV-4: a partitioned minority cannot aggregate or move the median
// ---------------------------------------------------------------------------

#[test]
fn chaos_partition_minority_cannot_aggregate() {
    let e = Env::default();
    let (client, _admin, sources, assets) = setup_full_oracle(&e, 4, 1);
    client.set_min_sources_required(&3u32);
    let asset = assets.get_unchecked(0);
    set_ledger(&e, 100, 1_000);

    // Only half of the source set is reachable: no quorum, no price.
    client.submit_price(&sources.get_unchecked(0), &asset, &1_000i128, &1_000u64);
    client.submit_price(&sources.get_unchecked(1), &asset, &1_000i128, &1_000u64);
    assert!(client.get_price(&asset, &0u64).is_none());
}

#[test]
fn chaos_partition_hostile_minority_cannot_move_median() {
    let e = Env::default();
    let (client, _admin, sources, assets) = setup_full_oracle(&e, 3, 1);
    let asset = assets.get_unchecked(0);
    set_ledger(&e, 100, 1_000);

    client.submit_price(&sources.get_unchecked(0), &asset, &1_000i128, &1_000u64);
    client.submit_price(&sources.get_unchecked(1), &asset, &1_010i128, &1_000u64);
    // A hostile source exploits the partition to push an extreme value.
    client.submit_price(&sources.get_unchecked(2), &asset, &9_000_000i128, &1_000u64);

    let price = client.get_price(&asset, &0u64).unwrap().price;
    assert!((1_000..=1_010).contains(&price), "median moved to {price}");
}

// ---------------------------------------------------------------------------
// INV-5: ledger-time skew is handled conservatively in both directions
// ---------------------------------------------------------------------------

#[test]
fn chaos_skew_source_clock_ahead_bounded_by_threshold() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    set_ledger(&e, 100, 10_000);
    let threshold = client.get_timestamp_threshold();

    // Hostile source stamps beyond the threshold to extend freshness: rejected.
    assert_eq!(
        client.try_submit_price(&source, &asset, &1_000i128, &(10_000 + threshold + 1)),
        Err(Ok(err(ErrorCode::InvalidTimestamp)))
    );

    // At most `threshold` seconds of extra freshness can be gained.
    let max_age = 60u64;
    client.submit_price(&source, &asset, &1_000i128, &(10_000 + threshold));
    set_ledger(&e, 101, 10_000 + threshold + max_age);
    assert!(client.get_price(&asset, &max_age).is_some());
    set_ledger(&e, 102, 10_000 + threshold + max_age + 1);
    assert!(client.get_price(&asset, &max_age).is_none());
}

#[test]
fn chaos_skew_ledger_clock_jumps_forward_fails_closed() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    set_ledger(&e, 100, 10_000);
    client.submit_price(&source, &asset, &1_000i128, &10_000u64);

    // Ledger time leaps forward: the price must be reported stale, not served.
    set_ledger(&e, 101, 10_000 + 3_600);
    assert!(client.get_price(&asset, &60u64).is_none());
}

// ---------------------------------------------------------------------------
// INV-6: storage pressure is bounded; TTL eviction fails loudly
// ---------------------------------------------------------------------------

#[test]
fn chaos_storage_flood_history_stays_bounded() {
    let e = Env::default();
    // `setup_contract` caps history at 10 entries.
    let (client, _admin, source, asset) = setup_basic(&e);

    // Hostile source floods submissions across many ledgers.
    for i in 1..=50u32 {
        set_ledger(&e, 100 + i, 1_000 + i as u64);
        client.submit_price(&source, &asset, &(1_000 + i as i128), &(1_000 + i as u64));
    }
    assert!(client.get_price_history(&asset, &0u32, &10u32).len() <= 10);
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 1_050i128);
}

#[test]
fn chaos_ttl_lapse_fails_closed_and_blocks_rewrite() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    set_ledger(&e, 100, 1_000);
    client.submit_price(&source, &asset, &1_000i128, &1_000u64);

    // Long inactivity: the ledger moves past every entry's TTL bump window.
    set_ledger(&e, 100 + 5_000_000, 1_000 + 25_000_000);

    // Consumers that bound staleness are refused the old price.
    assert!(client.get_price(&asset, &3_600u64).is_none());
    // Adversary tries to re-write the lapsed history with an older value.
    assert_eq!(
        client.try_submit_price(&source, &asset, &1i128, &900u64),
        Err(Ok(err(ErrorCode::InvalidTimestamp)))
    );
}
