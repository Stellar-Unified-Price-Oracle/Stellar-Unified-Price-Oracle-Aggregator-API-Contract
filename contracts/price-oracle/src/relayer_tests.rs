#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::{PriceOracleContract, PriceOracleContractClient};

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

fn add_relayer(e: &Env, client: &PriceOracleContractClient<'_>, name: &str) -> Address {
    let relayer = Address::generate(e);
    client.add_relayer(&relayer, &String::from_str(e, name));
    relayer
}

fn ledger_timestamp(e: &Env) -> u64 {
    e.ledger().timestamp()
}

// ---------------------------------------------------------------------------
// add_relayer
// ---------------------------------------------------------------------------

#[test]
fn test_add_relayer_success() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = Address::generate(&e);

    client.add_relayer(&relayer, &String::from_str(&e, "Hermes Relayer"));

    assert!(client.is_relayer(&relayer));
    let info = client.get_relayer_info(&relayer).unwrap();
    assert_eq!(info.name, String::from_str(&e, "Hermes Relayer"));
}

#[test]
fn test_add_relayer_stores_approved_at_ledger() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = Address::generate(&e);

    let current_ledger = e.ledger().sequence();
    client.add_relayer(&relayer, &String::from_str(&e, "R1"));
    let info = client.get_relayer_info(&relayer).unwrap();
    assert_eq!(info.approved_at_ledger, current_ledger);
}

// RelayerAlreadyExists = 17
#[test]
#[should_panic(expected = "Error(Contract, #17)")]
fn test_add_relayer_already_exists() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = Address::generate(&e);

    client.add_relayer(&relayer, &String::from_str(&e, "R1"));
    client.add_relayer(&relayer, &String::from_str(&e, "R1 duplicate"));
}

#[test]
#[should_panic]
fn test_add_relayer_not_admin() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = Address::generate(&e);

    use soroban_sdk::xdr::SorobanAuthorizationEntry;
    e.set_auths(&[] as &[SorobanAuthorizationEntry]);
    client.add_relayer(&relayer, &String::from_str(&e, "Unauthorized Relayer"));
}

// ---------------------------------------------------------------------------
// remove_relayer
// ---------------------------------------------------------------------------

#[test]
fn test_remove_relayer_success() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");

    assert!(client.is_relayer(&relayer));
    client.remove_relayer(&relayer);
    assert!(!client.is_relayer(&relayer));
}

// RelayerNotAuthorized = 16
#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_remove_relayer_not_registered() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = Address::generate(&e);

    client.remove_relayer(&relayer);
}

#[test]
#[should_panic]
fn test_remove_relayer_not_admin() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");

    use soroban_sdk::xdr::SorobanAuthorizationEntry;
    e.set_auths(&[] as &[SorobanAuthorizationEntry]);
    client.remove_relayer(&relayer);
}

// ---------------------------------------------------------------------------
// is_relayer / get_relayer_info
// ---------------------------------------------------------------------------

#[test]
fn test_is_relayer_unknown_address() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let random = Address::generate(&e);
    assert!(!client.is_relayer(&random));
}

#[test]
fn test_get_relayer_info_unknown_address() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let random = Address::generate(&e);
    assert!(client.get_relayer_info(&random).is_none());
}

#[test]
fn test_get_relayer_info_after_removal() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");

    client.remove_relayer(&relayer);
    assert!(client.get_relayer_info(&relayer).is_none());
}

// ---------------------------------------------------------------------------
// submit_price_relayed — happy path
// ---------------------------------------------------------------------------

#[test]
fn test_submit_price_relayed_success() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "Source1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "Hermes");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);

    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 1_000_000i128);
    assert_eq!(agg.num_sources, 1);
}

#[test]
fn test_submit_price_relayed_aggregates_with_multiple_sources() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    client.set_min_sources_required(&2u32);
    let source1 = add_source(&e, &client, "S1");
    let source2 = add_source(&e, &client, "S2");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "Hermes");

    let ts = ledger_timestamp(&e);
    // Source1 submits directly, source2 via relayer.
    client.submit_price(&source1, &asset, &1_000_000i128, &ts);
    client.submit_price_relayed(&relayer, &source2, &asset, &2_000_000i128, &ts);

    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.num_sources, 2);
    // Median of [1_000_000, 2_000_000] = 1_500_000
    assert_eq!(agg.price, 1_500_000i128);
}

#[test]
fn test_relayer_can_submit_on_behalf_of_any_registered_source() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source_a = add_source(&e, &client, "A");
    let source_b = add_source(&e, &client, "B");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "Hermes");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source_a, &asset, &500i128, &ts);
    client.submit_price_relayed(&relayer, &source_b, &asset, &800i128, &ts);

    let price_a = client.get_source_price(&asset, &source_a);
    let price_b = client.get_source_price(&asset, &source_b);
    assert_eq!(price_a.price, 500i128);
    assert_eq!(price_b.price, 800i128);
}

// ---------------------------------------------------------------------------
// submit_price_relayed — authorization failures
// ---------------------------------------------------------------------------

// RelayerNotAuthorized = 16
#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_submit_price_relayed_unapproved_relayer() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let random_relayer = Address::generate(&e); // never approved

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&random_relayer, &source, &asset, &1_000i128, &ts);
}

// NotAuthorized = 0  (check_source panics with NotAuthorized for unregistered sources)
#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_submit_price_relayed_unregistered_source() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");
    let ghost_source = Address::generate(&e); // never added as source

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &ghost_source, &asset, &1_000i128, &ts);
}

// AssetNotRegistered = 2
#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_submit_price_relayed_unregistered_asset() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let relayer = add_relayer(&e, &client, "R1");
    let ghost_asset = Address::generate(&e); // never registered

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &ghost_asset, &1_000i128, &ts);
}

// ---------------------------------------------------------------------------
// submit_price_relayed — price / timestamp validation
// ---------------------------------------------------------------------------

// InvalidPrice = 7
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_submit_price_relayed_zero_price() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &0i128, &ts);
}

// InvalidPrice = 7
#[test]
#[should_panic(expected = "Error(Contract, #7)")]
fn test_submit_price_relayed_negative_price() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &-1i128, &ts);
}

// InvalidTimestamp = 9
#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_submit_price_relayed_future_timestamp() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");

    // Default threshold is 300s; push timestamp 400s ahead.
    let far_future = ledger_timestamp(&e) + 400;
    client.submit_price_relayed(&relayer, &source, &asset, &1_000i128, &far_future);
}

// ---------------------------------------------------------------------------
// submit_price_relayed — contract paused
// ---------------------------------------------------------------------------

// ContractPaused = 12
#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn test_submit_price_relayed_when_paused() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");

    client.pause();
    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &1_000i128, &ts);
}

// ---------------------------------------------------------------------------
// Relayer removed between approval and submission
// ---------------------------------------------------------------------------

// RelayerNotAuthorized = 16
#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn test_submit_price_relayed_after_removal() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");

    client.remove_relayer(&relayer);

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &1_000i128, &ts);
}

// ---------------------------------------------------------------------------
// Fee tracking
// ---------------------------------------------------------------------------

#[test]
fn test_default_fee_is_zero() {
    let e = Env::default();
    let (client, _) = setup(&e);
    assert_eq!(client.get_relayer_fee_per_submission(), 0i128);
}

#[test]
fn test_set_and_get_relayer_fee() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    client.set_relayer_fee_per_submission(&5_000i128);
    assert_eq!(client.get_relayer_fee_per_submission(), 5_000i128);
}

#[test]
#[should_panic]
fn test_set_relayer_fee_not_admin() {
    let e = Env::default();
    let (client, _) = setup(&e);

    use soroban_sdk::xdr::SorobanAuthorizationEntry;
    e.set_auths(&[] as &[SorobanAuthorizationEntry]);
    client.set_relayer_fee_per_submission(&1_000i128);
}

#[test]
fn test_fee_accrues_per_relayed_submission() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "Hermes");

    client.set_relayer_fee_per_submission(&1_000i128);

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);
    assert_eq!(client.get_relayer_fee_balance(&relayer), 1_000i128);

    client.submit_price_relayed(&relayer, &source, &asset, &1_100_000i128, &ts);
    assert_eq!(client.get_relayer_fee_balance(&relayer), 2_000i128);
}

#[test]
fn test_fee_does_not_accrue_when_fee_is_zero() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "Hermes");

    // Fee stays at 0 (default).
    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &1_000_000i128, &ts);
    assert_eq!(client.get_relayer_fee_balance(&relayer), 0i128);
}

#[test]
fn test_fee_balance_is_independent_per_relayer() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source1 = add_source(&e, &client, "S1");
    let source2 = add_source(&e, &client, "S2");
    let asset = add_asset(&e, &client);
    let relayer_a = add_relayer(&e, &client, "Relayer-A");
    let relayer_b = add_relayer(&e, &client, "Relayer-B");

    client.set_relayer_fee_per_submission(&500i128);

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer_a, &source1, &asset, &1_000_000i128, &ts);
    client.submit_price_relayed(&relayer_a, &source2, &asset, &2_000_000i128, &ts);
    client.submit_price_relayed(&relayer_b, &source1, &asset, &1_500_000i128, &ts);

    assert_eq!(client.get_relayer_fee_balance(&relayer_a), 1_000i128);
    assert_eq!(client.get_relayer_fee_balance(&relayer_b), 500i128);
}

// ---------------------------------------------------------------------------
// Submission count
// ---------------------------------------------------------------------------

#[test]
fn test_submission_count_starts_at_zero() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    assert_eq!(client.get_relayer_submission_count(&relayer), 0u64);
}

#[test]
fn test_submission_count_increments() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "Hermes");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &1_000i128, &ts);
    assert_eq!(client.get_relayer_submission_count(&relayer), 1u64);

    client.submit_price_relayed(&relayer, &source, &asset, &2_000i128, &ts);
    assert_eq!(client.get_relayer_submission_count(&relayer), 2u64);
}

#[test]
fn test_submission_count_is_independent_per_relayer() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer_x = add_relayer(&e, &client, "X");
    let relayer_y = add_relayer(&e, &client, "Y");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer_x, &source, &asset, &1_000i128, &ts);
    client.submit_price_relayed(&relayer_x, &source, &asset, &2_000i128, &ts);
    client.submit_price_relayed(&relayer_y, &source, &asset, &3_000i128, &ts);

    assert_eq!(client.get_relayer_submission_count(&relayer_x), 2u64);
    assert_eq!(client.get_relayer_submission_count(&relayer_y), 1u64);
}

// ---------------------------------------------------------------------------
// Relayed price stored identically to direct submission
// ---------------------------------------------------------------------------

#[test]
fn test_relayed_price_stored_under_same_key_as_direct() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &9_999i128, &ts);

    // get_source_price reads DataKey::Submission(asset, source) — same key used by direct submit.
    let entry = client.get_source_price(&asset, &source);
    assert_eq!(entry.price, 9_999i128);
    assert_eq!(entry.source, source);
}

// ---------------------------------------------------------------------------
// Aggregation still works after relayed submission
// ---------------------------------------------------------------------------

#[test]
fn test_get_price_returns_none_before_any_submission() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let asset = add_asset(&e, &client);

    assert!(client.get_price(&asset, &0u64).is_none());
}

#[test]
fn test_relayed_submission_triggers_aggregation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    client.set_min_sources_required(&1u32);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let relayer = add_relayer(&e, &client, "R1");

    let ts = ledger_timestamp(&e);
    client.submit_price_relayed(&relayer, &source, &asset, &42_000i128, &ts);

    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 42_000i128);
    assert!(!agg.is_override);
}

// ---------------------------------------------------------------------------
// Direct submission unaffected by relayer registration
// ---------------------------------------------------------------------------

#[test]
fn test_direct_submit_still_works_alongside_relayer() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "S1");
    let asset = add_asset(&e, &client);
    let _relayer = add_relayer(&e, &client, "R1");

    let ts = ledger_timestamp(&e);
    client.submit_price(&source, &asset, &7_777i128, &ts);

    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 7_777i128);
}

// ---------------------------------------------------------------------------
// Multiple relayers
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_relayers_can_be_approved() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup(&e);

    let r1 = add_relayer(&e, &client, "Hermes");
    let r2 = add_relayer(&e, &client, "Egypt");
    let r3 = add_relayer(&e, &client, "Nexus");

    assert!(client.is_relayer(&r1));
    assert!(client.is_relayer(&r2));
    assert!(client.is_relayer(&r3));

    client.remove_relayer(&r2);
    assert!(!client.is_relayer(&r2));
    assert!(client.is_relayer(&r1));
    assert!(client.is_relayer(&r3));
}
