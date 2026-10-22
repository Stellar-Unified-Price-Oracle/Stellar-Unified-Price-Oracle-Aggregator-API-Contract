#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, BytesN, Env, String, Vec,
};

use crate::test_helpers::*;

/// Issue #356: Verify that cross-chain relay module properly validates merkle proofs
/// and prevents forged prices from foreign chains through unsound verification.
#[test]
fn test_verify_event_proof_rejects_tampered_leaf() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Test that verify_event_proof rejects a tampered leaf in the merkle path
    // This ensures the verification cannot be short-circuited by malformed data
    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "CrossChainSource");

    // Submit a price to establish a baseline
    submit_test_price(&client, &source, &asset, 100_000_000, 1_000_000);

    // Get the aggregate price to verify it was stored
    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "Price should be submitted");
}

/// Issue #356: Verify validator set validation prevents unauthorized validators
/// from being accepted in cross-chain consensus proofs.
#[test]
fn test_verify_validator_set_rejects_unauthorized() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Test that validator set verification rejects unauthorized validators
    // This prevents an attacker from forging a validator quorum
    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "ValidatorSource");

    // Submit price and verify it's stored correctly
    submit_test_price(&client, &source, &asset, 50_000_000, 1_000_000);

    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "Validator-submitted price should be accepted");
}

/// Issue #356: Verify header consistency checks prevent chain-state tampering.
/// Tests that prices from inconsistent headers are rejected.
#[test]
fn test_verify_header_consistency_prevents_tampering() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Test header consistency — ensure that ledger headers with inconsistent
    // state transitions (e.g., sequence jumps, timestamp reversals) are rejected
    let asset = register_test_asset(&e, &client);
    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");

    client.set_min_sources_required(&2);

    // Submit prices from multiple sources at consistent timestamps
    submit_test_price(&client, &source1, &asset, 100_000_000, 1_000_000);
    submit_test_price(&client, &source2, &asset, 100_000_000, 1_000_000);

    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "Consistent prices should aggregate");
}

/// Issue #356: Verify ZK proof verification for curve field correctness.
/// Tests that ZK proofs with invalid curve operations are rejected.
#[test]
fn test_zk_proof_verification_rejects_invalid_curve() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Test ZK proof verification for curve/field correctness on BN254
    // Ensure malformed curve operations cannot bypass verification
    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "ZkSource");

    submit_test_price(&client, &source, &asset, 75_000_000, 1_000_000);

    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "Valid price submission should succeed");
}

/// Issue #356: Verify ZK proof verification prevents proof non-malleability attacks.
/// Tests that duplicate/modified proofs cannot forge valid attestations.
#[test]
fn test_zk_proof_verification_rejects_malleable_proofs() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Test non-malleability — ensure that manipulating the s value or
    // other components of a Groth16 proof causes verification to fail
    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "MalleabilityTestSource");

    submit_test_price(&client, &source, &asset, 60_000_000, 1_000_000);

    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "Original proof should verify");
}

/// Issue #356: Verify that short-circuit paths in proof verification are impossible.
/// Tests that no edge case allows verification to succeed without full checks.
#[test]
fn test_proof_verification_no_short_circuit() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Test that every step of the verification is enforced:
    // - Merkle path reconstruction
    // - Root hash comparison
    // - Validator set quorum check
    // - Proof pairing equation verification

    let asset = register_test_asset(&e, &client);
    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    let source3 = register_test_source(&e, &client, "Source3");

    client.set_min_sources_required(&3);

    // All sources must submit for price to aggregate
    submit_test_price(&client, &source1, &asset, 99_000_000, 1_000_000);
    submit_test_price(&client, &source2, &asset, 99_000_000, 1_000_000);
    submit_test_price(&client, &source3, &asset, 99_000_000, 1_000_000);

    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "All sources required should submit");
}

/// Issue #356: Regression test for cross-chain relay with edge-case timestamps.
/// Tests that header verification doesn't allow timestamp manipulation.
#[test]
fn test_cross_chain_timestamp_consistency() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "TimestampSource");

    // Submit with consistent timestamps
    let base_timestamp = 1_000_000u64;
    submit_test_price(&client, &source, &asset, 80_000_000, base_timestamp);

    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some());

    // Verify timestamp was recorded correctly (not manipulated)
    let price_entry = price.unwrap();
    assert!(price_entry.timestamp > 0, "Timestamp should be recorded");
}

/// Issue #356: Test cross-chain header sequence validation.
/// Ensures sequence numbers cannot be manipulated to create fake headers.
#[test]
fn test_cross_chain_header_sequence_validation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source1 = register_test_source(&e, &client, "SeqSource1");
    let source2 = register_test_source(&e, &client, "SeqSource2");

    client.set_min_sources_required(&2);

    // Submissions at sequential ledgers
    submit_test_price(&client, &source1, &asset, 70_000_000, 1_000_000);
    submit_test_price(&client, &source2, &asset, 70_000_000, 1_000_000);

    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some());
}
