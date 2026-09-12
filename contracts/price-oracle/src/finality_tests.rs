#![cfg(test)]

//! # #188 — Economic Finality Gadget Tests
//!
//! Tests for:
//! - mark_price_pending: places aggregate in pending queue
//! - try_finalize_price: returns false before window, true after
//! - get_finalized_price: returns finalized price, error when absent
//! - get_finalized_price with min_finality constraint
//! - retract_price: admin can retract before finalization
//! - retract_price rejected after finalization (AlreadyFinalized)
//! - double-retract rejected (PriceRetracted)
//! - set/get_finality_ledgers configuration
//! - check_reorg returns false when no reorg present

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Env,
};

use crate::test_helpers::*;
use crate::{FinalityStatus, PriceOracleContract, PriceOracleContractClient};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn advance_ledger(e: &Env, seq: u32) {
    e.ledger().set(LedgerInfo {
        timestamp: (seq as u64) * 5,
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
// Configuration
// ---------------------------------------------------------------------------

#[test]
fn test_set_get_finality_ledgers() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_finality_ledgers(&32u32);
    assert_eq!(client.get_finality_ledgers(), 32u32);
}

#[test]
fn test_finality_ledgers_default_is_64() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    assert_eq!(client.get_finality_ledgers(), 64u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_set_finality_ledgers_zero_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_finality_ledgers(&0u32);
}

// ---------------------------------------------------------------------------
// mark_price_pending
// ---------------------------------------------------------------------------

#[test]
fn test_mark_price_pending_creates_entry() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&10u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);

    // Place pending
    client.mark_price_pending(&asset);

    // Entry exists and is Pending
    let status = client.get_finality_status(&asset, &10u32);
    assert_eq!(status, FinalityStatus::Pending);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_mark_price_pending_no_aggregate_panics() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    // No price ever submitted → no aggregate → NoData (#8)
    client.mark_price_pending(&asset);
}

// ---------------------------------------------------------------------------
// try_finalize_price
// ---------------------------------------------------------------------------

#[test]
fn test_try_finalize_before_window_returns_false() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);
    client.mark_price_pending(&asset);

    // Still inside window (need ledger >= 10 + 20 = 30)
    advance_ledger(&e, 25);
    let finalized = client.try_finalize_price(&asset, &10u32);
    assert!(!finalized);
}

#[test]
fn test_try_finalize_after_window_returns_true() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&10u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &2_000i128, &50u64);
    client.mark_price_pending(&asset);

    // Past finality: ledger >= 10 + 10 = 20
    advance_ledger(&e, 21);
    let finalized = client.try_finalize_price(&asset, &10u32);
    assert!(finalized);

    // Status no longer pending — finalized entry is now under FinalizedPrice key
    let fp = client.get_finalized_price(&asset, &0u32);
    assert_eq!(fp.price, 2_000i128);
    assert_eq!(fp.committed_ledger, 10u32);
}

// ---------------------------------------------------------------------------
// get_finalized_price with min_finality
// ---------------------------------------------------------------------------

#[test]
fn test_get_finalized_price_min_finality_passes() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&10u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &3_000i128, &50u64);
    client.mark_price_pending(&asset);

    advance_ledger(&e, 25);
    client.try_finalize_price(&asset, &10u32);

    // current_ledger(25) - committed_ledger(10) = 15 >= min_finality(10) → ok
    let fp = client.get_finalized_price(&asset, &10u32);
    assert_eq!(fp.price, 3_000i128);
}

#[test]
#[should_panic(expected = "Error(Contract, #43)")]
fn test_get_finalized_price_min_finality_fails() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&10u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &3_000i128, &50u64);
    client.mark_price_pending(&asset);

    advance_ledger(&e, 21);
    client.try_finalize_price(&asset, &10u32);

    // current_ledger(21) - committed_ledger(10) = 11 < min_finality(50) → InsufficientFinality (#43)
    client.get_finalized_price(&asset, &50u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_get_finalized_price_no_data() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    advance_ledger(&e, 10);
    // No finalized price ever written → NoData (#8)
    client.get_finalized_price(&asset, &0u32);
}

// ---------------------------------------------------------------------------
// retract_price
// ---------------------------------------------------------------------------

#[test]
fn test_retract_pending_price() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &5_000i128, &50u64);
    client.mark_price_pending(&asset);

    // Retract before finalization
    advance_ledger(&e, 15);
    client.retract_price(&asset, &10u32);

    // Status is now Retracted
    let status = client.get_finality_status(&asset, &10u32);
    assert_eq!(status, FinalityStatus::Retracted);
}

#[test]
#[should_panic(expected = "Error(Contract, #40)")]
fn test_retract_after_finalization_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&10u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &5_000i128, &50u64);
    client.mark_price_pending(&asset);

    // Finalize it
    advance_ledger(&e, 21);
    client.try_finalize_price(&asset, &10u32);

    // Now try to retract → AlreadyFinalized (#40)
    client.retract_price(&asset, &10u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #41)")]
fn test_double_retract_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &5_000i128, &50u64);
    client.mark_price_pending(&asset);

    advance_ledger(&e, 15);
    client.retract_price(&asset, &10u32);

    // Second retract → PriceRetracted (#41)
    client.retract_price(&asset, &10u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_retract_nonexistent_panics() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    advance_ledger(&e, 10);
    // Nothing pending → NoData (#8)
    client.retract_price(&asset, &10u32);
}

// ---------------------------------------------------------------------------
// try_finalize errors on already-finalized / retracted
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #40)")]
fn test_try_finalize_already_finalized_panics() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&10u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);
    client.mark_price_pending(&asset);

    advance_ledger(&e, 21);
    client.try_finalize_price(&asset, &10u32); // succeeds

    // Call again → AlreadyFinalized (#40)
    client.try_finalize_price(&asset, &10u32);
}

// ---------------------------------------------------------------------------
// check_reorg (structural — no reorg in test env)
// ---------------------------------------------------------------------------

#[test]
fn test_check_reorg_no_reorg() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_finality_ledgers(&10u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 10);
    client.submit_price(&source, &asset, &1_000i128, &50u64);
    client.mark_price_pending(&asset);

    // In test env there is no real reorg → always false
    let detected = client.check_reorg(&asset, &10u32);
    assert!(!detected);
}
