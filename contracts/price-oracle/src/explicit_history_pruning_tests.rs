#![cfg(test)]

//! Tests for explicit operator-triggered history pruning (`prune_history`).
//!
//! `prune_history(asset, target_entries)` lets an admin proactively prune the
//! oldest history entries for an asset down to a target count, independent of
//! the automatic pruning that runs during aggregation.

use soroban_sdk::{
    testutils::{Address as _, Events},
    Address, Env,
};

use crate::test_helpers::*;
use crate::PriceOracleContractClient;

/// Returns the number of contract events emitted in the most recent invocation.
fn event_count(e: &Env) -> usize {
    e.events().all().events().len()
}

/// Submits `count` prices for `asset` from `source`, one per distinct ledger,
/// starting at sequence 1.
fn seed_history(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    source: &Address,
    asset: &Address,
    count: u32,
) {
    for i in 0..count {
        ledger_default(e, i + 1, 1000 + (i as u64) * 100);
        submit_test_price_n(client, source, asset, 100_000 + i as i128, 1000 + (i as u64) * 100, (i + 1) as u64);
    }
}

#[test]
fn test_prune_history_reduces_to_target_and_returns_count() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = setup_basic(&e);

    seed_history(&e, &client, &source, &asset, 5);

    let (entries_before, _) = client.get_historical_prices_paginated(&asset, &0u32, &50u32);
    assert_eq!(entries_before.len(), 5);

    let pruned = client.prune_history(&asset, &2u32);
    assert_eq!(pruned, 3);

    let (entries_after, _) = client.get_historical_prices_paginated(&asset, &0u32, &50u32);
    assert_eq!(entries_after.len(), 2);

    // The two remaining entries must be the two most recent (highest ledger/price).
    assert_eq!(entries_after.get_unchecked(0).price, 100_003);
    assert_eq!(entries_after.get_unchecked(1).price, 100_004);
}

#[test]
fn test_prune_history_emits_one_event_per_pruned_entry() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = setup_basic(&e);

    seed_history(&e, &client, &source, &asset, 4);

    let pruned = client.prune_history(&asset, &1u32);
    assert_eq!(pruned, 3);
    assert_eq!(event_count(&e), 3, "one HistoryPrunedEvent per pruned entry");
}

#[test]
fn test_prune_history_noop_when_target_at_or_above_count() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = setup_basic(&e);

    seed_history(&e, &client, &source, &asset, 3);

    // Target equal to current count: no-op.
    let pruned = client.prune_history(&asset, &3u32);
    assert_eq!(pruned, 0);

    // Target above current count: still a no-op.
    let pruned = client.prune_history(&asset, &10u32);
    assert_eq!(pruned, 0);

    let (entries, _) = client.get_historical_prices_paginated(&asset, &0u32, &50u32);
    assert_eq!(entries.len(), 3);
}

#[test]
fn test_prune_history_down_to_zero() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = setup_basic(&e);

    seed_history(&e, &client, &source, &asset, 4);

    let pruned = client.prune_history(&asset, &0u32);
    assert_eq!(pruned, 4);

    let (entries, _) = client.get_historical_prices_paginated(&asset, &0u32, &50u32);
    assert_eq!(entries.len(), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_prune_history_requires_admin() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = setup_basic(&e);

    seed_history(&e, &client, &source, &asset, 3);

    clear_auth(&e);
    client.prune_history(&asset, &1u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_prune_history_unregistered_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let unregistered = Address::generate(&e);

    client.prune_history(&unregistered, &1u32);
}

#[test]
fn test_prune_history_is_separate_from_auto_pruning() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin, source, asset) = setup_basic(&e);

    // setup_basic configures max_history_length = 10, well above this seed
    // count, so auto-pruning never kicks in here — only the explicit call
    // below removes entries.
    seed_history(&e, &client, &source, &asset, 6);
    let (entries, _) = client.get_historical_prices_paginated(&asset, &0u32, &50u32);
    assert_eq!(entries.len(), 6, "auto-pruning must not have triggered");

    let pruned = client.prune_history(&asset, &4u32);
    assert_eq!(pruned, 2);

    let (entries, _) = client.get_historical_prices_paginated(&asset, &0u32, &50u32);
    assert_eq!(entries.len(), 4);
}
