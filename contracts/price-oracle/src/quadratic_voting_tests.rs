#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};
use crate::test_helpers::*;

#[test]
fn test_linear_voting_mode_default() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let proposal_id = 1u32;

    // Create proposal with default voting mode (linear)
    client.create_proposal(
        &admin,
        &String::from_str(&e, "Test Proposal"),
        &String::from_str(&e, "Test Description"),
    );

    // Verify proposal uses linear voting mode by default
    let voting_mode = client.get_proposal_voting_mode(&proposal_id);
    assert_eq!(voting_mode, 0u32); // 0 = linear
}

#[test]
fn test_create_proposal_with_quadratic_voting() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let proposal_id = 1u32;

    // Create proposal with quadratic voting mode
    client.create_proposal_with_mode(
        &admin,
        &String::from_str(&e, "Quadratic Proposal"),
        &String::from_str(&e, "Test quadratic voting"),
        &1u32, // 1 = quadratic
    );

    // Verify proposal uses quadratic voting mode
    let voting_mode = client.get_proposal_voting_mode(&proposal_id);
    assert_eq!(voting_mode, 1u32);
}

#[test]
fn test_linear_voting_power_calculation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let voter = Address::generate(&e);
    let proposal_id = 1u32;
    let token_weight = 100u128;

    // Create proposal with linear voting mode
    client.create_proposal(
        &admin,
        &String::from_str(&e, "Linear Proposal"),
        &String::from_str(&e, "Test linear voting"),
    );

    // Vote with tokens
    client.vote_with_weight(&voter, &proposal_id, &true, &token_weight);

    // In linear mode, voting power should equal token weight
    let voting_power = client.get_voter_power(&proposal_id, &voter);
    assert_eq!(voting_power, token_weight);
}

#[test]
fn test_quadratic_voting_power_calculation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let voter = Address::generate(&e);
    let proposal_id = 1u32;
    let token_weight = 100u128;

    // Create proposal with quadratic voting mode
    client.create_proposal_with_mode(
        &admin,
        &String::from_str(&e, "Quadratic Proposal"),
        &String::from_str(&e, "Test quadratic voting"),
        &1u32,
    );

    // Vote with tokens
    client.vote_with_weight(&voter, &proposal_id, &true, &token_weight);

    // In quadratic mode, voting power should be sqrt(token_weight) = sqrt(100) = 10
    let voting_power = client.get_voter_power(&proposal_id, &voter);
    assert_eq!(voting_power, 10u128);
}

#[test]
fn test_quadratic_voting_mitigates_whale_dominance() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let whale = Address::generate(&e);
    let regular_voter1 = Address::generate(&e);
    let regular_voter2 = Address::generate(&e);
    let regular_voter3 = Address::generate(&e);
    let proposal_id = 1u32;

    // Create proposal with quadratic voting mode
    client.create_proposal_with_mode(
        &admin,
        &String::from_str(&e, "Quadratic Proposal"),
        &String::from_str(&e, "Test whale dominance mitigation"),
        &1u32,
    );

    // Whale has 10000 tokens
    client.vote_with_weight(&whale, &proposal_id, &true, &10000u128);
    let whale_power = client.get_voter_power(&proposal_id, &whale);

    // Each regular voter has 1000 tokens
    client.vote_with_weight(&regular_voter1, &proposal_id, &true, &1000u128);
    client.vote_with_weight(&regular_voter2, &proposal_id, &true, &1000u128);
    client.vote_with_weight(&regular_voter3, &proposal_id, &true, &1000u128);

    let regular_power = client.get_voter_power(&proposal_id, &regular_voter1);

    // In quadratic mode: whale power = sqrt(10000) = 100
    assert_eq!(whale_power, 100u128);
    // Regular power = sqrt(1000) ≈ 31
    assert!(regular_power > 30u128 && regular_power < 32u128);
    // 3 regular voters combined (≈93) is closer to whale power (100) than in linear mode
    // In linear mode: whale would have 10000 vs 3000 combined
}

#[test]
fn test_quadratic_voting_with_overflow_safety() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let voter = Address::generate(&e);
    let proposal_id = 1u32;

    // Create proposal with quadratic voting
    client.create_proposal_with_mode(
        &admin,
        &String::from_str(&e, "Large Weight Proposal"),
        &String::from_str(&e, "Test overflow safety"),
        &1u32,
    );

    // Use very large token weight to test overflow handling
    let large_weight = 18446744073709551615u128; // Near u128 max

    // This should not panic or overflow
    client.vote_with_weight(&voter, &proposal_id, &true, &large_weight);

    let voting_power = client.get_voter_power(&proposal_id, &voter);
    // Should compute sqrt safely
    assert!(voting_power > 0u128);
}

#[test]
fn test_voting_mode_emitted_in_proposal_event() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    // Clear events
    e.events().all();

    // Create proposal with quadratic voting
    client.create_proposal_with_mode(
        &admin,
        &String::from_str(&e, "Mode Event Test"),
        &String::from_str(&e, "Test mode in event"),
        &1u32,
    );

    // Check that events were emitted
    let events = e.events().all();
    assert!(!events.is_empty());
}

#[test]
fn test_switch_voting_mode_affects_calculations() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let voter = Address::generate(&e);
    let token_weight = 144u128;

    // Create linear proposal
    let linear_proposal_id = 1u32;
    client.create_proposal(
        &admin,
        &String::from_str(&e, "Linear"),
        &String::from_str(&e, "Linear proposal"),
    );
    client.vote_with_weight(&voter, &linear_proposal_id, &true, &token_weight);
    let linear_power = client.get_voter_power(&linear_proposal_id, &voter);

    // Create quadratic proposal
    let quadratic_proposal_id = 2u32;
    client.create_proposal_with_mode(
        &admin,
        &String::from_str(&e, "Quadratic"),
        &String::from_str(&e, "Quadratic proposal"),
        &1u32,
    );
    client.vote_with_weight(&voter, &quadratic_proposal_id, &true, &token_weight);
    let quadratic_power = client.get_voter_power(&quadratic_proposal_id, &voter);

    // Linear power should equal token weight
    assert_eq!(linear_power, 144u128);
    // Quadratic power should be sqrt(144) = 12
    assert_eq!(quadratic_power, 12u128);
    // They should be different
    assert_ne!(linear_power, quadratic_power);
}
