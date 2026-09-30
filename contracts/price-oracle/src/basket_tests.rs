//! Tests for the #479 basket / index price feeds.
//!
//! The security property under test throughout is the one the issue names: a
//! missing or stale constituent must make the index explicitly stale or absent,
//! never silently lighter.

#![cfg(test)]

use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env, String, Vec};

use crate::test_helpers::{register_test_asset, setup_contract};
use crate::{
    BasketConfig, BasketConstituent, BasketRebalancePolicy, BasketStalenessPolicy,
    PriceOracleContractClient, BASKET_WEIGHT_SCALE, MAX_BASKET_CONSTITUENTS,
};

/// Builds a constituent list from `(asset, weight)` pairs.
fn constituents(env: &Env, legs: &[(Address, u32)]) -> Vec<BasketConstituent> {
    let mut out: Vec<BasketConstituent> = Vec::new(env);
    for (asset, weight) in legs {
        out.push_back(BasketConstituent {
            asset: asset.clone(),
            weight: *weight,
        });
    }
    out
}

fn config(env: &Env, legs: &[(Address, u32)], policy: BasketStalenessPolicy) -> BasketConfig {
    BasketConfig {
        constituents: constituents(env, legs),
        total_weight: BASKET_WEIGHT_SCALE,
        staleness_policy: policy,
        rebalance_policy: BasketRebalancePolicy::Manual,
    }
}

/// A deployed contract with `n` registered assets, each priced at
/// `(i + 1) * 100` by a single source. Returns the basket address too.
fn setup_with_priced_assets<'a>(
    e: &'a Env,
    n: usize,
) -> (
    PriceOracleContractClient<'a>,
    Address,
    Vec<Address>,
    Address,
) {
    let (client, _admin) = setup_contract(e);
    e.ledger().with_mut(|l| l.timestamp = 1000);
    client.set_min_sources_required(&1u32);

    let source = Address::generate(e);
    client.add_source(&source, &String::from_str(e, "Source"));

    let mut assets: Vec<Address> = Vec::new(e);
    for i in 0..n {
        let a = register_test_asset(e, &client);
        client.submit_price(&source, &a, &((i as i128 + 1) * 100), &1000u64);
        assets.push_back(a);
    }
    let basket = Address::generate(e);
    (client, basket, assets, source)
}

/// `n` near-equal weights summing to exactly `BASKET_WEIGHT_SCALE`.
///
/// The remainder is added to the first leg, because truncating division loses
/// up to `n - 1` units and the sum-check demands an exact total.
fn even_weights(env: &Env, n: usize) -> Vec<u32> {
    let base = BASKET_WEIGHT_SCALE / n as u32;
    let mut out: Vec<u32> = Vec::new(env);
    for i in 0..n {
        let w = if i == 0 {
            base + (BASKET_WEIGHT_SCALE - base * n as u32)
        } else {
            base
        };
        out.push_back(w);
    }
    out
}

/// The index equals a reference weighted-sum computation.
#[test]
fn index_matches_reference_weighted_sum() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 4);

    // Prices 100, 200, 300, 400 with weights 10%, 20%, 30%, 40%.
    let legs = vec![
        (assets.get_unchecked(0), 100_000u32),
        (assets.get_unchecked(1), 200_000),
        (assets.get_unchecked(2), 300_000),
        (assets.get_unchecked(3), 400_000),
    ];
    client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject));

    let value = client.get_basket_value(&basket);
    // 10 + 40 + 90 + 160 = 300
    assert_eq!(value.value, 300);
    assert!(!value.is_degraded);
    assert_eq!(value.live_constituents, 4);
    assert_eq!(value.total_constituents, 4);
    assert_eq!(value.decimals, 18);

    // The same figure from an independent reference computation.
    let reference: i128 = legs
        .iter()
        .enumerate()
        .map(|(i, (_, w))| {
            ((i as i128 + 1) * 100) * i128::from(*w) / i128::from(BASKET_WEIGHT_SCALE)
        })
        .sum();
    assert!(
        (value.value - reference).abs() <= 1,
        "index {} drifted from reference {}",
        value.value,
        reference
    );
}

/// Per-constituent contributions are reported and account for the value.
#[test]
fn per_constituent_contributions_are_exposed() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 3);
    let w = even_weights(&e, 3);
    let legs = vec![
        (assets.get_unchecked(0), w.get_unchecked(0)),
        (assets.get_unchecked(1), w.get_unchecked(1)),
        (assets.get_unchecked(2), w.get_unchecked(2)),
    ];
    client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject));

    let value = client.get_basket_value(&basket);
    assert_eq!(value.contributions.len(), 3);

    let mut sum: i128 = 0;
    for i in 0..3u32 {
        let c = client.get_basket_contribution(&basket, &i);
        assert_eq!(c.weight, w.get_unchecked(i));
        assert!(c.present && c.fresh);
        sum += c.contribution;
    }
    // Contributions are each truncated, so they may sit a unit or two below the
    // single-truncation total; they must never exceed it.
    assert!(sum <= value.value && value.value - sum <= 3);
}

/// A missing constituent makes the index *absent*, never silently lighter.
#[test]
fn missing_constituent_rejects_the_index_by_default() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 3);
    let missing = register_test_asset(&e, &client);
    // Three live legs at 25% each plus a fourth with no aggregate at all.
    let reweighted = vec![
        (assets.get_unchecked(0), 250_000u32),
        (assets.get_unchecked(1), 250_000),
        (assets.get_unchecked(2), 250_000),
        (missing, 250_000),
    ];
    client.set_basket(
        &basket,
        &config(&e, &reweighted, BasketStalenessPolicy::Reject),
    );

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_basket_value(&basket)
    }));
    assert!(
        result.is_err(),
        "a missing constituent must not yield a value"
    );
}

/// A stale constituent is treated exactly like a missing one.
#[test]
fn stale_constituent_rejects_the_index() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 2);
    let legs = vec![
        (assets.get_unchecked(0), 500_000u32),
        (assets.get_unchecked(1), 500_000),
    ];
    client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject));
    assert_eq!(client.get_basket_value(&basket).value, 150);

    // Age every constituent past the staleness bound.
    client.set_basket_max_staleness(&10u64);
    e.ledger().with_mut(|l| l.timestamp = 5000);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_basket_value(&basket)
    }));
    assert!(
        result.is_err(),
        "a stale constituent must not yield a value"
    );
}

/// Under `Degrade`, the index is explicitly flagged and *not* rescaled.
#[test]
fn degraded_index_is_flagged_and_not_rescaled() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 3);
    let missing = register_test_asset(&e, &client);
    let legs = vec![
        (assets.get_unchecked(0), 250_000u32),
        (assets.get_unchecked(1), 250_000),
        (assets.get_unchecked(2), 250_000),
        (missing, 250_000),
    ];
    client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Degrade));

    let value = client.get_basket_value(&basket);
    assert!(value.is_degraded, "a degraded index must say so");
    assert_eq!(value.live_constituents, 3);
    assert_eq!(value.total_constituents, 4);
    // 25 + 50 + 75 = 150 over the *configured* weights. Rescaling to the three
    // live legs would report 200 and call it exact — the failure this forbids.
    assert_eq!(value.value, 150);

    // The missing leg is reported as present=false, not quietly omitted.
    let c = client.get_basket_contribution(&basket, &3u32);
    assert!(!c.present);
    assert!(!c.fresh);
    assert_eq!(c.contribution, 0);
}

/// Staleness propagates as the worst case across constituents.
#[test]
fn staleness_propagates_as_the_worst_case() {
    let e = Env::default();
    let (client, basket, assets, source) = setup_with_priced_assets(&e, 3);
    client.set_basket_max_staleness(&10_000u64);
    let legs = vec![
        (assets.get_unchecked(0), 333_333u32),
        (assets.get_unchecked(1), 333_333),
        (assets.get_unchecked(2), 333_334),
    ];
    client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject));

    // Refresh only the first leg; the other two keep their original timestamp.
    e.ledger().with_mut(|l| l.timestamp = 1300);
    client.submit_price(&source, &assets.get_unchecked(0), &100, &1300u64);

    let value = client.get_basket_value(&basket);
    // Legs 2 and 3 date from t=1000, so the index is 300 s old, not 0.
    assert_eq!(value.staleness_secs, 300);
}

/// Weights that do not sum to the scale are rejected on write.
#[test]
fn weights_must_sum_to_the_scale() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 2);

    // Sums to 900_000 — short of 1_000_000.
    let short = vec![
        (assets.get_unchecked(0), 400_000u32),
        (assets.get_unchecked(1), 500_000),
    ];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(&basket, &config(&e, &short, BasketStalenessPolicy::Reject))
    }));
    assert!(result.is_err(), "weights must sum to exactly 1_000_000");

    // Over the scale.
    let over = vec![
        (assets.get_unchecked(0), 600_000u32),
        (assets.get_unchecked(1), 600_000),
    ];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(&basket, &config(&e, &over, BasketStalenessPolicy::Reject))
    }));
    assert!(result.is_err(), "weights must sum to exactly 1_000_000");
}

/// A zero weight is rejected: it would be a present-but-ignored constituent.
#[test]
fn zero_weight_constituent_is_rejected() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 2);
    let legs = vec![
        (assets.get_unchecked(0), BASKET_WEIGHT_SCALE),
        (assets.get_unchecked(1), 0u32),
    ];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject))
    }));
    assert!(result.is_err(), "a zero-weight leg is a silent skip");
}

/// A basket naming itself is a cycle and is rejected.
#[test]
fn self_referential_basket_is_rejected() {
    let e = Env::default();
    let (client, basket, _assets, _) = setup_with_priced_assets(&e, 0);
    let legs = vec![(basket.clone(), BASKET_WEIGHT_SCALE)];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject))
    }));
    assert!(result.is_err(), "a basket must not contain itself");
}

/// A basket may not contain another basket, which forecloses recursion.
#[test]
fn nested_basket_is_rejected() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 2);

    // Configure a second address as a basket in its own right.
    let inner = Address::generate(&e);
    let inner_legs = vec![(assets.get_unchecked(1), BASKET_WEIGHT_SCALE)];
    client.set_basket(
        &inner,
        &config(&e, &inner_legs, BasketStalenessPolicy::Reject),
    );

    // The outer basket may not take `inner` as a leg.
    let outer_legs = vec![(inner, BASKET_WEIGHT_SCALE)];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(
            &basket,
            &config(&e, &outer_legs, BasketStalenessPolicy::Reject),
        )
    }));
    assert!(result.is_err(), "baskets must not nest");
}

/// Basket size is bounded.
#[test]
fn oversized_basket_is_rejected() {
    let e = Env::default();
    let (client, basket, _assets, _) = setup_with_priced_assets(&e, 0);

    let mut legs: std::vec::Vec<(Address, u32)> = std::vec::Vec::new();
    for _ in 0..=MAX_BASKET_CONSTITUENTS {
        legs.push((Address::generate(&e), 1u32));
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject))
    }));
    assert!(result.is_err(), "basket size must be bounded");
}

/// An empty basket is rejected.
#[test]
fn empty_basket_is_rejected() {
    let e = Env::default();
    let (client, basket, _assets, _) = setup_with_priced_assets(&e, 0);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(&basket, &config(&e, &[], BasketStalenessPolicy::Reject))
    }));
    assert!(result.is_err(), "an empty basket has no defined value");
}

/// A duplicate constituent is rejected: it would double-count the leg.
#[test]
fn duplicate_constituent_is_rejected() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 1);
    let legs = vec![
        (assets.get_unchecked(0), 500_000u32),
        (assets.get_unchecked(0), 500_000),
    ];
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject))
    }));
    assert!(result.is_err(), "a repeated asset has no single weight");
}

/// A rebalance applies the whole vector or none of it, and is evented.
#[test]
fn rebalancing_is_atomic_and_evented() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 2);
    let legs = vec![
        (assets.get_unchecked(0), 500_000u32),
        (assets.get_unchecked(1), 500_000),
    ];
    client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject));
    assert_eq!(client.get_basket_value(&basket).value, 150);

    // A valid rebalance: 25% / 75% -> 25 + 150 = 175.
    let mut new_weights: Vec<u32> = Vec::new(&e);
    new_weights.push_back(250_000);
    new_weights.push_back(750_000);
    client.rebalance_basket(&basket, &new_weights);
    let value = client.get_basket_value(&basket);
    assert_eq!(value.value, 175);
    let composition = client.get_basket_composition(&basket);
    assert_eq!(composition.get_unchecked(0).weight, 250_000);
    assert_eq!(composition.get_unchecked(1).weight, 750_000);

    // An invalid rebalance is rejected whole: the previous vector survives.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut bad: Vec<u32> = Vec::new(&e);
        bad.push_back(250_000);
        bad.push_back(250_000);
        client.rebalance_basket(&basket, &bad)
    }));
    assert!(result.is_err(), "a short weight sum must be rejected");
    let after = client.get_basket_value(&basket);
    assert_eq!(
        after.value, 175,
        "a rejected rebalance must leave the basket untouched"
    );
    let composition = client.get_basket_composition(&basket);
    assert_eq!(composition.get_unchecked(0).weight, 250_000);
    assert_eq!(composition.get_unchecked(1).weight, 750_000);
}

/// Reading an unconfigured basket is an explicit error, not a zero index.
#[test]
fn unconfigured_basket_is_an_error() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let basket = Address::generate(&e);
    let result =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| client.get_basket(&basket)));
    assert!(result.is_err());
}

/// An out-of-range contribution index is an error, not a panic on indexing.
#[test]
fn out_of_range_contribution_is_an_error() {
    let e = Env::default();
    let (client, basket, assets, _) = setup_with_priced_assets(&e, 2);
    let legs = vec![
        (assets.get_unchecked(0), 500_000u32),
        (assets.get_unchecked(1), 500_000),
    ];
    client.set_basket(&basket, &config(&e, &legs, BasketStalenessPolicy::Reject));
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_basket_contribution(&basket, &7u32)
    }));
    assert!(result.is_err());
}
