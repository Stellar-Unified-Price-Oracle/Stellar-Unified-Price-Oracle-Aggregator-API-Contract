#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::{PriceOracleContract, PriceOracleContractClient, RelayerFailureReason};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn setup(e: &Env) -> (PriceOracleContractClient<'_>, Address) {
    e.mock_all_auths();
    let contract_id = e.register(PriceOracleContract, ());
    let client = PriceOracleContractClient::new(e, &contract_id);
    let admin = Address::generate(e);
    client.initialize(
        &admin,
        &1u32,
        &10u32,
        &18u32,
        &String::from_str(e, "Test Oracle"),
    );
    (client, admin)
}

fn add_relayer(e: &Env, client: &PriceOracleContractClient<'_>, name: &str) -> Address {
    let relayer = Address::generate(e);
    client.add_relayer(&relayer, &String::from_str(e, name));
    relayer
}

fn add_source(e: &Env, client: &PriceOracleContractClient<'_>, name: &str) -> Address {
    let source = Address::generate(e);
    client.add_source(&source, &String::from_str(e, name));
    source
}

fn add_asset(e: &Env, client: &PriceOracleContractClient<'_>) -> Address {
    let asset = Address::generate(e);
    client.register_asset(&asset);
    asset
}

// ---------------------------------------------------------------------------
// Bond configuration & deposit
// ---------------------------------------------------------------------------

#[test]
fn test_relayer_bond_config_defaults_to_zero() {
    let e = Env::default();
    let (client, _) = setup(&e);
    assert_eq!(client.get_relayer_bond_amount(), 0i128);
}

#[test]
fn test_deposit_relayer_bond_success() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let token = Address::generate(&e);

    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);

    client.deposit_relayer_bond(&relayer);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 1_000i128);
}

#[test]
fn test_deposit_relayer_bond_tops_up_shortfall_only() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let token = Address::generate(&e);

    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);
    client.deposit_relayer_bond(&relayer);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 1_000i128);

    // Raising the requirement and depositing again should only collect the shortfall.
    client.set_relayer_bond_amount(&1_500i128);
    client.deposit_relayer_bond(&relayer);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 1_500i128);

    // Already fully bonded: a further deposit call is a no-op.
    client.deposit_relayer_bond(&relayer);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 1_500i128);
}

// RelayerNotAuthorized = 50
#[test]
#[should_panic(expected = "Error(Contract, #50)")]
fn test_deposit_relayer_bond_unapproved_relayer_panics() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let token = Address::generate(&e);
    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);

    let random = Address::generate(&e);
    client.deposit_relayer_bond(&random);
}

// ---------------------------------------------------------------------------
// Bond gating on relayed submissions
// ---------------------------------------------------------------------------

// RelayerBondInsufficient = 106
#[test]
#[should_panic(expected = "Error(Contract, #106)")]
fn test_submission_blocked_without_required_bond() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);

    client.set_relayer_bond_amount(&1_000i128);

    let ts = e.ledger().timestamp();
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);
}

#[test]
fn test_submission_succeeds_once_bond_deposited() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);
    let token = Address::generate(&e);

    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);
    client.deposit_relayer_bond(&relayer);

    let ts = e.ledger().timestamp();
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);
    assert_eq!(client.get_relayer_submission_count(&relayer), 1u64);
}

// ---------------------------------------------------------------------------
// Withdrawal
// ---------------------------------------------------------------------------

#[test]
fn test_withdraw_relayer_bond_returns_full_amount() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let token = Address::generate(&e);

    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);
    client.deposit_relayer_bond(&relayer);

    client.withdraw_relayer_bond(&relayer);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 0i128);
}

#[test]
fn test_withdraw_relayer_bond_noop_when_nothing_deposited() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");

    // No panic expected even though no bond has ever been deposited.
    client.withdraw_relayer_bond(&relayer);
    assert_eq!(client.get_relayer_bond_balance(&relayer), 0i128);
}

// ---------------------------------------------------------------------------
// Failure recording & slashing
// ---------------------------------------------------------------------------

#[test]
fn test_record_relayer_failure_increments_count() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");

    assert_eq!(client.get_relayer_failure_count(&relayer), 0u32);
    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);
    assert_eq!(client.get_relayer_failure_count(&relayer), 1u32);
    client.record_relayer_failure(&relayer, &RelayerFailureReason::InvalidSubmission);
    assert_eq!(client.get_relayer_failure_count(&relayer), 2u32);
}

// RelayerFailureThresholdMiss = 107
#[test]
#[should_panic(expected = "Error(Contract, #107)")]
fn test_slash_relayer_below_threshold_panics_without_force() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let token = Address::generate(&e);

    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);
    client.deposit_relayer_bond(&relayer);

    // Default threshold is 3; only one failure has been reported.
    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);
    client.slash_relayer(&relayer, &false);
}

#[test]
fn test_slash_relayer_at_threshold_slashes_bond() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let token = Address::generate(&e);

    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);
    client.deposit_relayer_bond(&relayer);
    client.set_relayer_failure_threshold(&2u32);
    client.set_relayer_slash_percent(&25u32);

    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);
    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);

    client.slash_relayer(&relayer, &false);

    // 25% of 1000 = 250 slashed, 750 remaining.
    assert_eq!(client.get_relayer_bond_balance(&relayer), 750i128);
    // Failure counter resets after a slash.
    assert_eq!(client.get_relayer_failure_count(&relayer), 0u32);
}

#[test]
fn test_slash_relayer_forced_bypasses_threshold() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let token = Address::generate(&e);

    client.set_stake_token_contract(&token);
    client.set_relayer_bond_amount(&1_000i128);
    client.deposit_relayer_bond(&relayer);

    // No failures reported at all, but `force` bypasses the eligibility check.
    client.slash_relayer(&relayer, &true);
    assert!(client.get_relayer_bond_balance(&relayer) < 1_000i128);
}

// ---------------------------------------------------------------------------
// Reward accrual
// ---------------------------------------------------------------------------

#[test]
fn test_relayer_reward_accrues_at_full_rate_with_no_failures() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);

    client.set_relayer_reward_rate(&100i128);

    let ts = e.ledger().timestamp();
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);

    // No failures reported → full accuracy → full reward rate credited.
    assert_eq!(client.get_relayer_reward_balance(&relayer), 100i128);
}

#[test]
fn test_relayer_reward_reduced_by_reported_failures() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);

    client.set_relayer_reward_rate(&100i128);
    // One reported failure before the (unrelated) successful submission below.
    client.record_relayer_failure(&relayer, &RelayerFailureReason::Other);

    let ts = e.ledger().timestamp();
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);

    // accuracy_bps = 10_000 * 1 / (1 + 1) = 5_000 → reward = 100 * 5_000 / 10_000 = 50.
    assert_eq!(client.get_relayer_reward_balance(&relayer), 50i128);
}

#[test]
fn test_relayer_reward_disabled_by_default() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);

    let ts = e.ledger().timestamp();
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);

    assert_eq!(client.get_relayer_reward_balance(&relayer), 0i128);
}
