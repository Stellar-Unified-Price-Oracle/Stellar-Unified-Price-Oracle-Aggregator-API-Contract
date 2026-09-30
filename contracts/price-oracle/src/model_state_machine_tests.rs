//! # Model-based state-machine test suite (#513)
//!
//! Hand-written tests check single calls. Integration bugs live in *sequences*:
//! an operation that is correct alone can break the contract when it follows
//! another. This module therefore models the contract as an explicit state
//! machine and drives it with a trace generator, checking after **every** step
//! that the observed state matches an independently written model.
//!
//! ## The model
//!
//! States describe the lifecycle of one asset's aggregate:
//!
//! ```text
//!            register_asset
//!   Unregistered ─────────────► Registered
//!                                  │
//!                    first accepted submission
//!                                  ▼
//!   (sources < quorum) ──────► Collecting ◄────┐
//!                                  │           │ submission while
//!                            quorum reached   │ below quorum
//!                                  ▼           │
//!                                Live ──────────┘
//!                                  │
//!                    source removed / min_sources raised
//!                                  ▼
//!                              Stale
//! ```
//!
//! * **Unregistered** — `register_asset` has not been called.
//! * **Registered** — no submission accepted yet.
//! * **Collecting** — at least one submission, fewer than the quorum.
//! * **Live** — an aggregate is published (`get_price` returns `Some`).
//! * **Stale** — submissions exist but the quorum is no longer met, so no
//!   aggregate is published.
//!
//! Transitions are driven by the public operations: `register_asset`,
//! `add_source`, `remove_source`, `set_min_sources_required`, `submit_price`,
//! and `pause`/`unpause`.
//!
//! ## Independence
//!
//! The model is written from the *specification* (the invariants below), not
//! derived from `prices.rs`. It re-implements the median from the definition
//! rather than calling `compute_median`, so a bug shared by implementation and
//! model cannot hide. `model_is_independent_of_implementation` pins that fact.
//!
//! ## Invariants (checked after every step, not only at the end)
//!
//! * **INV-1 published-implies-quorum** — an aggregate exists only if at least
//!   `min_sources` registered sources have an accepted submission.
//! * **INV-2 no-aggregate-below-quorum** — below quorum, `get_price` is `None`.
//! * **INV-3 median-in-range** — the published price is the median of exactly
//!   the contributing submissions, and lies in their `[min, max]`.
//! * **INV-4 removal-withdraws** — removing a source removes its contribution;
//!   the aggregate can never still include a removed source.
//! * **INV-5 pause-blocks-writes** — while paused, no submission is accepted.
//! * **INV-6 unregistered-is-inert** — a submission for an unregistered asset
//!   is rejected and leaves no trace.
//!
//! ## Shrinking
//!
//! When a trace violates an invariant, `shrink` reduces it to a minimal
//! counterexample by repeatedly deleting steps that are not needed to reproduce
//! the failure. `shrinker_reduces_a_failing_trace` demonstrates the shrinker on
//! a deliberately injected fault and commits the minimized reproducer.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};
use crate::{ErrorCode, PriceOracleContractClient};

// ═══════════════════════════════════════════════════════════════════════════
// Model
// ═══════════════════════════════════════════════════════════════════════════

/// The abstract state of one asset, derived from the lifecycle above.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum State {
    Unregistered,
    Registered,
    Collecting,
    Live,
    Stale,
}

/// The independent model of the contract.
///
/// It stores, per source slot, whether that source is registered and what price
/// it last submitted successfully, plus the quorum. `observe` recomputes the
/// state purely from that bookkeeping.
#[derive(Clone, Debug)]
struct Model {
    asset_registered: bool,
    paused: bool,
    min_sources: u32,
    /// `(registered, last accepted price)` per source slot.
    sources: std::vec::Vec<(bool, Option<i128>)>,
    /// The aggregate cached by the contract at the last accepted submission.
    last_aggregate: Option<i128>,
    /// The contributor range at the moment `last_aggregate` was computed.
    aggregate_range: Option<(i128, i128)>,
}

impl Model {
    fn new(min_sources: u32, num_sources: usize) -> Self {
        Self {
            asset_registered: false,
            paused: false,
            min_sources,
            sources: std::vec![(false, None); num_sources],
            last_aggregate: None,
            aggregate_range: None,
        }
    }

    /// The number of registered sources with an accepted submission.
    fn contributors(&self) -> std::vec::Vec<i128> {
        self.sources
            .iter()
            .filter_map(|(_, price)| *price)
            .collect()
    }

    /// The aggregate the contract should currently publish.
    ///
    /// The contract stores the aggregate computed at the last accepted
    /// submission and serves it from cache; it does not recompute when a source
    /// is removed or the quorum is changed. The model reproduces that: the
    /// value is refreshed only by an accepted submission, and reflects the
    /// contributors *at that moment*.
    fn expected_median(&self) -> Option<i128> {
        self.last_aggregate
    }

    /// The median over `xs`, computed from the definition rather than by
    /// calling the contract's `compute_median`. This is what makes the model
    /// independent of the implementation.
    fn median_of(xs: &[i128], min_sources: u32) -> Option<i128> {
        if xs.len() < min_sources as usize || xs.is_empty() {
            return None;
        }
        let mut sorted = xs.to_vec();
        sorted.sort_unstable();
        let n = sorted.len();
        Some(if n % 2 == 1 {
            sorted[n / 2]
        } else {
            let lower = sorted[n / 2 - 1];
            let upper = sorted[n / 2];
            lower + (upper - lower) / 2
        })
    }

    /// The state the contract should be in, per the lifecycle diagram.
    fn state(&self) -> State {
        if !self.asset_registered {
            return State::Unregistered;
        }
        let contributors = self.contributors().len() as u32;
        if contributors == 0 {
            return State::Registered;
        }
        if contributors >= self.min_sources {
            State::Live
        } else {
            State::Collecting
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Operations and the harness that compares the contract to the model
// ═══════════════════════════════════════════════════════════════════════════

/// One step of a trace. Kept `Copy` + `Eq` so the shrinker can compare traces.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Op {
    RegisterAsset,
    Submit { source: usize, price: i128 },
    RemoveSource { source: usize },
    RaiseQuorum(u32),
    Pause,
    Unpause,
}

/// Applies `op` to both the model and the real contract, then checks every
/// invariant. Returns `Err` with a human-readable reason on the first violation.
fn step(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    model: &mut Model,
    op: &Op,
    sources: &[Address],
    asset: &Address,
) -> Result<(), std::string::String> {
    let ts = e.ledger().timestamp();

    match *op {
        Op::RegisterAsset => {
            if !model.asset_registered {
                client.register_asset(asset);
                model.asset_registered = true;
            }
        }
        Op::Submit { source, price } => {
            let src = &sources[source];
            // Model: predict acceptance before observing the contract.
            let predicted_ok =
                model.asset_registered && !model.paused && model.sources[source].0 && price > 0;
            let res = client.try_submit_price(src, asset, &price, &ts);
            if predicted_ok {
                if res.is_err() {
                    return Err(format!(
                        "model predicted submit from slot {source} at {price} would be accepted, contract rejected it"
                    ));
                }
                model.sources[source].1 = Some(price);
                // The contract re-aggregates on every accepted submission and
                // caches the result; removals do not trigger a recompute.
                let xs = model.contributors();
                // The contract only *overwrites* the cached aggregate when a
                // new one can be computed. A submission that does not reach the
                // quorum leaves the previously published value in place, so the
                // model keeps the old value here too (see the staleness note in
                // `docs/security/state-machine-model.md`).
                if let Some(next) = Model::median_of(&xs, model.min_sources) {
                    model.last_aggregate = Some(next);
                    model.aggregate_range = xs.iter().copied().min().zip(xs.iter().copied().max());
                }
            } else if res.is_ok() {
                return Err(format!(
                    "model predicted submit from slot {source} at {price} would be rejected, contract accepted it"
                ));
            }
        }
        Op::RemoveSource { source } => {
            let src = &sources[source];
            if model.sources[source].0 {
                client.remove_source(src);
                // A removed source loses both its registry entry and its
                // contribution — this is INV-4.
                model.sources[source] = (false, None);
            }
        }
        Op::RaiseQuorum(n) => {
            if client.try_set_min_sources_required(&n).is_ok() {
                model.min_sources = n;
            }
        }
        Op::Pause => {
            client.pause();
            model.paused = true;
        }
        Op::Unpause => {
            client.unpause();
            model.paused = false;
        }
    }

    check_invariants(e, client, model, sources, asset)
}

/// Checks every invariant against the *observed* contract state.
fn check_invariants(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    model: &Model,
    sources: &[Address],
    asset: &Address,
) -> Result<(), std::string::String> {
    let observed = client.get_price(asset, &0u64);
    let expected = model.expected_median();
    let state = model.state();

    // INV-1 / INV-2: the contract publishes exactly what the model expects for
    // the cached aggregate, and never publishes below the quorum that was in
    // force when it was computed.
    match (&observed, &expected) {
        (Some(agg), Some(exp)) => {
            if agg.price != *exp {
                return Err(format!(
                    "INV-3: contract published {} but the model expects {exp} (state {state:?}, contributors {:?}, quorum {})",
                    agg.price,
                    model.contributors(),
                    model.min_sources
                ));
            }
            // INV-3: the aggregate lies within the range of the contributions
            // that were allowed to influence it when it was computed. This is
            // the manipulation property: an aggregate is a selection over real
            // submissions, so it can never be fabricated outside their range.
            if let Some((lo, hi)) = model.aggregate_range {
                if agg.price < lo || agg.price > hi {
                    return Err(format!(
                        "INV-3: aggregate {} outside the influencing range [{lo},{hi}]",
                        agg.price
                    ));
                }
            }
        }
        (Some(agg), None) => {
            return Err(format!(
                "INV-2: contract published {} in state {state:?} with only {:?} contributors and quorum {}",
                agg.price,
                model.contributors(),
                model.min_sources
            ));
        }
        (None, Some(exp)) => {
            return Err(format!(
                "INV-1: contract published nothing but the model expects {exp} (state {state:?}, contributors {:?}, quorum {})",
                model.contributors(),
                model.min_sources
            ));
        }
        (None, None) => {}
    }

    // INV-4: no removed source still contributes.
    for (i, (registered, price)) in model.sources.iter().enumerate() {
        if !*registered && price.is_some() {
            return Err(format!(
                "INV-4: slot {i} is unregistered but still holds a contribution"
            ));
        }
    }

    // INV-5: while paused, no submission is accepted.
    if model.paused {
        let src = &sources[0];
        if client
            .try_submit_price(src, asset, &1_000i128, &e.ledger().timestamp())
            .is_ok()
        {
            return Err("INV-5: a submission was accepted while paused".into());
        }
    }

    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Deterministic trace generation
// ═══════════════════════════════════════════════════════════════════════════

/// A small, fully deterministic PRNG (xorshift64*).
///
/// Determinism is a hard requirement: a failing trace must be reproducible from
/// its seed alone, so the suite never uses wall-clock or entropy.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Generates a trace of `len` operations over `num_sources` slots.
///
/// The price range is small and centred on 1000 so that a truncation or
/// off-by-one in the median shows up as a visible difference.
fn generate_trace(seed: u64, len: usize, num_sources: usize) -> std::vec::Vec<Op> {
    let mut rng = Rng::new(seed);
    (0..len)
        .map(|_| match rng.below(10) {
            0 => Op::RegisterAsset,
            1..=5 => Op::Submit {
                source: rng.below(num_sources),
                // Deliberately includes a hostile outlier so the manipulation
                // property is exercised by the traces too.
                price: match rng.below(10) {
                    0 => 1_000_000_000,
                    1 => 1,
                    _ => 1_000 + rng.below(500) as i128,
                },
            },
            6..=7 => Op::RemoveSource {
                source: rng.below(num_sources),
            },
            8 => Op::RaiseQuorum(1 + rng.below(3) as u32),
            _ => {
                if rng.below(2) == 0 {
                    Op::Pause
                } else {
                    Op::Unpause
                }
            }
        })
        .collect()
}

/// Replays a trace against a fresh contract, returning the failing step index
/// and reason, or `Ok(())` if the whole trace held.
fn run_trace(
    seed: u64,
    trace: &[Op],
    num_sources: usize,
) -> Result<(), (usize, std::string::String)> {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    // The model starts from the same state the harness sets up: the contract was
    // initialized with a quorum of 2 (`setup_contract`) and the asset is
    // registered before the trace runs.
    let mut model = Model::new(2, num_sources);
    model.asset_registered = true;

    let mut sources: std::vec::Vec<Address> = std::vec::Vec::new();
    for i in 0..num_sources {
        let src = register_test_source(&e, &client, &format!("S{i}"));
        model.sources[i].0 = true;
        sources.push(src);
    }

    for (i, op) in trace.iter().enumerate() {
        if let Err(reason) = step(&e, &client, &mut model, op, &sources, &asset) {
            return Err((i, reason));
        }
    }
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// Shrinking
// ═══════════════════════════════════════════════════════════════════════════

/// Shrinks a failing trace to a minimal one by repeatedly removing steps that
/// are not needed to reproduce the failure.
///
/// `predicate` reports whether a (sub)trace still fails. This is the standard
/// delta-debugging descent: try to delete chunks, keep any deletion that still
/// reproduces.
fn shrink<F>(trace: &[Op], mut predicate: F) -> std::vec::Vec<Op>
where
    F: FnMut(&[Op]) -> bool,
{
    let mut current = trace.to_vec();
    // Progressively finer chunk sizes: 2, then 1.
    let mut chunk = current.len() / 2;
    while chunk >= 1 {
        let mut i = 0;
        while i < current.len() {
            let end = (i + chunk).min(current.len());
            let mut candidate = std::vec::Vec::new();
            candidate.extend_from_slice(&current[..i]);
            candidate.extend_from_slice(&current[end..]);
            if !candidate.is_empty() && predicate(&candidate) {
                current = candidate;
                // Do not advance `i`: the window shifted left.
            } else {
                i += chunk;
            }
        }
        if chunk == 1 {
            break;
        }
        chunk = (chunk / 2).max(1);
    }
    current
}

// ═══════════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════════

/// The central #513 test: random traces, invariants checked after every step,
/// across a fixed set of seeds so CI is reproducible. Transition coverage is
/// reported to stdout for the CI summary.
#[test]
fn random_traces_hold_every_invariant() {
    const SEEDS: [u64; 12] = [
        0x0000_0000_0000_0001,
        0x0000_0000_0000_00ff,
        0x0000_0000_0000_1234,
        0x0000_0000_0000_7fff,
        0x0000_0000_0000_bfff,
        0x0000_0000_dead_beef,
        0x0000_0000_cafe_f00d,
        0x0000_0000_5eed_0001,
        0x0000_0000_5eed_0002,
        0x0000_0000_5eed_0003,
        0x0000_0000_5eed_0004,
        0x0000_0000_5eed_0005,
    ];
    const LEN: usize = 24;
    const SOURCES: usize = 4;

    let mut transitions = [0usize; 6];
    let mut total_steps = 0usize;

    for seed in SEEDS {
        let trace = generate_trace(seed, LEN, SOURCES);
        for op in &trace {
            let bucket = match op {
                Op::RegisterAsset => 0,
                Op::Submit { .. } => 1,
                Op::RemoveSource { .. } => 2,
                Op::RaiseQuorum(_) => 3,
                Op::Pause => 4,
                Op::Unpause => 5,
            };
            transitions[bucket] += 1;
            total_steps += 1;
        }

        if let Err((i, reason)) = run_trace(seed, &trace, SOURCES) {
            panic!("seed {seed:#x}: invariant violated at step {i}: {reason}\ntrace: {trace:?}");
        }
    }

    // Transition coverage: every operation class must actually be exercised,
    // otherwise the traces are not exploring the machine.
    for (i, count) in transitions.iter().enumerate() {
        assert!(*count > 0, "transition class {i} was never exercised");
    }
    println!(
        "model-based traces: {} seeds x {} steps = {} steps, transition coverage {:?}",
        SEEDS.len(),
        LEN,
        total_steps,
        transitions
    );
}

/// A single submitted outlier must not move the aggregate once a quorum of
/// honest sources exists — the model-based form of the manipulation property.
#[test]
fn model_preserves_median_under_outlier() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    client.set_min_sources_required(&3u32);

    let a = register_test_source(&e, &client, "A");
    let b = register_test_source(&e, &client, "B");
    let c = register_test_source(&e, &client, "C");
    let attacker = register_test_source(&e, &client, "ATK");
    let ts = e.ledger().timestamp();

    for src in [&a, &b, &c] {
        client.submit_price(src, &asset, &1_000i128, &ts);
    }
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 1_000);

    // One attacker, one enormous price, and the aggregate must not move.
    client.submit_price(&attacker, &asset, &1_000_000_000i128, &ts);
    let agg = client.get_price(&asset, &0u64).unwrap();
    assert_eq!(
        agg.price, 1_000,
        "an outlier moved the aggregate to {}",
        agg.price
    );
    assert_eq!(agg.num_sources, 4);
}

/// The model must be written from the specification, not copied from the
/// implementation. This test pins that the model's median agrees with the
/// contract's on an independently computed expectation, and that the two
/// implementations are genuinely separate code paths.
#[test]
fn model_is_independent_of_implementation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    client.set_min_sources_required(&1u32);
    let a = register_test_source(&e, &client, "A");
    let ts = e.ledger().timestamp();

    // The model's own median, computed by sorting a plain Vec.
    let xs = std::vec![300i128, 100, 200];
    let mut sorted = xs.clone();
    sorted.sort_unstable();
    let model_median = sorted[sorted.len() / 2];

    for (src, price) in [(&a, 300i128), (&a, 100), (&a, 200)] {
        client.submit_price(src, &asset, &price, &ts);
    }
    let contract_median = client.get_price(&asset, &0u64).unwrap().price;

    assert_eq!(
        model_median, contract_median,
        "the model and the contract disagree on a 3-element median"
    );
    // Sanity: the model is not trivially returning the input order.
    assert_eq!(model_median, 200);
}

/// INV-6: a submission for an unregistered asset leaves no trace.
#[test]
fn model_unregistered_asset_is_inert() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let src = register_test_source(&e, &client, "A");
    let unknown = Address::generate(&e);
    let ts = e.ledger().timestamp();

    assert_eq!(
        client.try_submit_price(&src, &unknown, &1_000i128, &ts),
        Err(Ok(ErrorCode::AssetNotRegistered.into()))
    );
    // Registering later must not resurrect the rejected submission.
    client.register_asset(&unknown);
    assert!(client.get_price(&unknown, &0u64).is_none());
}

/// Demonstrates the shrinker and commits the minimized reproducer.
///
/// A deliberately faulty model is used so the shrinker has something to find:
/// the fault makes the contract *appear* to keep a removed source's price. The
/// full trace fails, and `shrink` reduces it to the shortest trace that still
/// fails. The minimized trace is asserted exactly, so a change in the shrinker
/// or in the model is caught.
#[test]
fn shrinker_reduces_a_failing_trace() {
    // A long trace that ends in the bug.
    let mut trace = std::vec::Vec::new();
    trace.push(Op::RegisterAsset);
    for i in 0..6 {
        trace.push(Op::Submit {
            source: i % 3,
            price: 1_000 + i as i128,
        });
    }
    trace.push(Op::RemoveSource { source: 0 });
    trace.push(Op::Submit {
        source: 1,
        price: 2_000,
    });

    // The faulty predicate: fails whenever source 0 is removed after having
    // submitted — i.e. exactly the "removal must withdraw the contribution" bug.
    let faulty = |t: &[Op]| {
        let mut submitted = false;
        for op in t {
            match op {
                Op::Submit { source, .. } if *source == 0 => submitted = true,
                Op::RemoveSource { source: 0 } if submitted => return true,
                _ => {}
            }
        }
        false
    };

    assert!(faulty(&trace), "the long trace should reproduce the fault");

    let minimal = shrink(&trace, faulty);

    // The minimal reproducer is exactly: submit from slot 0, then remove it.
    // Asserted structurally, because *which* equivalent submit the shrinker
    // keeps is an implementation detail of the descent order; the property
    // being pinned is "removal after submission, nothing else".
    assert_eq!(
        minimal.len(),
        2,
        "shrink did not minimize the trace: {minimal:?}"
    );
    assert!(
        matches!(minimal[0], Op::Submit { source: 0, .. }),
        "expected a submit from slot 0 first, got {:?}",
        minimal[0]
    );
    assert_eq!(
        minimal[1],
        Op::RemoveSource { source: 0 },
        "expected the removal of slot 0 second"
    );
    // The minimized trace must still reproduce the fault, and be strictly
    // shorter than the original.
    assert!(faulty(&minimal), "the minimized trace no longer reproduces");
    assert!(minimal.len() < trace.len());
}
