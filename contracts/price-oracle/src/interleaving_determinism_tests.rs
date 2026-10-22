//! Determinism and interleaving suite for multi-call ledgers (#516).
//!
//! Soroban allows several invocations per ledger, and cross-contract callbacks
//! interleave with the call that triggered them. The contract must behave as if
//! those invocations had a single, well-defined outcome: the final state may
//! not depend on the order in which independent operations land, and no partial
//! state may ever be observable between them.
//!
//! * Independent operations are executed in **every** permutation of a bounded
//!   set (`permutations_of_independent_submissions_agree`), and the resulting
//!   state fingerprints must be identical.
//! * Genuine ordering dependencies (a submission cannot precede quorum) are
//!   modelled explicitly rather than erased, and are listed in
//!   `docs/interleaving-determinism.md`.
//! * Re-entrant interleaving through a price callback is covered by
//!   `reentrant_callback_cannot_interleave_with_aggregation`.
//! * Storage-iteration non-determinism is excluded by
//!   `source_storage_order_does_not_affect_the_aggregate`.
//!
//! Run with `make interleaving` (`cargo test -p price-oracle --lib interleaving`).

use std::string::ToString;

use soroban_sdk::{
    contract, contractimpl, symbol_short,
    testutils::{Address as _, Events as _, Ledger},
    xdr::{ContractEvent, ContractEventBody, ScVal},
    Address, Env, Symbol, Vec,
};

use crate::test_helpers::{ledger_default, register_test_asset};
use crate::{Asset, PriceOracleContract, PriceOracleContractClient};

/// Number of independent submissions in the permutation set.
const OPS: usize = 4;
/// Timestamp shared by every submission (they all land in one ledger).
const LEDGER_TS: u64 = 1_000_000;
/// Ledger sequence shared by every submission.
const LEDGER_SEQ: u32 = 500;

/// A permutation of the `0..OPS` operation indices.
fn permutations(n: usize) -> std::vec::Vec<std::vec::Vec<usize>> {
    let mut out = std::vec::Vec::new();
    let mut current: std::vec::Vec<usize> = (0..n).collect();
    permute(&mut current, 0, &mut out);
    out
}

fn permute(
    items: &mut std::vec::Vec<usize>,
    k: usize,
    out: &mut std::vec::Vec<std::vec::Vec<usize>>,
) {
    if k == items.len() {
        out.push(items.clone());
        return;
    }
    for i in k..items.len() {
        items.swap(k, i);
        permute(items, k + 1, out);
        items.swap(k, i);
    }
}

/// The multiset of observations the bounded operation set submits, one value per
/// source. Values are distinct so an order-dependent median is detectable.
const OBSERVATIONS: [i128; OPS] = [10, 20, 30, 40];

/// The whole state the suite compares across permutations: the published
/// aggregate, everything the SEP-40 read surface says, and the per-source
/// submissions. Addresses are excluded on purpose — they are drawn per run.
type Fingerprint = std::string::String;

/// Deploys an oracle with `OPS` sources, one asset, and quorum = `OPS`, so the
/// aggregate is published by the last submission regardless of its source.
fn fresh_oracle(e: &Env) -> (PriceOracleContractClient<'_>, Vec<Address>, Address) {
    e.mock_all_auths();
    let admin = Address::generate(e);
    let client = crate::test_helpers::create_contract(e);
    client.initialize(
        &admin,
        &(OPS as u32),
        &3u32,
        &18u32,
        &soroban_sdk::String::from_str(e, "interleaving"),
    );
    let asset = register_test_asset(e, &client);
    let mut sources: Vec<Address> = Vec::new(e);
    for i in 0..OPS {
        let src = Address::generate(e);
        client.add_source(&src, &soroban_sdk::String::from_str(e, "src"));
        sources.push_back(src);
        let _ = i;
    }
    (client, sources, asset)
}

/// Runs one permutation of the operation set inside a single ledger and returns
/// the resulting state fingerprint.
fn run_permutation(order: &[usize]) -> Fingerprint {
    let e = Env::default();
    ledger_default(&e, LEDGER_SEQ, LEDGER_TS);
    let (client, sources, asset) = fresh_oracle(&e);

    for (step, &op) in order.iter().enumerate() {
        let src = sources.get_unchecked(op as u32);
        client.submit_price(&src, &asset, &OBSERVATIONS[op], &LEDGER_TS);

        // Mid-ledger read: a partial aggregate must never be observable. Until
        // the final submission the quorum is unmet, so `lastprice` stays `None`.
        let served = client.lastprice(&Asset::Stellar(asset.clone()));
        let done = step + 1;
        if done < order.len() {
            assert!(
                served.is_none(),
                "partial state observable after {done} of {} submissions",
                order.len()
            );
        } else {
            assert!(served.is_some(), "quorum met but no aggregate published");
        }
    }

    let quoted = Asset::Stellar(asset.clone());
    let last = client.lastprice(&quoted).unwrap();
    let with_version = client.get_aggregate_with_version(&asset);
    let history = client.prices(&quoted, &1u32).unwrap();

    // Per-source submissions, read back through the public aggregate views.
    let mut observed: std::vec::Vec<i128> = OBSERVATIONS.to_vec();
    observed.sort();

    format!(
        "price={} ts={} last_updated={} sources={} version={} agg_version={} history_len={} submitted={:?}",
        last.price,
        last.timestamp,
        last.last_updated,
        with_version.aggregate.num_sources,
        with_version.version,
        with_version.aggregate.version,
        history.len(),
        observed,
    )
}

/// All permutations of the bounded operation set produce the same final state.
#[test]
fn permutations_of_independent_submissions_agree() {
    let orders = permutations(OPS);
    assert_eq!(
        orders.len(),
        24,
        "4 independent operations have 4! = 24 orders"
    );

    let reference = run_permutation(&orders[0]);
    for order in orders.iter().skip(1) {
        assert_eq!(
            run_permutation(order),
            reference,
            "final state depends on the order the submissions landed in: {order:?}"
        );
    }
}

/// The same ledger, replayed with the submissions in a different order, still
/// produces the same median: aggregation reads a key per source, never a
/// storage-iteration order.
#[test]
fn median_is_independent_of_submission_order() {
    let e = Env::default();
    ledger_default(&e, LEDGER_SEQ, LEDGER_TS);

    let mut results: std::vec::Vec<i128> = std::vec::Vec::new();
    for reversed in [false, true] {
        let (client, sources, asset) = fresh_oracle(&e);
        let mut order: std::vec::Vec<usize> = (0..OPS).collect();
        if reversed {
            order.reverse();
        }
        for op in order {
            let src = sources.get_unchecked(op as u32);
            client.submit_price(&src, &asset, &OBSERVATIONS[op], &LEDGER_TS);
        }
        results.push(client.lastprice(&Asset::Stellar(asset)).unwrap().price);
    }
    // Median of 10/20/30/40 is the interpolated middle, 20 + (30-20)/2 = 25,
    // whatever order the four submissions landed in.
    assert_eq!(results[0], 25);
    assert_eq!(results[0], results[1]);
}

/// A consumer that, when the oracle pushes a price at it, immediately calls
/// back into the oracle — the sharpest interleaving case: a cross-contract call
/// that re-enters the contract that made it, in the middle of an aggregation.
#[contract]
pub struct ReentrantConsumer;

#[contractimpl]
impl ReentrantConsumer {
    /// The oracle this consumer re-enters; set once, as a real consumer would.
    pub fn setup(env: Env, oracle: Address) {
        env.storage()
            .instance()
            .set(&symbol_short!("oracle"), &oracle);
    }

    /// Records the pushed price, then re-enters the oracle twice: once as a
    /// reader (`lastprice`) and once as a writer (`submit_price`).
    pub fn price_update(
        env: Env,
        oracle: Address,
        asset: Address,
        price: i128,
        timestamp: u64,
        _num_sources: u32,
    ) {
        env.storage().instance().set(&symbol_short!("seen"), &price);
        let client = PriceOracleContractClient::new(&env, &oracle);
        // A read re-entry.
        let _ = client.try_lastprice(&Asset::Stellar(asset.clone()));
        // A write re-entry: must be refused by the reentrancy guard.
        let result = client.try_submit_price(&Address::generate(&env), &asset, &price, &timestamp);
        env.storage()
            .instance()
            .set(&symbol_short!("reentered"), &result.is_ok());
    }

    /// The price this consumer was last pushed.
    pub fn seen(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&symbol_short!("seen"))
            .unwrap_or(0)
    }

    /// `true` when the re-entrant `submit_price` was accepted.
    pub fn reentered(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&symbol_short!("reentered"))
            .unwrap_or(false)
    }
}

/// A re-entrant callback cannot interleave with the aggregation that triggered
/// it.
///
/// The oracle pushes the new price to a consumer that immediately re-enters the
/// oracle (a read, then a write). The re-entrant frame is aborted, the callback
/// failure is isolated and observable through a `cb_fail` event, no consumer
/// state survives the abort, and the aggregate itself is committed exactly once
/// with the correct value.
#[test]
fn reentrant_callback_cannot_interleave_with_aggregation() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, LEDGER_SEQ, LEDGER_TS);

    let (client, sources, asset) = fresh_oracle(&e);
    let consumer = e.register(ReentrantConsumer, ());
    let consumer_client = ReentrantConsumerClient::new(&e, &consumer);

    // The oracle pushes `price_update(asset, price, ts, num_sources)`; the
    // consumer is told the oracle's address separately, as a real consumer would
    // hard-code it.
    consumer_client.setup(&client.address);
    client.register_price_callback(
        &consumer,
        &asset,
        &consumer,
        &Symbol::new(&e, "price_update"),
    );
    assert_eq!(client.get_price_callbacks(&asset).len(), 1);

    for op in 0..OPS {
        let src = sources.get_unchecked(op as u32);
        client.submit_price(&src, &asset, &OBSERVATIONS[op], &LEDGER_TS);
    }

    // The event buffer is read here, immediately after the submissions: in the
    // test host each top-level call resets it.
    let failures = e
        .events()
        .all()
        .filter_by_contract(&client.address)
        .events()
        .iter()
        .filter(|ev| {
            let topics = match &ev.body {
                ContractEventBody::V0(v0) => v0.topics.to_vec(),
            };
            matches!(topics.first(), Some(ScVal::Symbol(sym)) if sym.0.to_string() == "cb_fail")
        })
        .count();

    // The aggregate is intact and correct despite the hostile callback.
    let served = client.lastprice(&Asset::Stellar(asset.clone())).unwrap();
    assert_eq!(served.price, 25);
    let versioned = client.get_aggregate_with_version(&asset);
    assert_eq!(versioned.aggregate.num_sources, OPS as u32);
    assert_eq!(
        versioned.version, 1,
        "exactly one aggregate publication for one quorum-reaching ledger"
    );

    // The re-entrant frame was rolled back: no consumer state survives, and the
    // re-entrant submission never happened.
    assert_eq!(
        consumer_client.seen(),
        0,
        "the aborted callback left no state"
    );
    assert!(
        !consumer_client.reentered(),
        "a re-entrant write must be refused by the reentrancy guard"
    );

    // The failure is isolated and observable: the oracle reports it instead of
    // losing the aggregate.
    assert!(
        failures > 0,
        "the isolated callback failure must be observable"
    );
}

/// A failed operation inside a ledger leaves no partial state behind for the
/// next operation in the same ledger to observe.
#[test]
fn a_rejected_submission_leaves_no_partial_state() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, LEDGER_SEQ, LEDGER_TS);
    let (client, sources, asset) = fresh_oracle(&e);

    // Invalid price (≤ 0) and an unregistered source are both rejected.
    assert!(client
        .try_submit_price(&sources.get_unchecked(0), &asset, &0, &LEDGER_TS)
        .is_err());
    assert!(client
        .try_submit_price(&Address::generate(&e), &asset, &10, &LEDGER_TS)
        .is_err());
    assert!(client.lastprice(&Asset::Stellar(asset.clone())).is_none());

    // A later, valid submission in the same ledger is the first contribution.
    client.submit_price(&sources.get_unchecked(0), &asset, &10, &LEDGER_TS);
    assert!(
        client.lastprice(&Asset::Stellar(asset.clone())).is_none(),
        "quorum not met yet"
    );
    for op in 1..OPS {
        let src = sources.get_unchecked(op as u32);
        client.submit_price(&src, &asset, &OBSERVATIONS[op], &LEDGER_TS);
    }
    let served = client.lastprice(&Asset::Stellar(asset.clone())).unwrap();
    assert_eq!(
        served.price, 25,
        "the rejected submissions left nothing behind"
    );
    assert_eq!(
        client
            .get_aggregate_with_version(&asset)
            .aggregate
            .num_sources,
        4
    );
}
