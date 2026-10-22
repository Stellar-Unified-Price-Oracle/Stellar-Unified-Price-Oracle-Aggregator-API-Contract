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
// get_relayer_dashboard
// ---------------------------------------------------------------------------

#[test]
fn test_dashboard_defaults_for_new_relayer() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");

    let dash = client.get_relayer_dashboard(&relayer);
    assert_eq!(dash.relayer, relayer);
    assert_eq!(dash.total_submissions, 0u64);
    assert_eq!(dash.failed_submissions, 0u32);
    assert_eq!(dash.success_rate_bps, 0u32);
    assert_eq!(dash.avg_latency_seconds, 0u64);
    assert_eq!(dash.fee_earnings, 0i128);
    assert_eq!(dash.reward_earnings, 0i128);
    assert_eq!(dash.bond_deposited, 0i128);
    assert_eq!(dash.per_asset.len(), 0);
    // Sole relayer in the registry ranks at the top.
    assert_eq!(dash.percentile_rank, 100u32);
}

#[test]
fn test_dashboard_reflects_submissions_and_fees() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);

    client.set_relayer_fee_per_submission(&10i128);

    let ts = e.ledger().timestamp();
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);

    let dash = client.get_relayer_dashboard(&relayer);
    assert_eq!(dash.total_submissions, 1u64);
    assert_eq!(dash.success_rate_bps, 10_000u32);
    assert_eq!(dash.fee_earnings, 10i128);
    assert_eq!(dash.avg_latency_seconds, 0u64);
    assert_eq!(dash.per_asset.len(), 1);
    let stat = dash.per_asset.get_unchecked(0);
    assert_eq!(stat.asset, asset);
    assert_eq!(stat.submissions, 1u64);
}

#[test]
fn test_dashboard_success_rate_with_reported_failures() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source_a = add_source(&e, &client, "A");
    let source_b = add_source(&e, &client, "B");
    let asset = add_asset(&e, &client);

    let ts = e.ledger().timestamp();
    client.submit_price_relayed(&relayer, &source_a, &asset, &1_000_000i128, &ts);
    client.submit_price_relayed(&relayer, &source_b, &asset, &1_100_000i128, &ts);
    client.record_relayer_failure(&relayer, &RelayerFailureReason::UnauthorizedPrice);

    // 2 successful, 1 failed → 10_000 * 2 / 3 = 6666.
    let dash = client.get_relayer_dashboard(&relayer);
    assert_eq!(dash.total_submissions, 2u64);
    assert_eq!(dash.failed_submissions, 1u32);
    assert_eq!(dash.success_rate_bps, 6_666u32);
}

#[test]
fn test_dashboard_percentile_rank_across_relayers() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer_a = add_relayer(&e, &client, "A");
    let relayer_b = add_relayer(&e, &client, "B");
    let relayer_c = add_relayer(&e, &client, "C");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();

    // A submits 3 times, B once, C never.
    client.submit_price_relayed(&relayer_a, &source, &asset, &1_000_000i128, &ts);
    client.submit_price_relayed(&relayer_a, &source, &asset, &1_000_001i128, &ts);
    client.submit_price_relayed(&relayer_a, &source, &asset, &1_000_002i128, &ts);
    client.submit_price_relayed(&relayer_b, &source, &asset, &1_000_003i128, &ts);

    assert_eq!(
        client.get_relayer_dashboard(&relayer_a).percentile_rank,
        100u32
    );
    assert_eq!(
        client.get_relayer_dashboard(&relayer_b).percentile_rank,
        66u32
    );
    assert_eq!(
        client.get_relayer_dashboard(&relayer_c).percentile_rank,
        33u32
    );
}

#[test]
fn test_dashboard_avg_latency_reflects_timestamp_gap() {
    use soroban_sdk::testutils::{Ledger, LedgerInfo};

    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "S");
    let asset = add_asset(&e, &client);

    e.ledger().set(LedgerInfo {
        timestamp: 1_000u64,
        protocol_version: 26,
        sequence_number: 100,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });

    // Observation timestamp is 40 seconds before ledger close time.
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &960u64);

    let dash = client.get_relayer_dashboard(&relayer);
    assert_eq!(dash.avg_latency_seconds, 40u64);
}
