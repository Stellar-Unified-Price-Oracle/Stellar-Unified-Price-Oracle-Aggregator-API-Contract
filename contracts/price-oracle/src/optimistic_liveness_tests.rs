#![cfg(test)]
//! #462 — Optimistic-path dispute liveness and griefing.
//!
//! Liveness assumption: an honest watcher must dispute within the dispute window
//! (default 120 ledgers). A dispute may stay unresolved for at most
//! `MAX_DISPUTE_DURATION` ledgers past the window, after which the proposal is
//! rejected and its price is never served.

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env};

use crate::optimistic::MAX_DISPUTE_DURATION;
use crate::test_helpers::{ledger_default, register_test_asset, setup_contract};

const BOND: i128 = 100_000_000;
const WINDOW: u32 = 120;
const PENDING: u32 = 0;
const FINALIZED: u32 = 1;
const DISPUTED: u32 = 2;
const RESOLVED: u32 = 3;

#[test]
fn liveness_window_boundary() {
    let e = Env::default();
    ledger_default(&e, 10, 1_000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let id = client.propose_price(&Address::generate(&e), &asset, &500i128, &1_000u64, &BOND);

    // Last ledger inside the window: still disputable, value not served.
    ledger_default(&e, 10 + WINDOW - 1, 1_500);
    assert_eq!(client.get_proposal(&id).unwrap().status, PENDING);
    assert!(client.get_price(&asset, &0u64).is_none());
    client.dispute_proposal(&Address::generate(&e), &id);
    assert_eq!(client.get_proposal(&id).unwrap().status, DISPUTED);
}

#[test]
fn dispute_after_window_is_rejected() {
    let e = Env::default();
    ledger_default(&e, 10, 1_000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let id = client.propose_price(&Address::generate(&e), &asset, &500i128, &1_000u64, &BOND);
    ledger_default(&e, 10 + WINDOW, 1_600);
    assert!(client
        .try_dispute_proposal(&Address::generate(&e), &id)
        .is_err());
    assert_eq!(client.get_proposal(&id).unwrap().status, FINALIZED);
}

#[test]
fn perpetual_dispute_is_bounded_and_fails_safe() {
    let e = Env::default();
    ledger_default(&e, 10, 1_000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let id = client.propose_price(&Address::generate(&e), &asset, &500i128, &1_000u64, &BOND);
    client.dispute_proposal(&Address::generate(&e), &id);

    let expires = client.get_proposal(&id).unwrap().expires_at_ledger;
    ledger_default(&e, expires + MAX_DISPUTE_DURATION - 1, 2_000);
    assert_eq!(client.get_proposal(&id).unwrap().status, DISPUTED);

    ledger_default(&e, expires + MAX_DISPUTE_DURATION, 3_000);
    let p = client.get_proposal(&id).unwrap();
    assert_eq!(p.status, RESOLVED);
    assert_eq!(p.resolution, 2);
    // Never served, and can no longer be force-approved.
    assert!(client.get_price(&asset, &0u64).is_none());
    assert!(client.try_resolve_dispute(&id, &true).is_err());
    assert_eq!(client.get_active_proposals().len(), 0);
}

#[test]
fn dispute_cycling_cannot_extend_window() {
    let e = Env::default();
    ledger_default(&e, 10, 1_000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let id = client.propose_price(&Address::generate(&e), &asset, &500i128, &1_000u64, &BOND);
    let expires = client.get_proposal(&id).unwrap().expires_at_ledger;

    client.dispute_proposal(&Address::generate(&e), &id);
    // No withdraw exists; a second dispute is rejected and the window is unchanged.
    assert!(client
        .try_dispute_proposal(&Address::generate(&e), &id)
        .is_err());
    assert_eq!(client.get_proposal(&id).unwrap().expires_at_ledger, expires);
}

#[test]
fn optimistic_value_is_not_multi_round_final() {
    let e = Env::default();
    ledger_default(&e, 10, 1_000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let id = client.propose_price(&Address::generate(&e), &asset, &500i128, &1_000u64, &BOND);

    // Pending/disputed optimistic values are served by neither path.
    client.dispute_proposal(&Address::generate(&e), &id);
    assert!(client.get_price(&asset, &0u64).is_none());
    assert!(client.try_get_finalized_price(&asset, &0u32).is_err());

    // Approval serves the value, but economic finality still requires its own window.
    client.resolve_dispute(&id, &true);
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 500);
    assert!(client.try_get_finalized_price(&asset, &0u32).is_err());
}

#[test]
fn rejected_dispute_serves_nothing() {
    let e = Env::default();
    ledger_default(&e, 10, 1_000);
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let id = client.propose_price(&Address::generate(&e), &asset, &500i128, &1_000u64, &BOND);
    client.dispute_proposal(&Address::generate(&e), &id);
    client.resolve_dispute(&id, &false);
    assert!(client.get_price(&asset, &0u64).is_none());
}
