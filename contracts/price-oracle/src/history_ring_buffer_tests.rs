//! # #877 — Ring-buffer history storage tests
//!
//! Tests for ring-buffer circular history storage prototype and migration.

#![cfg(test)]

use soroban_sdk::{
    testutils::Address as _,
    Address, Env,
};

use crate::history_ring_buffer::{RingBufferHistory, RingBufferMeta};
use crate::types::PriceHistoryEntry;

fn make_entry(asset: &Address, ledger: u32, price: i128) -> PriceHistoryEntry {
    PriceHistoryEntry {
        asset: asset.clone(),
        price,
        timestamp: 1_700_000_000 + ledger as u64,
        ledger,
        num_sources: 3,
    }
}

#[test]
fn test_ring_buffer_push_and_direct_lookup() {
    let e = Env::default();
    let asset = Address::generate(&e);
    let capacity = 4;

    for i in 0..4 {
        let entry = make_entry(&asset, i, 100 + i as i128);
        RingBufferHistory::push(&e, &asset, entry, capacity);
    }

    // Direct lookup works for all slots
    for i in 0..4 {
        let entry = RingBufferHistory::get_by_ledger(&e, &asset, i);
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().ledger, i);
    }
}

#[test]
fn test_ring_buffer_wraparound_overwrites_oldest() {
    let e = Env::default();
    let asset = Address::generate(&e);
    let capacity = 3;

    // Push 5 entries (capacity is 3) -> slots 0 and 1 get overwritten
    for i in 0..5 {
        let entry = make_entry(&asset, i, 1_000 + i as i128);
        RingBufferHistory::push(&e, &asset, entry, capacity);
    }

    // Ledgers 0 and 1 are evicted (outside retained window [2, 5))
    assert!(RingBufferHistory::get_by_ledger(&e, &asset, 0).is_none());
    assert!(RingBufferHistory::get_by_ledger(&e, &asset, 1).is_none());

    // Ledgers 2, 3, 4 are present
    assert_eq!(RingBufferHistory::get_by_ledger(&e, &asset, 2).unwrap().price, 1_002);
    assert_eq!(RingBufferHistory::get_by_ledger(&e, &asset, 3).unwrap().price, 1_003);
    assert_eq!(RingBufferHistory::get_by_ledger(&e, &asset, 4).unwrap().price, 1_004);
}
