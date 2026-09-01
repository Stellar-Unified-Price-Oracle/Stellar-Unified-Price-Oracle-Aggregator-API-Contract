#![cfg(test)]

//! # #187 — Commit-Reveal MEV Resistance Tests
//!
//! Tests for:
//! - Happy path: commit then reveal within correct windows
//! - Hash mismatch → CommitHashMismatch error
//! - Reveal too early → RevealWindowClosed
//! - Reveal too late → CommitExpired
//! - Double-commit → AlreadyCommitted
//! - Batch reveal: two assets atomically
//! - Batch over 100 entries rejected
//! - set/get_commit_window and set/get_reveal_window

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Bytes, BytesN, Env,
};

use crate::test_helpers::*;

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
        max_entry_ttl: 6000,
    });
}

/// Builds the sha256 preimage for a commit and returns the expected hash.
fn make_hash(e: &Env, price: i128, salt_val: u64, round_ledger: u32) -> BytesN<32> {
    let price_bytes = price.to_le_bytes();
    let salt_bytes = salt_val.to_le_bytes();
    let round_bytes = round_ledger.to_le_bytes();

    let mut preimage = Bytes::new(e);
    for b in price_bytes.iter() {
        preimage.push_back(*b);
    }
    for b in salt_bytes.iter() {
        preimage.push_back(*b);
    }
    for b in round_bytes.iter() {
        preimage.push_back(*b);
    }
    e.crypto().sha256(&preimage).into()
}

fn salt_bytes(e: &Env, val: u64) -> Bytes {
    let mut b = Bytes::new(e);
    for byte in val.to_le_bytes().iter() {
        b.push_back(*byte);
    }
    b
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[test]
fn test_set_get_commit_window() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_commit_window(&30u32);
    assert_eq!(client.get_commit_window(), 30u32);
}

#[test]
fn test_set_get_reveal_window() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_reveal_window(&15u32);
    assert_eq!(client.get_reveal_window(), 15u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_commit_window_zero_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_commit_window(&0u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_reveal_window_zero_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_reveal_window(&0u32);
}

// ---------------------------------------------------------------------------
// Round ledger alignment
// ---------------------------------------------------------------------------

#[test]
fn test_current_round_ledger_aligns_to_window() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_commit_window(&20u32);

    advance_ledger(&e, 25); // round = (25 / 20) * 20 = 20
    assert_eq!(client.current_round_ledger(), 20u32);

    advance_ledger(&e, 39); // round = (39 / 20) * 20 = 20
    assert_eq!(client.current_round_ledger(), 20u32);

    advance_ledger(&e, 40); // round = 40
    assert_eq!(client.current_round_ledger(), 40u32);
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[test]
fn test_commit_reveal_happy_path() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    // Commit at ledger 5 (round = 0)
    advance_ledger(&e, 5);
    let price: i128 = 50_000;
    let salt_val: u64 = 0xDEAD_BEEF;
    let round = client.current_round_ledger(); // 0
    let hash = make_hash(&e, price, salt_val, round);

    client.commit_price(&source, &asset, &hash);

    // Reveal at ledger 22 — inside reveal window [20, 40)
    advance_ledger(&e, 22);
    client.reveal_price(&source, &asset, &price, &salt_bytes(&e, salt_val), &round);

    let entry = client.get_source_price(&asset, &source);
    assert_eq!(entry.price, price);
}

// ---------------------------------------------------------------------------
// Error cases
// ---------------------------------------------------------------------------

#[test]
#[should_panic(expected = "Error(Contract, #31)")]
fn test_reveal_hash_mismatch() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 5);
    let price: i128 = 50_000;
    let salt_val: u64 = 1;
    let round = client.current_round_ledger();
    let hash = make_hash(&e, price, salt_val, round);
    client.commit_price(&source, &asset, &hash);

    advance_ledger(&e, 22);
    // Wrong price → hash mismatch
    client.reveal_price(
        &source,
        &asset,
        &99_999i128,
        &salt_bytes(&e, salt_val),
        &round,
    );
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn test_reveal_too_early_panics() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 5);
    let price: i128 = 50_000;
    let salt_val: u64 = 2;
    let round = client.current_round_ledger();
    let hash = make_hash(&e, price, salt_val, round);
    client.commit_price(&source, &asset, &hash);

    // Still inside commit window → RevealWindowClosed (#34)
    advance_ledger(&e, 10);
    client.reveal_price(&source, &asset, &price, &salt_bytes(&e, salt_val), &round);
}

#[test]
#[should_panic(expected = "Error(Contract, #32)")]
fn test_reveal_after_window_expires() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 5);
    let price: i128 = 50_000;
    let salt_val: u64 = 3;
    let round = client.current_round_ledger();
    let hash = make_hash(&e, price, salt_val, round);
    client.commit_price(&source, &asset, &hash);

    // Past reveal deadline: round(0) + commit_window(20) + reveal_window(20) = 40 → expired
    advance_ledger(&e, 41);
    client.reveal_price(&source, &asset, &price, &salt_bytes(&e, salt_val), &round);
}

#[test]
#[should_panic(expected = "Error(Contract, #35)")]
fn test_double_commit_same_round_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    advance_ledger(&e, 5);
    let round = client.current_round_ledger();
    let hash = make_hash(&e, 50_000, 4, round);
    client.commit_price(&source, &asset, &hash);
    // Second commit for same (source, asset, round) → AlreadyCommitted (#35)
    client.commit_price(&source, &asset, &hash);
}

#[test]
#[should_panic(expected = "Error(Contract, #34)")]
fn test_commit_after_window_closes_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    // Ledger 20 starts a new round; committing for round 0 is too late
    advance_ledger(&e, 20);
    let stale_round: u32 = 0;
    let hash = make_hash(&e, 50_000, 5, stale_round);
    client.commit_price(&source, &asset, &hash);
}

// ---------------------------------------------------------------------------
// Batch reveal
// ---------------------------------------------------------------------------

#[test]
fn test_batch_reveal_two_assets() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    let source = register_test_source(&e, &client, "S1");
    let asset1 = register_test_asset(&e, &client);
    let asset2 = register_test_asset(&e, &client);

    advance_ledger(&e, 5);
    let round = client.current_round_ledger();
    let (p1, s1) = (100_000i128, 10u64);
    let (p2, s2) = (200_000i128, 20u64);

    client.commit_price(&source, &asset1, &make_hash(&e, p1, s1, round));
    client.commit_price(&source, &asset2, &make_hash(&e, p2, s2, round));

    advance_ledger(&e, 22);

    let mut reveals = soroban_sdk::Vec::new(&e);
    reveals.push_back((asset1.clone(), p1, salt_bytes(&e, s1), round));
    reveals.push_back((asset2.clone(), p2, salt_bytes(&e, s2), round));

    client.reveal_prices_batch(&source, &reveals);

    assert_eq!(client.get_source_price(&asset1, &source).price, p1);
    assert_eq!(client.get_source_price(&asset2, &source).price, p2);
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_batch_over_100_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);
    let source = register_test_source(&e, &client, "S1");

    advance_ledger(&e, 5);
    let mut reveals = soroban_sdk::Vec::new(&e);
    for i in 0u64..101 {
        // Size check fires before asset registration lookup
        reveals.push_back((Address::generate(&e), 1i128, salt_bytes(&e, i), 0u32));
    }
    client.reveal_prices_batch(&source, &reveals);
}

// --- #292: Standalone Commit-Reveal Mode & Slashing ---

#[test]
fn test_standalone_commit_reveal_blocks_direct_submission() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    client.set_commit_reveal_enabled(&true);
    assert!(client.get_commit_reveal_enabled());

    let result = client.try_submit_price(&source, &asset, &100i128, &1000u64);
    assert!(result.is_err());
}

#[test]
fn test_standalone_commit_reveal_full_lifecycle() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    client.set_commit_reveal_enabled(&true);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);

    advance_ledger(&e, 5);
    let round = client.current_round_ledger();
    let price = 100_000i128;
    let salt_val = 42u64;
    let hash = make_hash(&e, price, salt_val, round);

    client.commit_price(&source, &asset, &hash);
    assert_eq!(client.get_source_price(&asset, &source).price, 0i128);

    advance_ledger(&e, 22);
    client.reveal_price(&source, &asset, &price, &salt_bytes(&e, salt_val), &round);
    assert_eq!(client.get_source_price(&asset, &source).price, price);
}

#[test]
fn test_slash_expired_commits() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    client.set_commit_reveal_enabled(&true);
    client.set_commit_window(&20u32);
    client.set_reveal_window(&20u32);
    client.set_commit_reveal_slash_amount(&1000i128);

    advance_ledger(&e, 5);
    let round = client.current_round_ledger();
    let hash = make_hash(&e, 100_000i128, 1u64, round);
    client.commit_price(&source, &asset, &hash);

    advance_ledger(&e, 50);
    client.slash_expired_commits(&asset, &source, &round);
}
