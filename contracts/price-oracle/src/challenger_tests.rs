#![cfg(test)]

use crate::test_helpers::*;
use crate::types::Challenge;
use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, String};

#[test]
fn test_challenge_price_success() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin) = setup_contract(&e);
    let asset = Address::generate(&e);
    let challenger = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);
    client.challenge_price(&asset, &1000i128, &proof);

    // Should succeed without panicking
    let rewards = client.get_challenger_rewards(&challenger);
    assert_eq!(rewards, 0); // Not resolved yet
}

#[test]
fn test_challenge_price_invalid_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);

    let proof = Bytes::new(&e);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.challenge_price(&asset, &1000i128, &proof);
    }));

    assert!(result.is_err());
}

#[test]
fn test_challenge_price_invalid_price() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.challenge_price(&asset, &0i128, &proof);
    }));

    assert!(result.is_err());
}

#[test]
fn test_resolve_challenge_valid() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);
    let challenger = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);
    client.challenge_price(&asset, &1000i128, &proof);

    // Resolve as valid
    client.resolve_challenge(&1u32, &true);

    // Challenger should have rewards now
    let rewards = client.get_challenger_rewards(&challenger);
    assert!(rewards > 0);
}

#[test]
fn test_resolve_challenge_invalid() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);
    let challenger = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);
    client.challenge_price(&asset, &1000i128, &proof);

    // Resolve as invalid
    client.resolve_challenge(&1u32, &false);

    // Challenger should have NO rewards
    let rewards = client.get_challenger_rewards(&challenger);
    assert_eq!(rewards, 0);
}

#[test]
fn test_claim_rewards_success() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);
    client.challenge_price(&asset, &1000i128, &proof);
    client.resolve_challenge(&1u32, &true);

    // Claim rewards
    let claimed = client.claim_rewards();
    assert!(claimed > 0);

    // Rewards should be cleared
    let challenger = Address::generate(&e);
    let rewards = client.get_challenger_rewards(&challenger);
    assert_eq!(rewards, 0);
}

#[test]
fn test_get_challenge_history() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);

    // Submit multiple challenges
    for i in 0..5 {
        client.challenge_price(&asset, &((1000 + i) as i128), &proof);
    }

    // Get history
    let history = client.get_challenge_history(&asset, &10u32);
    assert_eq!(history.len(), 5);
}

#[test]
fn test_multiple_challenges_same_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);

    client.challenge_price(&asset, &1000i128, &proof);
    client.challenge_price(&asset, &2000i128, &proof);
    client.challenge_price(&asset, &3000i128, &proof);

    let history = client.get_challenge_history(&asset, &100u32);
    assert_eq!(history.len(), 3);
}

#[test]
fn test_rewards_accumulate() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);
    let challenger = Address::generate(&e);

    client.register_asset(&asset);

    let proof = Bytes::new(&e);

    // Submit and resolve multiple challenges
    client.challenge_price(&asset, &1000i128, &proof);
    client.resolve_challenge(&1u32, &true);

    client.challenge_price(&asset, &2000i128, &proof);
    client.resolve_challenge(&2u32, &true);

    // Rewards should accumulate
    let rewards = client.get_challenger_rewards(&challenger);
    assert!(rewards > 0);
}
