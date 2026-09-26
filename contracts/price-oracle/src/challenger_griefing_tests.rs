#![cfg(test)]
//! #461 — Challenger griefing and false-challenge economics.
//!
//! See `docs/security/adversarial-review-461-462-464-466.md` for the cost analysis.

use soroban_sdk::testutils::{Address as _, MockAuth, MockAuthInvoke};
use soroban_sdk::{Address, Bytes, Env, IntoVal};

use crate::challenger::{
    MAX_CHALLENGER_STRIKES, MAX_OPEN_CHALLENGES_PER_ASSET, MAX_OPEN_CHALLENGES_PER_CHALLENGER,
};
use crate::test_helpers::{
    ledger_default, register_test_asset, register_test_source, setup_contract,
};

fn challenge(client: &crate::PriceOracleContractClient<'_>, who: &Address, asset: &Address) {
    client.challenge_price(who, asset, &1_000_000i128, &Bytes::new(&client.env));
}

#[test]
fn challenge_requires_challenger_auth() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let victim = Address::generate(&e);
    let attacker = Address::generate(&e);
    let proof = Bytes::new(&e);

    // Only the attacker signs; filing in the victim's name must fail.
    e.set_auths(&[]);
    let res = client
        .mock_auths(&[MockAuth {
            address: &attacker,
            invoke: &MockAuthInvoke {
                contract: &client.address,
                fn_name: "challenge_price",
                args: (&victim, &asset, 1_000i128, &proof).into_val(&e),
                sub_invokes: &[],
            },
        }])
        .try_challenge_price(&victim, &asset, &1_000i128, &proof);
    assert!(res.is_err());
}

#[test]
fn spam_is_bounded_per_challenger() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let spammer = Address::generate(&e);

    for _ in 0..MAX_OPEN_CHALLENGES_PER_CHALLENGER {
        challenge(&client, &spammer, &asset);
    }
    // Repeated challenges on the same asset/submission stop at the cap.
    assert!(client
        .try_challenge_price(&spammer, &asset, &1_000_000i128, &Bytes::new(&e))
        .is_err());
    assert_eq!(
        client.get_open_challenge_count(&asset),
        MAX_OPEN_CHALLENGES_PER_CHALLENGER
    );
}

#[test]
fn spam_is_bounded_per_asset_and_measured() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    for _ in 0..MAX_OPEN_CHALLENGES_PER_ASSET {
        challenge(&client, &Address::generate(&e), &asset);
    }
    assert!(client
        .try_challenge_price(&Address::generate(&e), &asset, &1_000i128, &Bytes::new(&e))
        .is_err());

    // Measured impact: defending costs exactly one admin resolution per challenge,
    // so the honest-side work is bounded by the cap, not by attacker volume.
    for id in 1..=MAX_OPEN_CHALLENGES_PER_ASSET {
        client.resolve_challenge(&id, &false);
    }
    assert_eq!(client.get_open_challenge_count(&asset), 0);
    let _ = admin;
}

#[test]
fn frivolous_challenges_forfeit_rewards_and_bar_challenger() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let griefer = Address::generate(&e);

    // Earn a valid reward first (1_000_000 / 1000 = 1_000).
    challenge(&client, &griefer, &asset);
    client.resolve_challenge(&1u32, &true);
    assert_eq!(client.get_challenger_rewards(&griefer), 1_000);

    // Each frivolous challenge forfeits a reward-sized bond and adds a strike.
    for i in 0..MAX_CHALLENGER_STRIKES {
        challenge(&client, &griefer, &asset);
        client.resolve_challenge(&(i + 2), &false);
    }
    assert_eq!(client.get_challenger_rewards(&griefer), 0);
    assert!(client
        .try_challenge_price(&griefer, &asset, &1_000_000i128, &Bytes::new(&e))
        .is_err());
}

#[test]
fn repeated_resolution_of_one_challenge_is_rejected() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let who = Address::generate(&e);
    challenge(&client, &who, &asset);
    client.resolve_challenge(&1u32, &true);
    assert!(client.try_resolve_challenge(&1u32, &true).is_err());
    assert_eq!(client.get_challenger_rewards(&who), 1_000);
}

#[test]
fn finalization_boundary_snipe_grants_no_advantage() {
    let e = Env::default();
    ledger_default(&e, 100, 10_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let src = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);
    client.submit_price(&src, &asset, &5_000i128, &10_000u64);

    // Challenge in the same ledger as the submission: the served price is
    // unchanged and nothing is rewarded without an admin resolution.
    let sniper = Address::generate(&e);
    client.challenge_price(&sniper, &asset, &1i128, &Bytes::new(&e));
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 5_000);
    assert_eq!(client.get_challenger_rewards(&sniper), 0);
}

#[test]
fn unresolved_challenge_is_explicitly_flagged() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    assert_eq!(client.get_open_challenge_count(&asset), 0);
    challenge(&client, &Address::generate(&e), &asset);
    assert_eq!(client.get_open_challenge_count(&asset), 1);
    client.resolve_challenge(&1u32, &false);
    assert_eq!(client.get_open_challenge_count(&asset), 0);
}
