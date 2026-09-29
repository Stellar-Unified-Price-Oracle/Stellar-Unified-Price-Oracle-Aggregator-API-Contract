//! Adversarial security tests for issues #458, #459, #460 and #463.
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
    mint_token(e, &token, &relayer, 1_000);
    client.deposit_relayer_bond(&relayer);
    relayer
}

#[test]
fn withdrawal_blocked_while_dispute_pending() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let relayer = bonded_relayer(&e, &client);

    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);
    assert_eq!(
        client.try_withdraw_relayer_bond(&relayer),
        Err(Ok(ErrorCode::RelayerBondLocked.into()))
    );

    // Once the slash is executable it lands on the full bond.
    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);
    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);
    assert_eq!(
        client.try_withdraw_relayer_bond(&relayer),
        Err(Ok(ErrorCode::RelayerBondLocked.into()))
    );
    client.slash_relayer(&relayer, &false);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 800i128);

    // After the slash settles the remaining bond is withdrawable.
    client.withdraw_relayer_bond(&relayer);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 0i128);
}

#[test]
fn identity_rotation_cannot_escape_slash() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let relayer = bonded_relayer(&e, &client);

    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);
    // Being de-listed does not release the bond or the pending failure.
    client.remove_relayer(&relayer);
    assert_eq!(
        client.try_withdraw_relayer_bond(&relayer),
        Err(Ok(ErrorCode::RelayerBondLocked.into()))
    );
    client.slash_relayer(&relayer, &true);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 800i128);
}

// ══════════════════════════════════════════════════════════════════════════════
// #463 — TTL eviction as a data-availability attack
// ══════════════════════════════════════════════════════════════════════════════

#[test]
fn evicted_min_sources_fails_closed() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let s1 = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    evict(&e, &client, &DataKey::CfgMinSources);

    // Previously this silently fell back to 1, letting a single source set the price.
    assert_eq!(
        client.try_get_min_sources_required(),
        Err(Ok(ErrorCode::ConfigMissing.into()))
    );
    let ts = e.ledger().timestamp();
    assert!(client
        .try_submit_price(&s1, &asset, &1_000i128, &ts)
        .is_err());
    assert!(client.get_price(&asset, &u64::MAX).is_none());
}

#[test]
fn evicted_source_entry_fails_closed() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    let s1 = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);

    evict(&e, &client, &DataKey::SrcActive(s1.clone()));
    let ts = e.ledger().timestamp();
    assert!(client
        .try_submit_price(&s1, &asset, &1_000i128, &ts)
        .is_err());
}

#[test]
fn evicted_admin_fails_closed() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    evict(&e, &client, &DataKey::Admin);
    assert!(client.try_set_min_sources_required(&1u32).is_err());
}
