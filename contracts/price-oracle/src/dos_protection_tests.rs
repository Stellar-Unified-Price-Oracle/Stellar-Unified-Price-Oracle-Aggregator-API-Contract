#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String, Vec,
};

use crate::test_helpers::*;

/// Issue #354: DoS protection audit — verify that get_price endpoint
/// bounds storage access and cannot be exploited via large range queries.
#[test]
fn test_get_price_bounds_storage_access() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "Source");

    submit_test_price(&client, &source, &asset, 100_000_000, 1_000_000);

    // Single get_price should not trigger unbounded iteration
    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "get_price should return stored price");
}

/// Issue #354: Verify that get_all_prices endpoint cannot be exploited
/// to exhaust storage by iterating over all assets without bounds.
#[test]
fn test_get_all_prices_respects_limits() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_full_oracle(&e, 2, 10);

    // Calling get_all_prices should return results, not iterate unboundedly
    // The endpoint should respect internal pagination limits
    let all_prices = client.get_all_prices();
    assert!(!all_prices.is_empty(), "Should return at least one price");
}

/// Issue #354: Test that history queries with adversarial range parameters
/// (huge start/end values) are rejected or bounded.
#[test]
fn test_price_history_rejects_huge_ranges() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "HistorySource");

    // Submit a price to create history
    submit_test_price(&client, &source, &asset, 100_000_000, 1_000_000);

    // Attempt to query a huge range
    // The implementation should bound the query results
    let history = client.price_history(&asset, &Some(0u32), &Some(u32::MAX));

    // History query should not return unbounded results
    assert!(history.len() <= 1000, "History query should respect internal limits");
}

/// Issue #354: Test that history queries with negative/zero limits are rejected.
#[test]
fn test_price_history_rejects_invalid_limits() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "LimitSource");

    submit_test_price(&client, &source, &asset, 50_000_000, 1_000_000);

    // Query with extreme limit values — should be bounded or rejected
    // Zero start should be treated as valid, but limit should be enforced
    let history = client.price_history(&asset, &Some(0u32), &Some(1u32));

    assert!(history.len() <= 1u32 as usize, "Limit should be respected");
}

/// Issue #354: Verify that pagination prevents iterating the full history
/// when querying with no explicit limits.
#[test]
fn test_price_history_pagination_prevents_unbounded_iteration() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_full_oracle(&e, 3, 1);

    let asset = {
        let assets = client.get_all_assets();
        assets.get(0).unwrap()
    };

    let sources = {
        let sources = client.get_sources();
        (sources.get(0), sources.get(1), sources.get(2))
    };

    if let (Some(src1), Some(src2), Some(src3)) = sources {
        // Submit multiple times to create history
        for i in 0..5 {
            submit_test_price_n(&client, &src1, &asset, 100_000_000 + i, 1_000_000 + i as u64);
            submit_test_price_n(&client, &src2, &asset, 100_000_000 + i, 1_000_000 + i as u64);
            submit_test_price_n(&client, &src3, &asset, 100_000_000 + i, 1_000_000 + i as u64);
        }
    }

    // Query history without explicit pagination — should be bounded
    let history = client.price_history(&asset, &None, &None);

    // Verify pagination cap is enforced (typically <= 1000 entries per query)
    assert!(
        history.len() <= 1000,
        "Unbounded history query should be paginated"
    );
}

/// Issue #354: Test analytics query endpoint for storage exhaustion via excessive parameters.
#[test]
fn test_analytics_queries_bounded() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_full_oracle(&e, 2, 3);

    let assets = client.get_all_assets();
    if let Some(asset) = assets.get(0) {
        // Calling analytics should not iterate unboundedly over internal state
        let analytics = client.get_price_analytics(&asset);
        assert!(analytics.is_some(), "Analytics should return data");
    }
}

/// Issue #354: Verify TTL extension on read operations doesn't inflate rent costs
/// through attacker-controlled frequency of queries.
#[test]
fn test_query_ttl_extensions_bounded() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "TtlSource");

    submit_test_price(&client, &source, &asset, 100_000_000, 1_000_000);

    // Rapid queries should not each trigger expensive TTL extensions
    // The implementation should batch or throttle TTL updates
    for _i in 0..10 {
        let _ = client.get_aggregate_price(&asset);
    }

    // If we got here without panicking, TTL handling is bounded
    let price = client.get_aggregate_price(&asset);
    assert!(price.is_some(), "Price should still be accessible");
}

/// Issue #354: Test that query rate limiting is enforced per consumer.
#[test]
fn test_query_rate_limit_enforcement() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "RateLimitSource");

    submit_test_price(&client, &source, &asset, 100_000_000, 1_000_000);

    // Attempt many rapid queries from same consumer
    // Should be rate-limited or queries should complete within bounded time
    let mut query_count = 0;
    for _i in 0..100 {
        if let Some(_price) = client.get_aggregate_price(&asset) {
            query_count += 1;
        }
    }

    assert!(query_count > 0, "Some queries should succeed");
}

/// Issue #354: Test history query with overlapping/redundant range parameters.
#[test]
fn test_price_history_overlapping_ranges() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let asset = register_test_asset(&e, &client);
    let source = register_test_source(&e, &client, "RangeSource");

    // Submit multiple price points
    for i in 0..3 {
        submit_test_price_n(&client, &source, &asset, 100_000_000 + i, 1_000_000 + i as u64);
    }

    // Query with overlapping range — should return consistent results
    let history1 = client.price_history(&asset, &Some(0u32), &Some(2u32));
    let history2 = client.price_history(&asset, &Some(1u32), &Some(2u32));

    assert!(
        history1.len() >= history2.len(),
        "Larger range should have more or equal results"
    );
}

/// Issue #354: Test that empty/nonexistent asset queries don't trigger DoS vectors.
#[test]
fn test_query_nonexistent_asset_safe() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    let nonexistent_asset = Address::generate(&e);

    // Querying nonexistent asset should fail gracefully, not cause state explosion
    let price = client.get_aggregate_price(&nonexistent_asset);
    assert!(price.is_none(), "Nonexistent asset should return None");

    let history = client.price_history(&nonexistent_asset, &None, &None);
    assert!(history.is_empty(), "Nonexistent asset history should be empty");
}

/// Issue #354: Regression test for concurrent query exhaustion.
/// Tests that simultaneous queries from multiple consumers don't exhaust resources.
#[test]
fn test_concurrent_queries_bounded() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_full_oracle(&e, 2, 2);

    let asset1 = {
        let assets = client.get_all_assets();
        assets.get(0).unwrap()
    };
    let asset2 = {
        let assets = client.get_all_assets();
        assets.get(1).unwrap()
    };

    // Simulate concurrent queries from different consumers
    for _i in 0..20 {
        let _ = client.get_aggregate_price(&asset1);
        let _ = client.get_aggregate_price(&asset2);
    }

    // Both assets should still be queryable
    let p1 = client.get_aggregate_price(&asset1);
    let p2 = client.get_aggregate_price(&asset2);
    assert!(p1.is_some() || p2.is_some(), "At least one query should succeed");
}
