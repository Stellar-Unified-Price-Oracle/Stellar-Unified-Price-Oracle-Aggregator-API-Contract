//! Cross-contract integration scenarios on the hermetic harness (#519).
//!
//! These exercise the aggregator the way a real deployment does — through
//! contract calls from other contracts, over the published SEP-40 interface —
//! rather than through direct state manipulation. Everything runs in-process on
//! a deterministic fixture (see [`crate::hermetic_harness`]), so the suite is
//! repeatable and needs no network.
//!
//! Coverage:
//!
//! * the full submit → aggregate → read path across multiple sources and assets;
//! * a SEP-40 consumer contract reading prices through the standard interface;
//! * a second, independent oracle contract, to confirm the aggregator is not
//!   coupled to one caller's shape;
//! * aggregation-method selection and quorum behaviour;
//! * state isolation between scenarios, and a replay determinism check.
//!
//! Replaces the need for a shared testnet for these paths. It does not replace
//! the testnet lifecycle job: see `docs/hermetic-harness.md` for the known
//! divergences between this harness and the real network.

#![cfg(test)]

use soroban_sdk::{
    contract, contractimpl, testutils::Address as _, Address, Env, String, Symbol, Vec,
};

use crate::hermetic_harness::{HermeticHarness, BASE_TIMESTAMP, DECIMALS};
use crate::types::{AggregationMethod, Asset, PriceData};
use crate::{ErrorCode, PriceOracleContractClient};

/// A SEP-40 style consumer, deployed as a real contract so the aggregator is
/// reached through a cross-contract call exactly as it would be on-chain.
#[contract]
pub struct MockConsumer;

#[contractimpl]
impl MockConsumer {
    /// Reads the aggregator's base asset and checks it is USD.
    pub fn read_base(env: Env, oracle: Address) -> bool {
        let client = PriceOracleContractClient::new(&env, &oracle);
        matches!(client.base(), Asset::Other(ref s) if *s == Symbol::new(&env, "USD"))
    }

    /// Number of assets the aggregator advertises.
    pub fn assets_count(env: Env, oracle: Address) -> u32 {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.assets().len()
    }

    /// Decimals the aggregator reports.
    pub fn decimals(env: Env, oracle: Address) -> u32 {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.decimals()
    }

    /// Latest price via the SEP-40 `lastprice` entrypoint.
    pub fn last_price(env: Env, oracle: Address, asset: Address) -> Option<PriceData> {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.lastprice(&Asset::Stellar(asset))
    }

    /// Aggregated price for an asset, or `0` when none is published.
    pub fn price_or_zero(env: Env, oracle: Address, asset: Address) -> i128 {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.get_price(&asset, &0).map(|p| p.price).unwrap_or(0)
    }
}

/// A second, independent oracle contract.
///
/// Its only role is to be a *different* contract calling the aggregator, which
/// is what makes these tests cross-contract rather than a single contract
/// talking to itself.
#[contract]
pub struct IndependentOracle;

#[contractimpl]
impl IndependentOracle {
    /// Consumes a price and derives a doubled figure, so the test can assert
    /// the caller's own computation happened on top of the aggregator's value.
    pub fn doubled_price(env: Env, oracle: Address, asset: Address) -> i128 {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client
            .get_price(&asset, &0)
            .map(|p| p.price * 2)
            .unwrap_or(0)
    }

    /// Publishes a price into the aggregator, so writes are cross-contract too.
    pub fn relay_price(env: Env, oracle: Address, source: Address, asset: Address, price: i128) {
        let client = PriceOracleContractClient::new(&env, &oracle);
        client.submit_price(&source, &asset, &price, &env.ledger().timestamp());
    }
}

fn deploy_consumer(e: &Env) -> Address {
    e.register(MockConsumer, ())
}

fn deploy_independent(e: &Env) -> Address {
    e.register(IndependentOracle, ())
}

// ─────────────────────────────────────────────────────────────────────────────
// State reset
// ─────────────────────────────────────────────────────────────────────────────

/// A freshly deployed harness starts from a clean world.
///
/// This is the state-reset guarantee the issue asks for, asserted directly:
/// nothing from a previous scenario can be present, because each scenario owns
/// its own `Env` and its own deployment.
#[test]
fn fresh_harness_is_clean() {
    let e = Env::default();
    let h = HermeticHarness::deploy(&e);
    h.assert_clean();
}

/// Two harnesses in the same `Env` are independent: registering sources on one
/// must not affect the other. Storage is namespaced per contract, and this is
/// the test that would catch a regression in that assumption.
#[test]
fn harnesses_are_isolated_from_each_other() {
    let e = Env::default();
    let mut a = HermeticHarness::deploy(&e);
    a.add_sources(2).add_assets(1);

    let b = HermeticHarness::deploy(&e);
    b.assert_clean();

    assert_eq!(a.client().get_oracle_sources().sources.len(), 2);
    assert_eq!(b.client().get_oracle_sources().sources.len(), 0);
}

// ─────────────────────────────────────────────────────────────────────────────
// End-to-end submit → aggregate → read
// ─────────────────────────────────────────────────────────────────────────────

/// The core path: several sources submit, the median is published, and a
/// consumer contract reads it back over SEP-40.
#[test]
fn multi_source_aggregation_is_consumable_cross_contract() {
    let e = Env::default();
    let mut h = HermeticHarness::deploy(&e);
    h.add_sources(3).add_assets(1);
    let consumer = deploy_consumer(&e);
    let oracle = h.address();
    let asset = h.asset(0);

    // No price before anyone submits.
    assert_eq!(
        MockConsumerClient::new(&e, &consumer).price_or_zero(&oracle, &asset),
        0,
        "no price should be published before any submission"
    );

    h.at_ledger(10);
    h.submit(0, 0, 1_000_000);
    h.at_ledger(11);
    h.submit(1, 0, 2_000_000);
    h.at_ledger(12);
    h.submit(2, 0, 3_000_000);

    // Median of {1, 2, 3} is 2.
    let published = h
        .client()
        .get_price(&asset, &0)
        .expect("price must publish");
    assert_eq!(published.price, 2_000_000, "aggregate must be the median");
    assert_eq!(published.num_sources, 3);

    // …and a separate contract reads exactly that.
    let consumer_client = MockConsumerClient::new(&e, &consumer);
    assert_eq!(
        consumer_client.price_or_zero(&oracle, &asset),
        2_000_000,
        "a consumer contract must observe the published median"
    );
    assert_eq!(consumer_client.decimals(&oracle), DECIMALS);
    assert!(consumer_client.read_base(&oracle));
    assert_eq!(consumer_client.assets_count(&oracle), 1);

    // SEP-40 `lastprice` must agree with `get_price`.
    let sep40 = consumer_client
        .last_price(&oracle, &asset)
        .expect("SEP-40 lastprice must be available");
    assert_eq!(sep40.price, 2_000_000);
    assert_eq!(sep40.timestamp, BASE_TIMESTAMP + 12 * 5);
}

/// A second contract can drive writes as well as reads, and its own arithmetic
/// on top of the aggregator's value must be observable.
#[test]
fn second_contract_can_write_and_compute() {
    let e = Env::default();
    let mut h = HermeticHarness::deploy(&e);
    h.add_sources(2).add_assets(1);
    let oracle_addr = h.address();
    let other = deploy_independent(&e);
    let asset = h.asset(0);
    let source = h.source(0);

    h.at_ledger(20);
    // `submit_price` requires the *source's* authorization, and here the call
    // originates inside another contract rather than at the root. The SDK's
    // `mock_all_auths` only satisfies authorizations recorded in the root
    // invocation, so it rejects this; the non-root variant is the documented
    // way to test a contract that bundles calls to another contract.
    e.mock_all_auths_allowing_non_root_auth();
    let other_client = IndependentOracleClient::new(&e, &other);
    other_client.relay_price(&oracle_addr, &source, &asset, &1_500_000i128);

    // Quorum is 2, so one submission is not yet an aggregate.
    assert_eq!(
        other_client.doubled_price(&oracle_addr, &asset),
        0,
        "a single submission must not satisfy a quorum of 2"
    );

    h.at_ledger(21);
    h.submit(1, 0, 1_500_000);

    assert_eq!(
        other_client.doubled_price(&oracle_addr, &asset),
        3_000_000,
        "the consumer's own arithmetic must be visible once quorum is met"
    );
}

/// Several assets are tracked independently, with no cross-talk.
#[test]
fn multiple_assets_are_independent() {
    let e = Env::default();
    let mut h = HermeticHarness::deploy(&e);
    h.add_sources(2).add_assets(2);
    let consumer = deploy_consumer(&e);
    let oracle = h.address();

    h.at_ledger(30);
    h.submit(0, 0, 100);
    h.submit(1, 0, 100);
    h.submit(0, 1, 900);
    h.submit(1, 1, 900);

    let c = MockConsumerClient::new(&e, &consumer);
    assert_eq!(c.price_or_zero(&oracle, &h.asset(0)), 100);
    assert_eq!(c.price_or_zero(&oracle, &h.asset(1)), 900);
    assert_eq!(c.assets_count(&oracle), 2);
}

// ─────────────────────────────────────────────────────────────────────────────
// Aggregation methods
// ─────────────────────────────────────────────────────────────────────────────

/// The aggregation method is honoured end to end, not just stored: switching to
/// the mean changes the published value for the same submissions.
#[test]
fn aggregation_method_changes_the_published_value() {
    let e = Env::default();
    let mut h = HermeticHarness::deploy(&e);
    h.add_sources(3).add_assets(1);
    let asset = h.asset(0);

    h.at_ledger(40);
    h.submit(0, 0, 1_000);
    h.submit(1, 0, 2_000);
    h.submit(2, 0, 6_000);

    // Default is median: 2_000.
    assert_eq!(
        h.client().get_price(&asset, &0).unwrap().price,
        2_000,
        "default aggregation must be the median"
    );

    h.client()
        .set_aggregation_method(&(AggregationMethod::Mean as u32));
    h.at_ledger(41);
    h.submit(0, 0, 1_000);
    h.submit(1, 0, 2_000);
    h.submit(2, 0, 6_000);

    // Mean of {1, 2, 6} = 3_000.
    assert_eq!(
        h.client().get_price(&asset, &0).unwrap().price,
        3_000,
        "mean aggregation must average the same submissions"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Determinism
// ─────────────────────────────────────────────────────────────────────────────

/// Replaying an identical scenario twice in fresh `Env`s produces an identical
/// result.
///
/// This is the determinism the harness promises, asserted rather than assumed.
/// The harness script additionally runs the whole suite twice and diffs the
/// output, which covers repetition across processes; this test covers repetition
/// within one.
#[test]
fn scenario_replay_is_deterministic() {
    let run = || {
        let e = Env::default();
        let mut h = HermeticHarness::deploy(&e);
        h.add_sources(3).add_assets(2);

        h.at_ledger(50);
        h.submit(0, 0, 1_234_567i128);
        h.at_ledger(51);
        h.submit(1, 0, 1_234_999);
        h.at_ledger(52);
        h.submit(2, 0, 1_234_000);
        h.submit(0, 1, 9_999_999);
        h.submit(1, 1, 9_999_999);
        h.submit(2, 1, 9_999_999);

        let a = h.client().get_price(&h.asset(0), &0).unwrap();
        let b = h.client().get_price(&h.asset(1), &0).unwrap();
        let history = h.client().get_price_history(&h.asset(0), &0u32, &3u32);
        (a, b, history.len())
    };

    assert_eq!(
        run(),
        run(),
        "identical scenarios must produce identical results"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Fail-closed paths across contracts
// ─────────────────────────────────────────────────────────────────────────────

/// An unregistered asset read from another contract yields no price rather than
/// a default. Guards the fail-closed property through a cross-contract call.
#[test]
fn unregistered_asset_fails_closed_across_contracts() {
    let e = Env::default();
    let mut h = HermeticHarness::deploy(&e);
    h.add_sources(1).add_assets(1);
    let consumer = deploy_consumer(&e);
    let oracle = h.address();

    let stranger = Address::generate(&e);

    // `price_or_zero` calls the aggregator, which rejects an unregistered asset.
    // The error propagates out of the consumer call rather than being flattened
    // to a 0 — which is the fail-closed property that matters: a consumer must
    // never be handed a plausible-looking price for an unknown asset.
    let res = MockConsumerClient::new(&e, &consumer).try_price_or_zero(&oracle, &stranger);
    assert!(
        res.is_err(),
        "an unregistered asset must not yield a price to a consumer"
    );

    // The underlying cause is the distinct AssetNotRegistered code.
    assert_eq!(
        h.client().try_get_aggregate_with_version(&stranger),
        Err(Ok(ErrorCode::AssetNotRegistered.into())),
        "unregistered asset must error rather than fabricate a price"
    );
}

/// A source submitting for an unregistered asset is rejected, so a bad write
/// cannot create state.
#[test]
fn submission_for_unregistered_asset_is_rejected() {
    let e = Env::default();
    let mut h = HermeticHarness::deploy(&e);
    h.add_sources(1).add_assets(1);

    let stranger = Address::generate(&e);
    h.at_ledger(60);
    assert_eq!(
        h.client()
            .try_submit_price(&h.source(0), &stranger, &1_000i128, &h.now()),
        Err(Ok(ErrorCode::AssetNotRegistered.into())),
        "submitting for an unregistered asset must be rejected"
    );
}
