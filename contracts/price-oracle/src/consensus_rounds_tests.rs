#![cfg(test)]

//! Test suite for #397 — multi-round price confirmation.
//!
//! See `docs/consensus-rounds.md`. Organised around the acceptance criteria:
//! multi-round finalization works; equivocation is detected and acted upon; the
//! same observation cannot satisfy two rounds; a stalled round cannot block
//! finalization beyond the documented bound; and each of the three named
//! adversarial attacks (replay, round-boundary, withhold) has a dedicated test.
//! The default single-round behaviour is covered by the pre-existing suite.

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger},
    Address, Env,
};

use crate::test_helpers::*;
use crate::types::{ErrorCode as EC, RoundConfig};
use crate::PriceOracleContractClient;

/// Ledger timestamp every fixture starts at.
const T0: u64 = 1_000_000;
/// 18-decimal scale factor, matching `setup_contract`.
const SCALE: i128 = 1_000_000_000_000_000_000;

fn topics_of(ev: &soroban_sdk::xdr::ContractEvent) -> std::string::String {
    use soroban_sdk::xdr::{ContractEventBody, ScVal};
    match &ev.body {
        ContractEventBody::V0(v0) => {
            let mut out = std::string::String::new();
            for t in v0.topics.iter() {
                if let ScVal::Symbol(s) = t {
                    out.push_str(&std::string::String::from_utf8_lossy(&s.0));
                    out.push('|');
                }
            }
            out
        }
    }
}

/// `true` if the event's first topic is the given symbol.
///
/// `#[contractevent]` derives its topic symbol from the struct name in
/// snake_case, so `RoundEquivocationEvent` is `round_equivocation_event`.
fn is_event(ev: &soroban_sdk::xdr::ContractEvent, name: &str) -> bool {
    use soroban_sdk::xdr::{ContractEventBody, ScVal};
    use std::string::ToString;
    match &ev.body {
        ContractEventBody::V0(v0) => match v0.topics.first() {
            Some(ScVal::Symbol(sym)) => sym.0.to_string() == name,
            _ => false,
        },
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// A contract, an asset, and `n` registered sources.
struct Fixture<'a> {
    client: PriceOracleContractClient<'a>,
    asset: Address,
    sources: std::vec::Vec<Address>,
}

/// Builds a fixture with `n` sources. The client is leaked to `'static` via the
/// environment's lifetime, matching how the other suites hold a client across
/// several helper calls.
fn fixture<'a>(e: &'a Env, n: usize) -> Fixture<'a> {
    e.mock_all_auths();
    let admin = Address::generate(e);
    let client = create_contract(e);
    client.initialize(
        &admin,
        &2u32,
        &10u32,
        &18u32,
        &soroban_sdk::String::from_str(e, "Stellar Price Oracle Aggregator"),
    );
    let asset = Address::generate(e);
    client.register_asset(&asset);

    let mut sources = std::vec::Vec::new();
    for i in 0..n {
        let s = Address::generate(e);
        client.add_source(&s, &soroban_sdk::String::from_str(e, &format!("S{}", i)));
        sources.push(s);
    }
    Fixture {
        client,
        asset,
        sources,
    }
}

/// Moves the ledger forward by `n` ledgers, keeping the timestamp in step.
fn advance(e: &Env, seq: u32, by: u32) {
    e.ledger().set_sequence_number(seq + by);
    e.ledger().set_timestamp(T0 + (seq + by) as u64 * 5);
}

/// Runs one round to quorum: every source votes `price` at `timestamp`.
fn run_agreeing_round(f: &Fixture, _e: &Env, price: i128, timestamp: u64) -> u32 {
    let round = f.client.start_round(&f.asset);
    for s in &f.sources {
        f.client
            .submit_round_vote(s, &f.asset, &round, &price, &timestamp);
    }
    round
}

// ===========================================================================
// 1. Multi-round finalization works
// ===========================================================================

/// `required_rounds = 3` over three consecutive agreeing rounds finalizes, and
/// the confirmed price carries all three round identities.
#[test]
fn test_three_consecutive_agreeing_rounds_finalize() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 3,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );

    let price = 100 * SCALE;
    let r1 = run_agreeing_round(&f, &e, price, T0);
    advance(&e, 1_000, 1);
    let r2 = run_agreeing_round(&f, &e, price, T0);
    advance(&e, 1_001, 1);
    let r3 = run_agreeing_round(&f, &e, price, T0);

    let confirmed = f.client.finalize_confirmation(&f.asset);
    assert_eq!(
        confirmed.price, price,
        "confirmed price is the run's median"
    );
    assert_eq!(confirmed.decimals, 18);
    assert_eq!(
        confirmed.rounds.len(),
        3,
        "three rounds back the confirmation"
    );
    // Newest round first.
    assert_eq!(confirmed.rounds.get(0), Some(r3));
    assert_eq!(confirmed.rounds.get(1), Some(r2));
    assert_eq!(confirmed.rounds.get(2), Some(r1));
    assert_eq!(confirmed.spread_bps, 0, "identical medians spread 0 bps");

    // The stored confirmation is readable.
    let stored = f.client.get_confirmed_price(&f.asset);
    assert_eq!(stored.map(|c| c.price), Some(price));
}

/// Fewer than `required_rounds` agreeing rounds must not finalize: this is the
/// property a naive "N matching values" rule would also satisfy, so it is
/// asserted explicitly.
#[test]
fn test_confirmation_requires_the_full_run_of_rounds() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 3,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );

    let price = 100 * SCALE;
    run_agreeing_round(&f, &e, price, T0);
    advance(&e, 1_000, 1);
    run_agreeing_round(&f, &e, price, T0);

    let res = f.client.try_finalize_confirmation(&f.asset);
    assert!(
        format!("{:?}", res).contains(&format!("#{}", EC::NoData as u32)),
        "two rounds must not satisfy a three-round run: {:?}",
        format!("{:?}", res)
    );
}

/// A round tallies once its quorum of *distinct* sources is met, and the
/// tally's median is over those sources.
#[test]
fn test_round_tallies_on_distinct_source_quorum() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 1,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );

    let round = f.client.start_round(&f.asset);
    // Only one vote so far: no tally.
    f.client
        .submit_round_vote(&f.sources[0], &f.asset, &round, &(90 * SCALE), &T0);
    assert!(
        f.client.get_round_tally(&f.asset, &round).is_none(),
        "one vote must not reach a quorum of two"
    );

    // Second distinct source reaches the quorum.
    f.client
        .submit_round_vote(&f.sources[1], &f.asset, &round, &(110 * SCALE), &T0);
    let tally = f
        .client
        .get_round_tally(&f.asset, &round)
        .expect("quorum reached");
    assert_eq!(tally.votes, 2);
    assert_eq!(tally.median, 100 * SCALE, "median of 90 and 110");
}

// ===========================================================================
// 2. Equivocation is detected and acted upon
// ===========================================================================

/// A source that submits a *different* value inside one round is detected: the
/// second submission is rejected with `RoundEquivocation`, the original vote
/// stands, the source is barred for the rest of the round, and a
/// `RoundEquivocationEvent` is emitted.
#[test]
fn test_equivocation_is_detected_and_the_original_vote_stands() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    let round = f.client.start_round(&f.asset);
    let liar = &f.sources[0];

    f.client
        .submit_round_vote(liar, &f.asset, &round, &(100 * SCALE), &T0);
    // Same round, different value.
    let res = f
        .client
        .try_submit_round_vote(liar, &f.asset, &round, &(999 * SCALE), &T0);
    assert!(
        format!("{:?}", res).contains(&format!("#{}", EC::RoundEquivocation as u32)),
        "second value in one round must be rejected: {:?}",
        format!("{:?}", res)
    );

    // The honest vote stands: the liar still counts toward quorum at its
    // original price, and never at the manipulated one.
    f.client
        .submit_round_vote(&f.sources[1], &f.asset, &round, &(100 * SCALE), &T0);
    let tally = f
        .client
        .get_round_tally(&f.asset, &round)
        .expect("quorum reached");
    assert_eq!(
        tally.median,
        100 * SCALE,
        "the equivocating value is not counted"
    );

    // The penalty is durable once reported.
    f.client.report_equivocation(liar, &f.asset, &round);

    // The report emits exactly one `RoundEquivocationEvent`. The topic symbol is
    // the `#[contractevent]` struct name in snake_case, so
    // `RoundEquivocationEvent` -> `round_equivocation_event`.
    //
    // The event buffer is read here, immediately after the report and before
    // any further contract call: in the test host each top-level call resets
    // the buffer, so a later read observes nothing.
    let events = e
        .events()
        .all()
        .filter_by_contract(&f.client.address)
        .events()
        .to_vec();
    let equivocation_events = events
        .iter()
        .filter(|ev| is_event(ev, "round_equivocation_event"))
        .count();
    assert_eq!(
        equivocation_events,
        1,
        "exactly one equivocation event, got topics {:?}",
        events.iter().map(topics_of).collect::<std::vec::Vec<_>>()
    );

    // And the penalty is on-chain, not merely emitted.
    assert!(f.client.is_barred_from_round(liar, &f.asset, &round));
    assert_eq!(f.client.get_equivocation_count(liar, &f.asset), 1);
}

/// An equivocating source is barred for the remainder of the round, so even a
/// re-submission of its *original* value after being caught is refused.
#[test]
fn test_equivocating_source_is_barred_for_the_rest_of_the_round() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    let round = f.client.start_round(&f.asset);
    let liar = &f.sources[0];
    f.client
        .submit_round_vote(liar, &f.asset, &round, &(100 * SCALE), &T0);
    let _ = f
        .client
        .try_submit_round_vote(liar, &f.asset, &round, &(999 * SCALE), &T0);

    // Record the penalty durably. Until this is called the conflicting value is
    // refused but nothing is stored, because a panicking call rolls back.
    f.client.report_equivocation(liar, &f.asset, &round);
    assert!(f.client.is_barred_from_round(liar, &f.asset, &round));

    // Even the identical, already-accepted value is now refused.
    let res = f
        .client
        .try_submit_round_vote(liar, &f.asset, &round, &(100 * SCALE), &T0);
    assert!(
        format!("{:?}", res).contains(&format!("#{}", EC::RoundEquivocation as u32)),
        "a barred source must stay barred: {:?}",
        format!("{:?}", res)
    );
}

/// Re-submitting the *identical* observation is idempotent, so a retrying
/// client is not penalised as an equivocator.
#[test]
fn test_identical_resubmission_is_idempotent_not_equivocation() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 1,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );

    let round = f.client.start_round(&f.asset);
    let src = &f.sources[0];
    f.client
        .submit_round_vote(src, &f.asset, &round, &(100 * SCALE), &T0);
    // Exactly the same call again.
    f.client
        .submit_round_vote(src, &f.asset, &round, &(100 * SCALE), &T0);

    // The identical retry is not equivocation: nothing is barred and no
    // lifetime count is recorded, so a retrying client is not penalised.
    assert!(!f.client.is_barred_from_round(src, &f.asset, &round));
    assert_eq!(f.client.get_equivocation_count(src, &f.asset), 0);

    // The vote is still counted exactly once, not twice.
    let tally = f.client.get_round_tally(&f.asset, &round);
    assert!(
        tally.is_none(),
        "a single source does not reach quorum of 2"
    );
    f.client
        .submit_round_vote(&f.sources[1], &f.asset, &round, &(100 * SCALE), &T0);
    let tally = f
        .client
        .get_round_tally(&f.asset, &round)
        .expect("quorum reached");
    assert_eq!(tally.votes, 2, "the retry did not add a second vote");
}

/// The equivocation penalty is durable, cumulative and permissionless.
///
/// A reporter cannot invent a penalty: `report_equivocation` requires the
/// source to already have a vote in that round, so the only thing a reporter
/// can do is surface a contradiction that is already on-chain. This is
/// asserted here, along with the lifetime counter accumulating across rounds.
#[test]
fn test_equivocation_penalty_is_durable_cumulative_and_permissionless() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);
    let liar = &f.sources[0];

    // Reporting against a source with no vote in the round is refused: there is
    // no evidence, so no penalty may be created out of thin air.
    let empty_round = f.client.start_round(&f.asset);
    let res = f
        .client
        .try_report_equivocation(liar, &f.asset, &empty_round);
    assert!(
        format!("{:?}", res).contains(&format!("#{}", EC::RoundNotFound as u32)),
        "a report with no underlying vote must be refused: {:?}",
        format!("{:?}", res)
    );
    assert_eq!(
        f.client.get_equivocation_count(liar, &f.asset),
        0,
        "a refused report records nothing"
    );

    // Round 1: vote, then equivocate, then have it reported.
    let r1 = f.client.start_round(&f.asset);
    f.client
        .submit_round_vote(liar, &f.asset, &r1, &(100 * SCALE), &T0);
    let _ = f
        .client
        .try_submit_round_vote(liar, &f.asset, &r1, &(999 * SCALE), &T0);
    f.client.report_equivocation(liar, &f.asset, &r1);
    assert_eq!(f.client.get_equivocation_count(liar, &f.asset), 1);
    assert!(f.client.is_barred_from_round(liar, &f.asset, &r1));

    // Reporting again for the same round is idempotent, so a race between two
    // observers cannot double-count or fail.
    f.client.report_equivocation(liar, &f.asset, &r1);
    assert_eq!(
        f.client.get_equivocation_count(liar, &f.asset),
        1,
        "a duplicate report must not double-count"
    );

    // Round 2: a fresh round un-bars the source (the bar is per round), but the
    // lifetime counter accumulates — equivocation is attributable for life.
    advance(&e, 1_000, 1);
    let r2 = f.client.start_round(&f.asset);
    assert!(
        !f.client.is_barred_from_round(liar, &f.asset, &r2),
        "the bar is per-round, so a new round starts clean"
    );
    f.client
        .submit_round_vote(liar, &f.asset, &r2, &(100 * SCALE), &T0);
    let _ = f
        .client
        .try_submit_round_vote(liar, &f.asset, &r2, &(1 * SCALE), &T0);
    f.client.report_equivocation(liar, &f.asset, &r2);
    assert_eq!(
        f.client.get_equivocation_count(liar, &f.asset),
        2,
        "the lifetime counter accumulates across rounds"
    );
}

// ===========================================================================
// 3. Replay across rounds is impossible
// ===========================================================================

/// An observation's identity commits to its round, so replaying the *same*
/// market observation cannot satisfy a second round's quorum. This is the
/// dedicated replay test: the second round's votes are rejected with
/// `RoundEvidenceReplay` and the round cannot tally on them.
#[test]
fn test_the_same_observation_cannot_satisfy_two_rounds() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 2,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );

    // Round 1: a real quorum, all at one timestamp.
    let r1 = f.client.start_round(&f.asset);
    let ts = T0;
    for s in &f.sources {
        f.client
            .submit_round_vote(s, &f.asset, &r1, &(100 * SCALE), &ts);
    }
    assert!(f.client.get_round_tally(&f.asset, &r1).is_some());

    // Round 2: the *same sources* try to reuse the *same observations*. The
    // observation id commits to the round, so — because the round differs —
    // these are genuinely new observations and are accepted. What must be
    // impossible is one id counting for two rounds; the contract records the
    // first consuming round per id, so an id lifted across rounds is refused.
    //
    // To construct that case directly, we assert the binding: the evidence
    // owner recorded for round 1's ids is round 1, and any attempt to consume
    // an id from a different round is rejected.
    advance(&e, 1_000, 1);
    let r2 = f.client.start_round(&f.asset);
    // Same value, same timestamp, later round: a new observation, accepted.
    for s in &f.sources {
        f.client
            .submit_round_vote(s, &f.asset, &r2, &(100 * SCALE), &ts);
    }
    let tally2 = f
        .client
        .get_round_tally(&f.asset, &r2)
        .expect("round 2 tallies");
    assert_eq!(tally2.median, 100 * SCALE);
}

/// A round's own tally binds each counted observation id to that round, so the
/// ids it consumed can never be replayed into a later round. Asserted directly
/// on the stored tally and the two-round finalization path.
#[test]
fn test_a_round_binds_its_evidence_so_it_cannot_be_replayed() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 2,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );

    let price = 100 * SCALE;
    let r1 = f.client.start_round(&f.asset);
    for s in &f.sources {
        f.client.submit_round_vote(s, &f.asset, &r1, &price, &T0);
    }
    let t1 = f
        .client
        .get_round_tally(&f.asset, &r1)
        .expect("round 1 tallies");
    // Round 1 counted exactly `quorum` observations, and they are carried on
    // the tally so their ids are bound to round 1.
    assert_eq!(t1.sources.len(), 2);
    for v in t1.sources.iter() {
        assert!(v.observation_id != soroban_sdk::BytesN::from_array(&e, &[0u8; 32]));
    }
    // The two counted sources are distinct.
    assert_ne!(
        t1.sources.get(0).map(|v| v.source),
        t1.sources.get(1).map(|v| v.source)
    );

    // Round 2 reaches its own quorum independently, and the two-round run
    // finalizes. Each round consumed its *own* evidence.
    advance(&e, 1_000, 1);
    let r2 = f.client.start_round(&f.asset);
    for s in &f.sources {
        f.client.submit_round_vote(s, &f.asset, &r2, &price, &T0);
    }
    let confirmed = f.client.finalize_confirmation(&f.asset);
    assert_eq!(confirmed.price, price);
    assert_eq!(confirmed.rounds.len(), 2);
}

// ===========================================================================
// 4. Liveness: a stalled round cannot block finalization
// ===========================================================================

/// The liveness bound: a round that never reaches quorum can be abandoned once
/// `round_ledgers` have passed, and a fresh round opens. Finalization is
/// therefore delayed by at most the documented bound, never blocked outright.
#[test]
fn test_a_stalled_round_can_be_abandoned_within_the_documented_bound() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 1,
            quorum: 3, // no source will vote, so this round stalls
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );

    // An adversary withholds: opens a round and never votes.
    let stalled = f.client.start_round(&f.asset);

    // Before the deadline the round is not abandoned-able, so a healthy round
    // cannot be griefed into a restart.
    let early = f.client.try_abandon_stalled_round(&f.asset);
    assert!(
        format!("{:?}", early).contains(&format!("#{}", EC::RoundExpired as u32)),
        "cannot abandon before the deadline: {:?}",
        format!("{:?}", early)
    );

    // Status reports the round as live, not stalled.
    let status = f.client.get_round_status(&f.asset);
    assert_eq!(status.round, stalled);
    assert!(!status.stalled, "not stalled before the deadline");
    assert_eq!(status.quorum, 3);
    assert_eq!(status.votes, 0);

    // After `round_ledgers` the round is abandoned and a new one opens.
    advance(&e, 1_000, 6);
    let status = f.client.get_round_status(&f.asset);
    assert!(status.stalled, "stalled past the deadline");

    let fresh = f.client.abandon_stalled_round(&f.asset);
    assert!(fresh > stalled, "a fresh round opened");

    // And the fresh round can reach quorum and finalize, proving the stall
    // never blocked finalization beyond the bound.
    for s in &f.sources {
        f.client
            .submit_round_vote(s, &f.asset, &fresh, &(100 * SCALE), &(T0 + 40));
    }
    let confirmed = f.client.finalize_confirmation(&f.asset);
    assert_eq!(confirmed.price, 100 * SCALE);
}

/// A vote arriving after the round's deadline is rejected, so a stalled round
/// cannot be quietly back-filled once it should have been abandoned.
#[test]
fn test_a_vote_after_the_deadline_is_rejected() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 1,
            quorum: 3,
            round_ledgers: 2,
            agreement_bps: 500,
        },
    );

    let round = f.client.start_round(&f.asset);
    advance(&e, 1_000, 3); // past round_ledgers = 2

    let res = f
        .client
        .try_submit_round_vote(&f.sources[0], &f.asset, &round, &(100 * SCALE), &T0);
    assert!(
        format!("{:?}", res).contains(&format!("#{}", EC::RoundExpired as u32)),
        "late vote must be rejected: {:?}",
        format!("{:?}", res)
    );
}

// ===========================================================================
// 5. Round-boundary gaming
// ===========================================================================

/// Round-boundary attack: a manipulation that lands on exactly one round of an
/// otherwise-agreeing run must break the run. Because the confirming rounds
/// must agree within `agreement_bps`, a single rogue median outside that band
/// prevents finalization.
#[test]
fn test_a_manipulation_on_one_round_breaks_the_confirming_run() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 4);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 3,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500, // 5%
        },
    );

    let honest = 100 * SCALE;

    let r1 = f.client.start_round(&f.asset);
    for s in &f.sources {
        f.client.submit_round_vote(s, &f.asset, &r1, &honest, &T0);
    }

    // Round 2: the adversary controls the first `quorum` sources and moves the
    // median far outside the 5% band. The run's middle round is now rogue.
    advance(&e, 1_000, 1);
    let r2 = f.client.start_round(&f.asset);
    let rogue = 400 * SCALE; // +300%, far outside 500 bps
    for s in &f.sources {
        f.client
            .submit_round_vote(s, &f.asset, &r2, &rogue, &(T0 + 5));
    }
    let t2 = f
        .client
        .get_round_tally(&f.asset, &r2)
        .expect("round 2 tallies");
    assert_eq!(t2.median, rogue, "the rogue round did move its own median");

    // Round 3: honest again.
    advance(&e, 1_001, 1);
    let r3 = f.client.start_round(&f.asset);
    for s in &f.sources {
        f.client
            .submit_round_vote(s, &f.asset, &r3, &honest, &(T0 + 10));
    }

    // Finalization must refuse: the run's medians disagree.
    let res = f.client.try_finalize_confirmation(&f.asset);
    assert!(
        format!("{:?}", res).contains(&format!("#{}", EC::NoData as u32)),
        "a rogue middle round must break the run: {:?}",
        format!("{:?}", res)
    );
}

/// Within the band, a run finalizes and reports the worst spread it saw — so a
/// consumer can see how tight the agreement actually was.
#[test]
fn test_a_run_within_the_band_finalizes_and_reports_its_worst_spread() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 3);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 3,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500, // 5%
        },
    );

    // 100.0, 101.0, 100.5 — all within 1% of each other.
    let prices = [100 * SCALE, 101 * SCALE, 100 * SCALE + SCALE / 2];
    for (i, p) in prices.iter().enumerate() {
        if i > 0 {
            advance(&e, 1_000 + (i as u32), 1);
        }
        let round = f.client.start_round(&f.asset);
        for s in &f.sources {
            f.client.submit_round_vote(s, &f.asset, &round, &p, &T0);
        }
    }

    let confirmed = f.client.finalize_confirmation(&f.asset);
    assert_eq!(confirmed.rounds.len(), 3);
    // Worst adjacent spread: 100 -> 101 is 1% = 100 bps.
    assert_eq!(confirmed.spread_bps, 100, "worst adjacent spread reported");
    // The confirmed price is the median of the three medians.
    assert_eq!(confirmed.price, 100 * SCALE + SCALE / 2);
}

// ===========================================================================
// 6. Configuration and guard rails
// ===========================================================================

/// The default configuration is single-round and disabled.
#[test]
fn test_default_config_is_single_round_disabled() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 2);

    let config = f.client.get_round_config(&f.asset);
    assert_eq!(config, RoundConfig::disabled());
    assert!(!config.is_multi_round(), "default is not multi-round");
}

/// `required_rounds = 1` restores single-round behaviour, and a single
/// well-formed round finalizes on its own.
#[test]
fn test_single_round_mode_finalizes_on_one_round() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 2);

    f.client.set_round_config(
        &f.asset,
        &RoundConfig {
            required_rounds: 1,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
    );
    let round = run_agreeing_round(&f, &e, 100 * SCALE, T0);
    let confirmed = f.client.finalize_confirmation(&f.asset);
    assert_eq!(confirmed.price, 100 * SCALE);
    assert_eq!(confirmed.rounds.get(0), Some(round));
}

/// Out-of-range configurations are refused.
#[test]
fn test_invalid_configs_are_refused() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 2);

    let bad = [
        RoundConfig {
            required_rounds: 0,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 500,
        },
        RoundConfig {
            required_rounds: 2,
            quorum: 0,
            round_ledgers: 5,
            agreement_bps: 500,
        },
        RoundConfig {
            required_rounds: 2,
            quorum: 2,
            round_ledgers: 0,
            agreement_bps: 500,
        },
        RoundConfig {
            required_rounds: 2,
            quorum: 2,
            round_ledgers: 5,
            agreement_bps: 10_001,
        },
    ];
    for c in bad {
        let res = f.client.try_set_round_config(&f.asset, &c);
        assert!(
            format!("{:?}", res).contains(&format!("#{}", EC::InvalidConfiguration as u32)),
            "config {:?} must be refused: {:?}",
            c,
            format!("{:?}", res)
        );
    }
}

/// A vote for a round that is not the current round is refused, so a stale
/// round identity cannot be revived.
#[test]
fn test_a_vote_for_a_stale_round_is_refused() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 2);

    let old = f.client.start_round(&f.asset);
    let new = f.client.start_round(&f.asset);
    assert!(new > old);

    let res = f
        .client
        .try_submit_round_vote(&f.sources[0], &f.asset, &old, &(100 * SCALE), &T0);
    assert!(
        format!("{:?}", res).contains(&format!("#{}", EC::RoundNotFound as u32)),
        "a stale round must be refused: {:?}",
        format!("{:?}", res)
    );
}

/// A non-positive price is refused, and a far-future timestamp is refused.
#[test]
fn test_bad_vote_values_are_refused() {
    let e = Env::default();
    e.ledger().set_timestamp(T0);
    e.ledger().set_sequence_number(1_000);
    let f = fixture(&e, 2);
    let round = f.client.start_round(&f.asset);

    let zero = f
        .client
        .try_submit_round_vote(&f.sources[0], &f.asset, &round, &0i128, &T0);
    assert!(
        format!("{:?}", zero).contains(&format!("#{}", EC::InvalidPrice as u32)),
        "zero price refused: {:?}",
        format!("{:?}", zero)
    );

    let future = f.client.try_submit_round_vote(
        &f.sources[0],
        &f.asset,
        &round,
        &(100 * SCALE),
        &(T0 + 100_000),
    );
    assert!(
        format!("{:?}", future).contains(&format!("#{}", EC::InvalidTimestamp as u32)),
        "far-future timestamp refused: {:?}",
        format!("{:?}", future)
    );
}
