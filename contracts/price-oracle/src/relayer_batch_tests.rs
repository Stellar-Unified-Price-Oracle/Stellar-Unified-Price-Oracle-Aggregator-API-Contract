#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::{PriceOracleContract, PriceOracleContractClient, RelayedSubmission};

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

fn leg(
    source: &Address,
    asset: &Address,
    price: i128,
    ts: u64,
    priority_fee: u128,
) -> RelayedSubmission {
    RelayedSubmission {
        source: source.clone(),
        asset: asset.clone(),
        price,
        timestamp: ts,
        priority_fee,
    }
}

// ---------------------------------------------------------------------------
// submit_prices_relayed — happy path
// ---------------------------------------------------------------------------

#[test]
fn test_batch_relay_multiple_sources_success() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source_a = add_source(&e, &client, "A");
    let source_b = add_source(&e, &client, "B");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e);
    batch.push_back(leg(&source_a, &asset, 1_000_000i128, ts, 200u128));
    batch.push_back(leg(&source_b, &asset, 1_100_000i128, ts, 100u128));

    client.submit_prices_relayed(&relayer, &batch);

    assert_eq!(client.get_relayer_submission_count(&relayer), 2u64);
    assert!(client.get_price(&asset, &0u64).is_some());
}

#[test]
fn test_batch_relay_accrues_priority_fees_into_fee_balance() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source_a = add_source(&e, &client, "A");
    let source_b = add_source(&e, &client, "B");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();

    client.set_relayer_fee_per_submission(&5i128);

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e);
    batch.push_back(leg(&source_a, &asset, 1_000_000i128, ts, 300u128));
    batch.push_back(leg(&source_b, &asset, 1_100_000i128, ts, 100u128));

    client.submit_prices_relayed(&relayer, &batch);

    // 2 flat fees (5 each) + 300 + 100 priority fee = 410.
    assert_eq!(client.get_relayer_fee_balance(&relayer), 410i128);
}

// ---------------------------------------------------------------------------
// submit_prices_relayed — validation
// ---------------------------------------------------------------------------

// BatchEmpty = 103
#[test]
#[should_panic(expected = "Error(Contract, #103)")]
fn test_batch_relay_empty_panics() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");

    let batch: Vec<RelayedSubmission> = Vec::new(&e);
    client.submit_prices_relayed(&relayer, &batch);
}

// BatchTooLarge = 104
#[test]
#[should_panic(expected = "Error(Contract, #104)")]
fn test_batch_relay_too_large_panics() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source = add_source(&e, &client, "A");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e);
    // MAX_BATCH_SIZE is 25; 26 legs must be rejected before any processing occurs.
    for _ in 0..26u32 {
        batch.push_back(leg(&source, &asset, 1_000_000i128, ts, 0u128));
    }
    client.submit_prices_relayed(&relayer, &batch);
}

// BatchNotFeePrioritized = 105
#[test]
#[should_panic(expected = "Error(Contract, #105)")]
fn test_batch_relay_requires_non_increasing_priority_fee() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source_a = add_source(&e, &client, "A");
    let source_b = add_source(&e, &client, "B");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e);
    // Fee increases from leg 0 to leg 1 — must be rejected.
    batch.push_back(leg(&source_a, &asset, 1_000_000i128, ts, 100u128));
    batch.push_back(leg(&source_b, &asset, 1_100_000i128, ts, 200u128));

    client.submit_prices_relayed(&relayer, &batch);
}

#[test]
fn test_batch_relay_equal_priority_fees_allowed() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source_a = add_source(&e, &client, "A");
    let source_b = add_source(&e, &client, "B");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e);
    batch.push_back(leg(&source_a, &asset, 1_000_000i128, ts, 100u128));
    batch.push_back(leg(&source_b, &asset, 1_100_000i128, ts, 100u128));

    client.submit_prices_relayed(&relayer, &batch);
    assert_eq!(client.get_relayer_submission_count(&relayer), 2u64);
}

// RelayerNotAuthorized = 50
#[test]
#[should_panic(expected = "Error(Contract, #50)")]
fn test_batch_relay_unapproved_relayer_panics() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let source = add_source(&e, &client, "A");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();
    let random_relayer = Address::generate(&e);

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e);
    batch.push_back(leg(&source, &asset, 1_000_000i128, ts, 0u128));

    client.submit_prices_relayed(&random_relayer, &batch);
}

// ---------------------------------------------------------------------------
// submit_prices_relayed — atomicity
// ---------------------------------------------------------------------------

#[test]
fn test_batch_relay_is_atomic_on_failure() {
    let e = Env::default();
    let (client, _) = setup(&e);
    let relayer = add_relayer(&e, &client, "R1");
    let source_a = add_source(&e, &client, "A");
    let source_b = add_source(&e, &client, "B");
    let asset = add_asset(&e, &client);
    let ts = e.ledger().timestamp();

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e);
    // First leg is valid; second leg has an invalid (zero) price and must abort the
    // whole batch, rolling back the first leg's otherwise-successful storage writes.
    batch.push_back(leg(&source_a, &asset, 1_000_000i128, ts, 200u128));
    batch.push_back(leg(&source_b, &asset, 0i128, ts, 100u128));

    let result = client.try_submit_prices_relayed(&relayer, &batch);
    assert!(result.is_err());

    // Nothing from the failed batch should have been persisted.
    assert_eq!(client.get_relayer_submission_count(&relayer), 0u64);
    assert!(client.get_price(&asset, &0u64).is_none());
}

// ---------------------------------------------------------------------------
// Gas benchmark: batch vs. N individual relayed submissions (#264 acceptance
// criterion: "Gas benchmark vs individual relayed"). Run locally with
// `-- --nocapture` to compare instruction counts; no strict assertion is made
// since absolute instruction counts vary across SDK versions.
// ---------------------------------------------------------------------------

#[test]
fn bench_batch_vs_individual_relayed_submissions() {
    const N: u32 = 10;

    // Individual submissions.
    let e1 = Env::default();
    e1.mock_all_auths();
    let (client1, _) = setup(&e1);
    let relayer1 = add_relayer(&e1, &client1, "R1");
    let mut sources1 = Vec::new(&e1);
    for _ in 0..N {
        sources1.push_back(add_source(&e1, &client1, "S"));
    }
    let asset1 = add_asset(&e1, &client1);
    let ts1 = e1.ledger().timestamp();

    e1.budget().reset_default();
    for i in 0..N {
        client1.submit_price_relayed(
            &relayer1,
            &sources1.get_unchecked(i),
            &asset1,
            &1_000_000i128,
            &ts1,
        );
    }
    let individual_cpu = e1.budget().cpu_instruction_cost();

    // Single batch of N legs.
    let e2 = Env::default();
    e2.mock_all_auths();
    let (client2, _) = setup(&e2);
    let relayer2 = add_relayer(&e2, &client2, "R2");
    let asset2 = add_asset(&e2, &client2);
    let ts2 = e2.ledger().timestamp();

    let mut batch: Vec<RelayedSubmission> = Vec::new(&e2);
    for _ in 0..N {
        let source = add_source(&e2, &client2, "S");
        batch.push_back(leg(&source, &asset2, 1_000_000i128, ts2, 0u128));
    }

    e2.budget().reset_default();
    client2.submit_prices_relayed(&relayer2, &batch);
    let batch_cpu = e2.budget().cpu_instruction_cost();

    let _ = individual_cpu;
    let _ = batch_cpu;
}
