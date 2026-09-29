#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};
use crate::test_helpers::*;

#[test]
fn test_delegate_voting_power() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let token_holder = Address::generate(&e);
    let delegate = Address::generate(&e);

    // Token holder should not have any delegations initially
    let delegation = client.get_voter_delegation(&token_holder);
    assert!(delegation.is_none());

    // Delegate voting power
    client.delegate_voting_power(&token_holder, &delegate);

    // Check delegation was registered
    let new_delegation = client.get_voter_delegation(&token_holder);
    assert!(new_delegation.is_some());
    assert_eq!(new_delegation.unwrap(), delegate);
}

#[test]
fn test_revoke_voting_delegation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let token_holder = Address::generate(&e);
    let delegate = Address::generate(&e);

    // Setup delegation
    client.delegate_voting_power(&token_holder, &delegate);
    let delegation = client.get_voter_delegation(&token_holder);
    assert!(delegation.is_some());

    // Revoke delegation
    client.revoke_voting_delegation(&token_holder);

    // Delegation should be gone
    let revoked_delegation = client.get_voter_delegation(&token_holder);
    assert!(revoked_delegation.is_none());
}

#[test]
fn test_redelegate_voting_power() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let token_holder = Address::generate(&e);
    let delegate1 = Address::generate(&e);
    let delegate2 = Address::generate(&e);

    // Delegate to first address
    client.delegate_voting_power(&token_holder, &delegate1);
    let delegation1 = client.get_voter_delegation(&token_holder);
    assert_eq!(delegation1.unwrap(), delegate1);

    // Re-delegate to second address
    client.delegate_voting_power(&token_holder, &delegate2);
    let delegation2 = client.get_voter_delegation(&token_holder);
    assert_eq!(delegation2.unwrap(), delegate2);
}

#[test]
fn test_self_delegation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let token_holder = Address::generate(&e);

    // Allow self-delegation
    client.delegate_voting_power(&token_holder, &token_holder);

    let delegation = client.get_voter_delegation(&token_holder);
    assert_eq!(delegation.unwrap(), token_holder);
}

#[test]
fn test_snapshot_delegated_power_at_proposal_creation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let token_holder = Address::generate(&e);
    let delegate = Address::generate(&e);
    let proposal_id = 1u32;

    // Setup delegation
    client.delegate_voting_power(&token_holder, &delegate);

    // Create proposal (snapshot should capture current delegations)
    client.create_proposal(
        &admin,
        &String::from_str(&e, "Test Proposal"),
        &String::from_str(&e, "Test Description"),
    );

    // Verify delegation snapshot exists for this proposal
    let snapshot = client.get_delegation_snapshot(&proposal_id, &token_holder);
    assert!(snapshot.is_some());
}

#[test]
fn test_vote_with_delegated_power() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let token_holder = Address::generate(&e);
    let delegate = Address::generate(&e);
    let proposal_id = 1u32;

    // Setup delegation
    client.delegate_voting_power(&token_holder, &delegate);

    // Create proposal
    client.create_proposal(
        &admin,
        &String::from_str(&e, "Test Proposal"),
        &String::from_str(&e, "Test Description"),
    );

    // Delegate should be able to vote with delegated power
    client.vote(&delegate, &proposal_id, &true);

    // Verify vote was counted with delegated power
    let vote_power = client.get_vote_power(&proposal_id, &delegate);
    assert!(vote_power.is_some());
}

#[test]
fn test_delegation_events_emitted() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);

    let token_holder = Address::generate(&e);
    let delegate = Address::generate(&e);

    // Clear previous events
    e.events().all();

    // Delegate voting power
    client.delegate_voting_power(&token_holder, &delegate);

    // Check delegation event was emitted
    let events = e.events().all();
    assert!(!events.is_empty());
}
