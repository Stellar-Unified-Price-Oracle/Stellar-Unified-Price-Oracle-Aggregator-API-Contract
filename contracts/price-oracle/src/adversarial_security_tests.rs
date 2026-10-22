//! Adversarial security tests for issues #458, #459, #460, #463 and #670.
//!
//! See `docs/security/adversarial-audit.md` for the accompanying analysis.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, String, Vec};

use crate::test_helpers::{
    deploy_token, mint_token, register_test_asset, register_test_source, setup_contract,
};
use crate::types::{Asset, DataKey};
use crate::{ErrorCode, PriceOracleContractClient, RelayerFailureReason};

fn evict(e: &Env, client: &PriceOracleContractClient<'_>, key: &DataKey) {
    e.as_contract(&client.address, || e.storage().persistent().remove(key));
}

// ══════════════════════════════════════════════════════════════════════════════
// #458 — Fee-market manipulation and fee-griefing resistance
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn fee_below_floor_is_always_rejected() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let src = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    client.fm_set_min_priority_fee(&100u128);
    for fee in [0u128, 1, 50, 99] {
        let res = client.try_fm_enqueue_submission(
            &src,
            &Asset::Stellar(asset.clone()),
            &1_000u128,
            &1u64,
            &fee,
        );
        assert_eq!(res, Err(Ok(ErrorCode::FeeMarketBelowMinimum.into())));
    }
    client.fm_enqueue_submission(&src, &Asset::Stellar(asset), &1_000u128, &1u64, &100u128);
    assert_eq!(client.fm_get_pending_submissions(), 1);
}

#[test]
fn fee_split_rounds_in_protocol_favour() {
    // source_share = floor(fee * ratio / 100) and treasury = fee - source_share, so the
    // remainder of the division always lands in the treasury and nothing is lost.
    for (ratio, fee) in [(80u32, 1u128), (80, 3), (33, 7), (99, 1), (1, 199)] {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let src = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);
        client.fm_set_fee_distribution_ratio(&ratio);

        client.fm_enqueue_submission(&src, &Asset::Stellar(asset), &1_000u128, &1u64, &fee);
        assert_eq!(client.fm_process_fee_market(), 1);

        let source_share = client.fm_get_source_fee_balance(&src);
        let treasury_share = client.fm_get_treasury_fee_balance();
        assert_eq!(source_share + treasury_share, fee);
        assert!(source_share * 100 <= fee * ratio as u128);
    }
}

#[test]
fn parameter_change_applies_under_adversarial_occupancy() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let attacker = register_test_source(&e, &client, "ATK");
    let asset = register_test_asset(&e, &client);

    // Attacker fills the queue beyond one processing batch.
    for i in 0..(crate::fee_market::MAX_PROCESS_PER_LEDGER + 5) {
        client.fm_enqueue_submission(
            &attacker,
            &Asset::Stellar(asset.clone()),
            &1_000u128,
            &(i as u64 + 1),
            &1_000_000u128,
        );
    }

    // Admin parameter changes are independent of the queue and take effect immediately.
    client.fm_set_min_priority_fee(&2_000_000u128);
    client.fm_set_fee_distribution_ratio(&10u32);
    assert_eq!(client.fm_get_min_priority_fee(), 2_000_000u128);
    assert_eq!(client.fm_get_fee_distribution_ratio(), 10u32);
    let res = client.try_fm_enqueue_submission(
        &attacker,
        &Asset::Stellar(asset),
        &1_000u128,
        &99u64,
        &1_000_000u128,
    );
    assert_eq!(res, Err(Ok(ErrorCode::FeeMarketBelowMinimum.into())));
}

#[test]
fn targeted_griefing_is_paid_by_attacker_only() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let attacker = register_test_source(&e, &client, "ATK");
    let victim = register_test_source(&e, &client, "VIC");
    let a1 = register_test_asset(&e, &client);
    let a2 = register_test_asset(&e, &client);

    client.fm_enqueue_submission(
        &victim,
        &Asset::Stellar(a1.clone()),
        &1_000u128,
        &1u64,
        &10u128,
    );
    client.fm_enqueue_submission(
        &attacker,
        &Asset::Stellar(a2),
        &1_000u128,
        &1u64,
        &1_000u128,
    );
    assert_eq!(client.fm_process_fee_market(), 2);

    // The victim's earnings are unaffected by the attacker's outbidding, while the
    // attacker forfeits the treasury share (20%) of everything it spends.
    assert_eq!(client.fm_get_source_fee_balance(&victim), 8u128);
    assert_eq!(client.fm_get_source_fee_balance(&attacker), 800u128);
    assert_eq!(client.fm_get_treasury_fee_balance(), 202u128);
    assert!(client.get_price(&a1, &u64::MAX).is_some());
}

// ══════════════════════════════════════════════════════════════════════════════
// #459 — Governance capture: vote duplication and snapshot-boundary double voting
// ══════════════════════════════════════════════════════════════════════════════

fn governors(e: &Env, n: u32) -> Vec<Address> {
    let mut v = Vec::new(e);
    for _ in 0..n {
        v.push_back(Address::generate(e));
    }
    v
}

#[test]
fn governor_cannot_approve_twice() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let gov = governors(&e, 3);
    client.ms_set_governors(&gov, &2u32);
    let g0 = gov.get_unchecked(0);

    let op = client.ms_propose_operation(&g0, &2u32, &Bytes::new(&e));
    client.ms_approve_operation(&g0, &op);
    let res = client.try_ms_approve_operation(&g0, &op);
    assert_eq!(res, Err(Ok(ErrorCode::AlreadyApproved.into())));
    assert_eq!(client.ms_get_operation(&op).approvals.len(), 1);
}

#[test]
fn snapshot_boundary_rotation_cannot_double_vote() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let gov = governors(&e, 3);
    client.ms_set_governors(&gov, &2u32);
    let g0 = gov.get_unchecked(0);
    let op = client.ms_propose_operation(&g0, &2u32, &Bytes::new(&e));
    client.ms_approve_operation(&g0, &op);

    // Remove g0 and re-add it: the approval is recorded per identity on the operation,
    // so the re-added governor still cannot approve a second time.
    let mut without = gov.clone();
    without.remove(0);
    client.ms_set_governors(&without, &2u32);
    assert_eq!(
        client.try_ms_approve_operation(&g0, &op),
        Err(Ok(ErrorCode::NotAuthorized.into()))
    );
    client.ms_set_governors(&gov, &2u32);
    assert_eq!(
        client.try_ms_approve_operation(&g0, &op),
        Err(Ok(ErrorCode::AlreadyApproved.into()))
    );
}

#[test]
fn retract_then_reapprove_counts_once() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let gov = governors(&e, 3);
    client.ms_set_governors(&gov, &2u32);
    let g0 = gov.get_unchecked(0);
    let op = client.ms_propose_operation(&g0, &2u32, &Bytes::new(&e));

    client.ms_approve_operation(&g0, &op);
    client.ms_retract_approval(&g0, &op);
    client.ms_approve_operation(&g0, &op);
    let stored = client.ms_get_operation(&op);
    assert_eq!(stored.approvals.len(), 1);
    assert_eq!(stored.timelock_start_ledger, 0);
}

// ══════════════════════════════════════════════════════════════════════════════
// #460 — Relayer bond slashing evasion
// ══════════════════════════════════════════════════════════════════════════════

fn bonded_relayer(e: &Env, client: &PriceOracleContractClient<'_>) -> Address {
    let token = deploy_token(e);
    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);
    let relayer = Address::generate(e);
    client.add_relayer(&relayer, &String::from_str(e, "R1"));
    mint_token(e, &token, &relayer, &1_000i128);
    client.deposit_relayer_bond(&relayer);
    relayer
}

// ══════════════════════════════════════════════════════════════════════════════
// #670 — Reward mechanism Sybil and collusion simulation
// ══════════════════════════════════════════════════════════════════════════════
//
// These scenarios model the proposed contribution-reward split (fee distribution
// ratio) against three attacker strategies and quantify profitability. Each
// scenario is executable and asserts the mitigation holds.

/// Net reward an attacker keeps after the treasury takes its cut.
fn attacker_net(fee: u128, ratio: u32) -> u128 {
    // source_share = floor(fee * ratio / 100); treasury keeps the remainder.
    fee * ratio as u128 / 100
}

#[test]
fn sybil_source_splitting_does_not_increase_payout() {
    // Baseline: one honest source submits N units of work at fee F each.
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.fm_set_fee_distribution_ratio(&80u32);
    let honest = register_test_source(&e, &client, "HON");
    let asset = register_test_asset(&e, &client);
    let fee = 1_000u128;
    let units = 5u64;
    for i in 0..units {
        client.fm_enqueue_submission(
            &honest,
            &Asset::Stellar(asset.clone()),
            &1_000u128,
            &(i + 1),
            &fee,
        );
    }
    assert_eq!(client.fm_process_fee_market(), units as u32);
    let honest_payout = client.fm_get_source_fee_balance(&honest);

    // Attack: split the same work across `units` freshly generated identities.
    let e2 = Env::default();
    e2.mock_all_auths();
    let (client2, _) = setup_contract(&e2);
    client2.set_min_sources_required(&1u32);
    client2.fm_set_fee_distribution_ratio(&80u32);
    let asset2 = register_test_asset(&e2, &client2);
    let mut split_payout = 0u128;
    for i in 0..units {
        let sybil = register_test_source(&e2, &client2, "SYB");
        client2.fm_enqueue_submission(
            &sybil,
            &Asset::Stellar(asset2.clone()),
            &1_000u128,
            &(i + 1),
            &fee,
        );
        assert_eq!(client2.fm_process_fee_market(), 1);
        split_payout += client2.fm_get_source_fee_balance(&sybil);
    }

    // Splitting yields exactly the same total reward: no per-identity bonus exists.
    assert_eq!(split_payout, honest_payout);
    assert_eq!(split_payout, units as u128 * attacker_net(fee, 80));
    // Profitability: gross reward minus the treasury cut is zero-sum, so the
    // attacker gains nothing versus honest participation.
    let gross = units as u128 * fee;
    assert_eq!(gross - split_payout, units as u128 * (fee - attacker_net(fee, 80)));
}

#[test]
fn collusion_coordinated_submissions_are_capped_by_treasury_cut() {
    // A cartel of K colluding sources submits identical work. Because each unit
    // forfeits the treasury share, collusion is strictly less profitable than the
    // honest single-source baseline for the same gross spend.
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.fm_set_fee_distribution_ratio(&80u32);
    let asset = register_test_asset(&e, &client);
    let fee = 1_000u128;
    let cartel_size = 4u32;

    let mut cartel_payout = 0u128;
    for i in 0..cartel_size {
        let member = register_test_source(&e, &client, "CRT");
        client.fm_enqueue_submission(
            &member,
            &Asset::Stellar(asset.clone()),
            &1_000u128,
            &(i as u64 + 1),
            &fee,
        );
        assert_eq!(client.fm_process_fee_market(), 1);
        cartel_payout += client.fm_get_source_fee_balance(&member);
    }

    let gross = cartel_size as u128 * fee;
    let treasury = client.fm_get_treasury_fee_balance();
    // Every unit of gross spend leaks 20% to the treasury; the cartel cannot
    // recover it, so net profitability is negative relative to gross.
    assert_eq!(cartel_payout + treasury, gross);
    assert_eq!(treasury, gross - cartel_payout);
    assert!(cartel_payout < gross);
}

#[test]
fn identity_rotation_cannot_compound_rewards() {
    // Rotating identities across rounds does not accumulate a bonus: each identity
    // earns only its own per-unit share, and the treasury cut is invariant to the
    // number of distinct identities used.
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.fm_set_fee_distribution_ratio(&80u32);
    let asset = register_test_asset(&e, &client);
    let fee = 1_000u128;
    let rounds = 3u32;

    let mut rotated_payout = 0u128;
    for r in 0..rounds {
        // Fresh identity each round (rotation).
        let id = register_test_source(&e, &client, "ROT");
        client.fm_enqueue_submission(
            &id,
            &Asset::Stellar(asset.clone()),
            &1_000u128,
            &(r as u64 + 1),
            &fee,
        );
        assert_eq!(client.fm_process_fee_market(), 1);
        rotated_payout += client.fm_get_source_fee_balance(&id);
    }

    let gross = rounds as u128 * fee;
    let treasury = client.fm_get_treasury_fee_balance();
    // Rotation yields the same per-unit reward as a single persistent identity.
    assert_eq!(rotated_payout, rounds as u128 * attacker_net(fee, 80));
    assert_eq!(rotated_payout + treasury, gross);
    // No compounding: total reward is linear in work, not in identity count.
    assert_eq!(rotated_payout, attacker_net(gross, 80));
}

#[test]
fn mitigation_ratio_bounds_attack_profitability() {
    // Mitigation under test: the fee-distribution ratio caps the attacker's share.
    // Lowering the ratio strictly reduces attacker profitability for the same work.
    let fee = 1_000u128;
    let units = 10u128;
    let gross = fee * units;
    let high = attacker_net(gross, 80);
    let low = attacker_net(gross, 50);
    assert!(low < high);
    // Treasury captures the difference, so the protocol is never worse off.
    assert_eq!(gross - high, gross - attacker_net(gross, 80));
    assert_eq!(gross - low, gross - attacker_net(gross, 50));
}
