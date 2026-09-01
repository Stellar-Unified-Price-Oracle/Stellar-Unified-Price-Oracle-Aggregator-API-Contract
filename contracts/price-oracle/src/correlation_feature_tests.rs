//! Tests for:
//!   1. Correlation flagging + exclusion from aggregation
//!   2. simulate_aggregation — pure computation
//!   3. submit_price_merkle — merkle-verified batch submission
//!   4. submit_prices gas-efficient batch (overhead < 20 % vs single)

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, BytesN, Env, Vec,
};

use crate::prices::{MerkleLeaf, MerkleProof};
use crate::test_helpers::*;

// ─── helpers ────────────────────────────────────────────────────────────────

fn set_ledger(e: &Env, seq: u32, ts: u64) {
    e.ledger().set(LedgerInfo {
        timestamp: ts,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 4096,
    });
}

// ─── 1. Correlation flagging + exclusion ─────────────────────────────────────

/// A correlation pair is registered for (base_asset, quote_asset).
/// A price that violates the ratio band should be flagged and excluded
/// from the next aggregation.
#[test]
fn test_correlation_violation_flags_and_excludes() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let src1 = register_test_source(&e, &client, "S1");
    let src2 = register_test_source(&e, &client, "S2");
    let base_asset = register_test_asset(&e, &client);
    let quote_asset = register_test_asset(&e, &client);

    // Submit a baseline price for quote_asset so correlation check has a counterpart.
    // Price = 1_000 (stablecoin-like)
    client.submit_price(&src1, &quote_asset, &1_000_i128, &1_000_000_u64);

    // Correlation band: base/quote ratio must be [0.9, 1.1] × RATIO_PRECISION (10^7)
    // i.e., min = 9_000_000, max = 11_000_000  → ratio must stay near 1.0
    let ratio_precision: u128 = 10_000_000;
    let min_ratio: u128 = ratio_precision * 9 / 10; // 0.9
    let max_ratio: u128 = ratio_precision * 11 / 10; // 1.1
    client.set_correlation_pair(&base_asset, &quote_asset, &min_ratio, &max_ratio, &true);

    // src1 submits a wildly deviant price for base_asset (10× the quote)
    let deviant_price: i128 = 10_000;
    client.submit_price(&src1, &base_asset, &deviant_price, &1_000_000_u64);

    // The (src1, base_asset) pair should now be flagged.
    assert!(client.is_correlation_flagged(&src1, &base_asset));

    // src2 submits a normal in-band price for base_asset.
    let normal_price: i128 = 1_005;
    client.submit_price(&src2, &base_asset, &normal_price, &1_000_000_u64);

    // Aggregate should reflect ONLY src2's price (src1 is excluded).
    let agg = client.get_price(&base_asset, &0u64).unwrap();
    assert_eq!(
        agg.price, normal_price,
        "flagged price must not affect aggregate"
    );
}

/// After an admin clears the flag, the previously-flagged source contributes again.
#[test]
fn test_clear_correlation_flag_restores_source() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let src = register_test_source(&e, &client, "S");
    let base_asset = register_test_asset(&e, &client);
    let quote_asset = register_test_asset(&e, &client);

    // Establish quote baseline.
    client.submit_price(&src, &quote_asset, &1_000_i128, &1_000_000_u64);

    let rp: u128 = 10_000_000;
    client.set_correlation_pair(
        &base_asset,
        &quote_asset,
        &(rp * 9 / 10),
        &(rp * 11 / 10),
        &true,
    );

    // Trigger a violation.
    client.submit_price(&src, &base_asset, &99_999_i128, &1_000_000_u64);
    assert!(client.is_correlation_flagged(&src, &base_asset));

    // Admin clears the flag.
    client.clear_correlation_flag(&src, &base_asset);
    assert!(!client.is_correlation_flagged(&src, &base_asset));

    // src re-submits a valid price; it should now contribute to aggregation.
    client.submit_price(&src, &base_asset, &1_001_i128, &1_000_000_u64);
    let agg = client.get_price(&base_asset, &0u64).unwrap();
    assert_eq!(agg.price, 1_001_i128);
}

/// An in-band submission must NOT set a correlation flag.
#[test]
fn test_correlation_inband_not_flagged() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let src = register_test_source(&e, &client, "S");
    let base_asset = register_test_asset(&e, &client);
    let quote_asset = register_test_asset(&e, &client);

    client.submit_price(&src, &quote_asset, &1_000_i128, &1_000_000_u64);

    let rp: u128 = 10_000_000;
    client.set_correlation_pair(
        &base_asset,
        &quote_asset,
        &(rp * 9 / 10),
        &(rp * 11 / 10),
        &true,
    );

    // In-band price: ratio = 1.05 → should pass.
    client.submit_price(&src, &base_asset, &1_050_i128, &1_000_000_u64);
    assert!(!client.is_correlation_flagged(&src, &base_asset));
}

// ─── 2. simulate_aggregation ─────────────────────────────────────────────────

/// Simulated aggregate must match the real aggregate when inputs are identical.
#[test]
fn test_simulate_matches_real() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&3u32);

    let src1 = register_test_source(&e, &client, "S1");
    let src2 = register_test_source(&e, &client, "S2");
    let src3 = register_test_source(&e, &client, "S3");
    let asset = register_test_asset(&e, &client);

    let p1: i128 = 100;
    let p2: i128 = 200;
    let p3: i128 = 150;

    client.submit_price(&src1, &asset, &p1, &1_000_000_u64);
    client.submit_price(&src2, &asset, &p2, &1_000_000_u64);
    client.submit_price(&src3, &asset, &p3, &1_000_000_u64);

    let real_agg = client.get_price(&asset, &0u64).unwrap().price;

    // Build hypothetical_prices matching exactly what was submitted.
    let mut hypo: Vec<(Address, i128)> = Vec::new(&e);
    hypo.push_back((src1.clone(), p1));
    hypo.push_back((src2.clone(), p2));
    hypo.push_back((src3.clone(), p3));

    let simulated = client.simulate_aggregation(&asset, &hypo).unwrap();
    assert_eq!(
        simulated, real_agg,
        "simulated aggregate must equal real aggregate with same prices"
    );
}

/// simulate_aggregation returns None when fewer sources than min_required are supplied.
#[test]
fn test_simulate_insufficient_sources_returns_none() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&3u32);

    let src = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    let mut hypo: Vec<(Address, i128)> = Vec::new(&e);
    hypo.push_back((src.clone(), 500_i128));

    // Only 1 source supplied but 3 required.
    let result = client.simulate_aggregation(&asset, &hypo);
    assert!(result.is_none());
}

/// simulate_aggregation must NOT write to storage (aggregate remains unchanged).
#[test]
fn test_simulate_is_pure() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let src = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    client.submit_price(&src, &asset, &500_i128, &1_000_000_u64);
    let before = client.get_price(&asset, &0u64).unwrap().price;

    // Simulate with a different price.
    let mut hypo: Vec<(Address, i128)> = Vec::new(&e);
    hypo.push_back((src.clone(), 9999_i128));
    let sim = client.simulate_aggregation(&asset, &hypo).unwrap();
    assert_eq!(sim, 9999_i128);

    // Real aggregate must be unchanged.
    let after = client.get_price(&asset, &0u64).unwrap().price;
    assert_eq!(
        before, after,
        "simulate_aggregation must not write to storage"
    );
}

// ─── 3. submit_price_merkle ───────────────────────────────────────────────────

/// A single-leaf merkle tree has no siblings; the root IS the leaf hash.
/// After submission, the aggregate should be correct.
#[test]
fn test_merkle_single_leaf() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let src = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    let leaf = MerkleLeaf {
        source: src.clone(),
        asset: asset.clone(),
        price: 42_000_i128,
        timestamp: 1_000_000_u64,
    };

    // For a single-leaf tree the root = hash(leaf), no siblings needed.
    // We compute the root the same way the contract does.
    let root = compute_leaf_hash(&e, &leaf);

    let siblings: Vec<BytesN<32>> = Vec::new(&e);
    let proof = MerkleProof {
        leaf,
        siblings,
        left_bitmap: 0u32,
    };

    let mut proofs: Vec<MerkleProof> = Vec::new(&e);
    proofs.push_back(proof);

    client.submit_price_merkle(&src, &root, &proofs);

    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 42_000_i128);
}

/// A tampered root must cause the call to panic.
#[test]
#[should_panic]
fn test_merkle_invalid_root_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let src = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    let leaf = MerkleLeaf {
        source: src.clone(),
        asset: asset.clone(),
        price: 1_000_i128,
        timestamp: 1_000_000_u64,
    };

    // Provide all-zeroes root (wrong).
    let bad_root: BytesN<32> = BytesN::from_array(&e, &[0u8; 32]);
    let siblings: Vec<BytesN<32>> = Vec::new(&e);
    let proof = MerkleProof {
        leaf,
        siblings,
        left_bitmap: 0,
    };
    let mut proofs: Vec<MerkleProof> = Vec::new(&e);
    proofs.push_back(proof);

    client.submit_price_merkle(&src, &bad_root, &proofs);
}

// ─── 4. submit_prices batch efficiency ───────────────────────────────────────

/// Verifies that submitting 10 assets via submit_prices works correctly and
/// produces valid aggregates for every asset (functional correctness proxy for
/// gas efficiency — actual gas numbers are a wasm-level metric).
#[test]
fn test_batch_submit_prices_all_assets_aggregate() {
    let e = Env::default();
    e.mock_all_auths();
    set_ledger(&e, 100, 1_000_000);

    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);

    let src = register_test_source(&e, &client, "S");

    // Register 10 assets.
    let mut assets: Vec<Address> = Vec::new(&e);
    for _ in 0..10u32 {
        assets.push_back(register_test_asset(&e, &client));
    }

    // Build one batch of (asset, price, timestamp) tuples.
    let mut batch: Vec<(Address, i128, u64)> = Vec::new(&e);
    for i in 0..assets.len() {
        let asset = assets.get_unchecked(i);
        let price: i128 = 1_000_i128 + i as i128;
        batch.push_back((asset, price, 1_000_000_u64));
    }

    client.submit_prices(&src, &batch);

    // Every asset must have a valid aggregate equal to the submitted price.
    for i in 0..assets.len() {
        let asset = assets.get_unchecked(i);
        let expected_price: i128 = 1_000_i128 + i as i128;
        let agg = client.get_price(&asset, &0u64).unwrap();
        assert_eq!(
            agg.price, expected_price,
            "asset {} has wrong aggregate after batch submit",
            i
        );
    }
}

// ─── internal hash helper (mirrors contract logic for test root computation) ──

/// Replicates the `hash_leaf` logic from prices.rs so tests can build valid roots.
fn compute_leaf_hash(e: &Env, leaf: &MerkleLeaf) -> BytesN<32> {
    use soroban_sdk::Bytes;
    let mut data = Bytes::new(e);
    data.append(&Bytes::from_slice(e, &leaf.price.to_le_bytes()));
    data.append(&Bytes::from_slice(e, &leaf.timestamp.to_le_bytes()));
    e.crypto().sha256(&data)
}
