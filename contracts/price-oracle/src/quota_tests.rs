#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::test_helpers::*;

#[test]
fn test_set_source_quota() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    client.set_source_quota(&source, &10u32);
    assert_eq!(client.get_source_quota(&source), 10u32);
}

#[test]
fn test_set_quota_period() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    client.set_quota_period(&1000u32);
    assert_eq!(client.get_quota_period(), 1000u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_quota_period_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    client.set_quota_period(&1000u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_source_quota_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    clear_auth(&e);
    client.set_source_quota(&source, &10u32);
}

#[test]
fn test_default_quota_unlimited() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");

    // Default should be 0 (unlimited)
    assert_eq!(client.get_source_quota(&source), 0u32);
}

#[test]
fn test_submissions_within_quota() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_quota_period(&100u32);
    client.set_source_quota(&source, &5u32);

    ledger_default(&e, 100, 1234567890);

    // Submit 5 prices (at quota limit)
    for i in 0..5 {
        submit_test_price(&client, &source, &asset, 100i128 + i as i128, 1234567890 + i as u64);
    }

    // Verify all submissions were successful
    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 5);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn test_submissions_exceed_quota() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_quota_period(&100u32);
    client.set_source_quota(&source, &3u32);

    ledger_default(&e, 100, 1234567890);

    // Try to submit 4 prices (exceeds quota of 3)
    for i in 0..4 {
        submit_test_price(&client, &source, &asset, 100i128 + i as i128, 1234567890 + i as u64);
    }
}

#[test]
fn test_quota_reset_after_period() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_quota_period(&100u32);
    client.set_source_quota(&source, &2u32);

    // Submit at ledger 100
    ledger_default(&e, 100, 1234567890);
    submit_test_price(&client, &source, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source, &asset, 110i128, 1234567890);

    // At ledger 200, quota period has elapsed, quota resets
    ledger_default(&e, 200, 1234567891);
    submit_test_price(&client, &source, &asset, 120i128, 1234567891);
    submit_test_price(&client, &source, &asset, 130i128, 1234567891);

    // All 4 submissions should succeed
    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 4);
}

#[test]
fn test_quota_boundary_exact_at_end() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_quota_period(&100u32);
    client.set_source_quota(&source, &3u32);

    ledger_default(&e, 100, 1234567890);

    // Submit exactly 3
    for i in 0..3 {
        submit_test_price(&client, &source, &asset, 100i128 + i as i128, 1234567890 + i as u64);
    }

    // Verify 3 were accepted
    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 3);
}

#[test]
fn test_multiple_sources_independent_quotas() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    let asset = register_test_asset(&e, &client);

    client.set_quota_period(&100u32);
    client.set_source_quota(&source1, &2u32);
    client.set_source_quota(&source2, &5u32);

    ledger_default(&e, 100, 1234567890);

    // source1 submits 2
    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source1, &asset, 110i128, 1234567890);

    // source2 submits 5
    for i in 0..5 {
        submit_test_price(&client, &source2, &asset, 200i128 + i as i128, 1234567890 + i as u64);
    }

    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 7);
}

#[test]
fn test_zero_quota_treated_as_unlimited() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_quota_period(&100u32);
    client.set_source_quota(&source, &0u32); // Unlimited

    ledger_default(&e, 100, 1234567890);

    // Submit many more than would fit in a typical quota
    for i in 0..20 {
        submit_test_price(&client, &source, &asset, 100i128 + i as i128, 1234567890 + i as u64);
    }

    let all_prices = client.get_all_prices(&asset);
    assert_eq!(all_prices.len(), 20);
}

#[test]
#[should_panic(expected = "Error(Contract, #14)")]
fn test_quota_exceeded_at_first_submission_over_limit() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = register_test_source(&e, &client, "Source");
    let asset = register_test_asset(&e, &client);

    client.set_quota_period(&100u32);
    client.set_source_quota(&source, &1u32);

    ledger_default(&e, 100, 1234567890);

    submit_test_price(&client, &source, &asset, 100i128, 1234567890);
    // Second submission should fail
    submit_test_price(&client, &source, &asset, 110i128, 1234567890);
}

#[test]
fn test_get_quota_period_default() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    // Default period
    let period = client.get_quota_period();
    // Should be retrievable
    let _ = period;
}
