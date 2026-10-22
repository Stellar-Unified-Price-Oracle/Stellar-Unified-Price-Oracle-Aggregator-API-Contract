#![cfg(test)]
//! Tests for the adversarial hardening track:
//!
//! * **#398** pre-aggregation DQ pipeline — step/drift bounds hold against a
//!   majority submitting just inside the threshold, staged drift is caught,
//!   and every rejection is reconstructible from its event.
//! * **#400** blackout windows — half-open boundaries, quorum-only volatility
//!   trigger, bounded duration + cooldown, and pause > freeze > blackout.
//! * **#401** source comparison — collusion detected while each series stays
//!   inside its own bound, influence quantified, excluded sources flagged.
//! * **#402** source lifecycle — probation cap, atomic offboarding with zero
//!   residual influence, evidence-gated slashing, rotated-identity blocking.

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    Address, BytesN, Env, Event as _, String, Vec,
};

use crate::blackout::{
    BlackoutExitedEvent, BlackoutScheduledEvent, BlackoutWithheldEvent, COOLDOWN_SECS,
    MAX_BLACKOUT_SECS, MIN_NOTICE_SECS, ORIGIN_SCHEDULED, ORIGIN_VOLATILITY,
    VOLATILITY_BLACKOUT_SECS,
};
use crate::dq_pipeline::{
    DqConfig, DqInputRejectedEvent, REASON_BOUNDS, REASON_DRIFT, REASON_STALE, REASON_STEP,
};
use crate::source_comparison::{self, ComparisonRound};
use crate::source_lifecycle::{LifecycleConfig, SourceSlashedEvent};
use crate::test_helpers::*;
use crate::types::{DataKey, ErrorCode};
use crate::PriceOracleContractClient;

const P: i128 = 1_000_000;

fn err(code: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(code as u32)
}

struct Ctx<'a> {
    e: &'a Env,
    client: PriceOracleContractClient<'a>,
    asset: Address,
    seq: u32,
    ts: u64,
}

impl<'a> Ctx<'a> {
    fn new(e: &'a Env) -> Self {
        ledger_default(e, 1, 100_000);
        let (client, _admin) = setup_contract(e);
        client.set_min_sources_required(&1u32);
        let asset = register_test_asset(e, &client);
        Ctx {
            e,
            client,
            asset,
            seq: 1,
            ts: 100_000,
        }
    }

    fn sources(&self, n: u32) -> Vec<Address> {
        let mut v = Vec::new(self.e);
        for _ in 0..n {
            v.push_back(register_test_source(self.e, &self.client, "S"));
        }
        v
    }

    /// Moves the ledger forward by `secs`.
    fn advance(&mut self, secs: u64) {
        self.seq += 10;
        self.ts += secs;
        ledger_default(self.e, self.seq, self.ts);
    }

    fn submit(&self, src: &Address, price: i128) {
        self.client.submit_price(src, &self.asset, &price, &self.ts);
    }

    /// One round: advance a minute, then every source submits its price.
    fn round(&mut self, srcs: &Vec<Address>, prices: &[i128]) {
        self.advance(60);
        for (i, p) in prices.iter().enumerate() {
            self.submit(&srcs.get_unchecked(i as u32), *p);
        }
    }

    fn price(&self) -> i128 {
        self.client.get_price(&self.asset, &0u64).unwrap().price
    }

    fn emitted(&self, xdr: soroban_sdk::xdr::ContractEvent) -> bool {
        self.e
            .events()
            .all()
            .filter_by_contract(&self.client.address)
            .events()
            .contains(&xdr)
    }
}

fn dq(step: u32, drift: u32) -> DqConfig {
    DqConfig {
        max_staleness_secs: 0,
        min_price: 1,
        max_price: 0,
        max_step_bps: step,
        max_drift_bps: drift,
        drift_window_secs: 86_400,
    }
}

// ── #398 DQ pipeline ────────────────────────────────────────────────────────

/// A majority (here: every source) submitting exactly at the step threshold is
/// accepted, but cannot move the aggregate beyond `last * (1 + step)`; one unit
/// past the threshold is rejected with a reconstructible event.
#[test]
fn dq_step_bound_holds_against_majority_just_inside() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(3);
    c.round(&s, &[P, P, P]);
    c.client.set_dq_config(&c.asset, &dq(500, 0));

    let inside = P + P * 500 / 10_000;
    c.round(&s, &[inside, inside, inside]);
    assert_eq!(c.price(), inside);

    let outside = inside + inside * 500 / 10_000 + 1;
    c.round(&s, &[outside, outside, outside]);
    assert_eq!(
        c.price(),
        inside,
        "no round may move the aggregate past the step bound"
    );
    assert!(c.emitted(
        DqInputRejectedEvent {
            asset: c.asset.clone(),
            source: s.get_unchecked(2),
            reason: REASON_STEP,
            value: outside,
            threshold: 500,
            reference: inside,
        }
        .to_xdr(&e, &c.client.address)
    ));
}

/// Staged drift: +4 % per round stays inside the 5 % step check every time,
/// but the cumulative 10 % drift bound stops it within the window.
#[test]
fn dq_detects_slow_staged_drift() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(3);
    c.round(&s, &[P, P, P]);
    c.client.set_dq_config(&c.asset, &dq(500, 1_000));

    let mut target = P;
    let mut rejected = false;
    for _ in 0..6 {
        target += target * 400 / 10_000;
        c.round(&s, &[target, target, target]);
        assert!(
            c.price() <= P + P / 10,
            "cumulative drift must stay within 10 %"
        );
        rejected |= c.emitted(
            DqInputRejectedEvent {
                asset: c.asset.clone(),
                source: s.get_unchecked(2),
                reason: REASON_DRIFT,
                value: target,
                threshold: 1_000,
                reference: P,
            }
            .to_xdr(&e, &c.client.address),
        );
    }
    assert!(rejected, "staged drift must be rejected by the drift check");
}

/// Bounds and staleness rejections carry the threshold and reference applied.
#[test]
fn dq_bounds_and_staleness_are_evented() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(2);
    c.round(&s, &[P, P]);
    c.client.set_dq_config(
        &c.asset,
        &DqConfig {
            max_staleness_secs: 100,
            min_price: P / 2,
            max_price: 2 * P,
            max_step_bps: 0,
            max_drift_bps: 0,
            drift_window_secs: 0,
        },
    );

    c.advance(60);
    c.submit(&s.get_unchecked(0), 3 * P);
    assert!(c.emitted(
        DqInputRejectedEvent {
            asset: c.asset.clone(),
            source: s.get_unchecked(0),
            reason: REASON_BOUNDS,
            value: 3 * P,
            threshold: 2 * P,
            reference: 2 * P,
        }
        .to_xdr(&e, &c.client.address)
    ));

    // Source 1 last submitted 60 s ago; 60 + 50 > 100 makes it stale.
    let submitted_at = c.ts - 60;
    c.advance(50);
    c.submit(&s.get_unchecked(0), P);
    assert!(c.emitted(
        DqInputRejectedEvent {
            asset: c.asset.clone(),
            source: s.get_unchecked(1),
            reason: REASON_STALE,
            value: 110,
            threshold: 100,
            reference: submitted_at as i128,
        }
        .to_xdr(&e, &c.client.address)
    ));
}

// ── #400 blackout windows ───────────────────────────────────────────────────

/// `[start, end)` is enforced exactly: publication at `start - 1` and `end`,
/// withheld at `start` and `end - 1`; the in-window submission is used after.
#[test]
fn blackout_boundaries_are_half_open() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(1);
    let src = s.get_unchecked(0);
    c.round(&s, &[P]);

    let start = c.ts + MIN_NOTICE_SECS;
    let end = start + 1_000;
    c.client.schedule_blackout(&c.asset, &start, &end);
    assert!(c.emitted(
        BlackoutScheduledEvent {
            asset: c.asset.clone(),
            start,
            end,
            origin: ORIGIN_SCHEDULED,
        }
        .to_xdr(&e, &c.client.address)
    ));

    c.advance(start - 1 - c.ts);
    c.submit(&src, P + 1);
    assert_eq!(c.price(), P + 1);

    c.advance(1);
    c.submit(&src, P + 2);
    assert_eq!(c.price(), P + 1, "withheld at start");
    assert!(c.emitted(
        BlackoutWithheldEvent {
            asset: c.asset.clone(),
            at: start,
            window_end: end,
        }
        .to_xdr(&e, &c.client.address)
    ));

    c.advance(end - 1 - c.ts);
    c.submit(&src, P + 3);
    assert_eq!(c.price(), P + 1, "withheld at end - 1");

    c.advance(1);
    c.submit(&src, P + 4);
    assert_eq!(c.price(), P + 4, "published at end");
    assert!(c.emitted(
        BlackoutExitedEvent {
            asset: c.asset.clone(),
            start,
            end,
            cancelled: false,
        }
        .to_xdr(&e, &c.client.address)
    ));
    assert_eq!(c.client.get_blackout(&c.asset), None);
}

/// One source signalling repeatedly never opens a window; quorum distinct
/// sources do, and further signals do not extend it.
#[test]
fn single_source_cannot_trigger_or_extend_blackout() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(3);
    c.round(&s, &[P, P, P]);

    for _ in 0..5 {
        assert!(!c.client.signal_volatility(&s.get_unchecked(0), &c.asset));
    }
    assert_eq!(c.client.get_blackout(&c.asset), None);

    assert!(!c.client.signal_volatility(&s.get_unchecked(1), &c.asset));
    assert!(c.client.signal_volatility(&s.get_unchecked(2), &c.asset));
    let w = c.client.get_blackout(&c.asset).unwrap();
    assert_eq!(w.origin, ORIGIN_VOLATILITY);
    assert_eq!(w.end - w.start, VOLATILITY_BLACKOUT_SECS);

    c.advance(10);
    assert!(!c.client.signal_volatility(&s.get_unchecked(0), &c.asset));
    assert_eq!(c.client.get_blackout(&c.asset).unwrap().end, w.end);
}

/// Duration (with extensions) is capped and consecutive windows need a
/// cooldown, so continuous blackout is impossible.
#[test]
fn blackout_griefing_is_bounded() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(3);
    c.round(&s, &[P, P, P]);

    let start = c.ts + MIN_NOTICE_SECS;
    assert!(c
        .client
        .try_schedule_blackout(&c.asset, &start, &(start + MAX_BLACKOUT_SECS + 1))
        .is_err());
    assert!(
        c.client
            .try_schedule_blackout(&c.asset, &(c.ts + 10), &(c.ts + 100))
            .is_err(),
        "minimum notice is required"
    );

    c.client.schedule_blackout(&c.asset, &start, &(start + 100));
    assert!(c
        .client
        .try_extend_blackout(&c.asset, &(start + MAX_BLACKOUT_SECS + 1))
        .is_err());
    c.client
        .extend_blackout(&c.asset, &(start + MAX_BLACKOUT_SECS));

    // After the window, a volatility quorum inside the cooldown cannot reopen.
    c.advance(MIN_NOTICE_SECS + MAX_BLACKOUT_SECS + 1);
    c.submit(&s.get_unchecked(0), P);
    c.client.signal_volatility(&s.get_unchecked(0), &c.asset);
    c.client.signal_volatility(&s.get_unchecked(1), &c.asset);
    assert!(c
        .client
        .try_signal_volatility(&s.get_unchecked(2), &c.asset)
        .is_err());

    c.advance(COOLDOWN_SECS);
    c.client.signal_volatility(&s.get_unchecked(0), &c.asset);
    c.client.signal_volatility(&s.get_unchecked(1), &c.asset);
    assert!(c.client.signal_volatility(&s.get_unchecked(2), &c.asset));
}

/// pause > freeze > blackout, for every overlap.
#[test]
fn blackout_precedence_with_pause_and_freeze() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(3);
    c.round(&s, &[P, P, P]);
    let src = s.get_unchecked(0);
    for i in 0..3 {
        c.client.signal_volatility(&s.get_unchecked(i), &c.asset);
    }

    // blackout + pause: pause wins, the submission itself is rejected.
    c.client.pause();
    c.advance(10);
    assert_eq!(
        c.client.try_submit_price(&src, &c.asset, &(2 * P), &c.ts),
        Err(Ok(err(ErrorCode::ContractPaused)))
    );
    c.client.unpause();

    // blackout + freeze: the frozen snapshot is served, submissions rejected.
    c.client
        .freeze_price(&c.asset, &String::from_str(&e, "incident"));
    assert!(c
        .client
        .try_submit_price(&src, &c.asset, &(2 * P), &c.ts)
        .is_err());
    assert_eq!(c.price(), P);
    c.client.unfreeze_price(&c.asset);

    // blackout alone: submission stored, aggregation withheld.
    c.submit(&src, 2 * P);
    assert_eq!(c.price(), P);
}

// ── #401 source comparison ──────────────────────────────────────────────────

/// Two colluders lean +40 bps every round — well inside a 100 bps deviation
/// bound — while honest sources deviate with uncorrelated signs. Only the
/// colluding pair is flagged.
#[test]
fn collusion_signal_detects_coordinated_deviation_within_bounds() {
    let e = Env::default();
    let srcs: std::vec::Vec<Address> = (0..5).map(|_| Address::generate(&e)).collect();
    // Honest signs follow mutually orthogonal Walsh patterns (+-+-, ++--,
    // ++++----), so no honest pair co-moves.
    let sign = |i: usize, r: usize| if (r >> i) & 1 == 0 { 1i128 } else { -1 };
    let mut rounds = Vec::new(&e);
    for r in 0..8 {
        let mut sources = Vec::new(&e);
        let mut prices = Vec::new(&e);
        let mut counted = Vec::new(&e);
        for (i, s) in srcs.iter().enumerate() {
            let dev_bps = if i < 3 { 20 * sign(i, r) } else { 40 };
            assert!(
                dev_bps.abs() <= 100,
                "each series stays within its own bound"
            );
            sources.push_back(s.clone());
            prices.push_back(P + P * dev_bps / 10_000);
            counted.push_back(true);
        }
        rounds.push_back(ComparisonRound {
            aggregate: P,
            sources,
            prices,
            counted,
        });
    }
    let report = source_comparison::analyze(&e, &rounds);
    assert_eq!(report.collusion.len(), 1);
    let pair = report.collusion.get_unchecked(0);
    assert_eq!((pair.a, pair.b), (srcs[3].clone(), srcs[4].clone()));
    assert!(pair.similarity_bps >= 9_900);
}

/// End-to-end: a source whose value is always screened out is admitted but
/// never counted and is flagged; counted sources get a quantified influence.
#[test]
fn excluded_source_is_flagged_and_influence_quantified() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(4);
    c.round(&s, &[P, P + 1_000, P + 2_000, P]);
    c.client.set_source_comparison(&c.asset, &true);
    c.client.set_dq_config(
        &c.asset,
        &DqConfig {
            max_price: 2 * P,
            ..dq(0, 0)
        },
    );
    for _ in 0..3 {
        c.round(&s, &[P, P + 1_000, P + 2_000, 5 * P]);
    }

    let report = c.client.get_source_comparison(&c.asset);
    let excluded = s.get_unchecked(3);
    for st in report.sources.iter() {
        if st.source == excluded {
            assert!(st.excluded);
            assert_eq!(st.rounds_counted, 0);
        } else {
            assert!(!st.excluded);
            assert!(st.rounds_counted > 0);
        }
    }
    let infl = |a: &Address| {
        report
            .sources
            .iter()
            .find(|st| st.source == *a)
            .unwrap()
            .influence_bps
    };
    assert!(infl(&s.get_unchecked(0)) > 0);
    assert_eq!(infl(&excluded), 0);
}

// ── #402 source lifecycle ───────────────────────────────────────────────────

fn identity(e: &Env, n: u8) -> BytesN<32> {
    BytesN::from_array(e, &[n; 32])
}

/// Three probation sources agreeing on a hostile value contribute at most one
/// counted value (`PROBATION_MAX_COUNTED = 1`), so they cannot own quorum.
#[test]
fn probation_caps_quorum_influence() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let honest = c.sources(1);
    let mut all = honest.clone();
    for i in 0..3u8 {
        let p = Address::generate(&e);
        c.client
            .onboard_source(&p, &String::from_str(&e, "P"), &identity(&e, i));
        all.push_back(p);
    }
    c.round(&all, &[P, 5 * P, 5 * P, 5 * P]);
    let agg = c.client.get_price(&c.asset, &0u64).unwrap();
    assert_eq!(agg.num_sources, 2);
    assert_eq!(agg.price, 3 * P);
}

fn bonded(c: &Ctx<'_>, srcs: &Vec<Address>) {
    let token = deploy_token(c.e);
    c.client.set_stake_token_contract(&token);
    c.client.set_source_bond(&100i128);
    for s in srcs.iter() {
        mint_token(c.e, &token, &s, 100);
        c.client.deposit_source_bond(&s);
    }
}

/// Offboarding with evidence slashes, revokes every derived record, and the
/// removed source has zero influence on every subsequent aggregate.
#[test]
fn offboarding_is_atomic_and_leaves_zero_influence() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let mut s = c.sources(3);
    let m = Address::generate(&e);
    c.client
        .onboard_source(&m, &String::from_str(&e, "M"), &identity(&e, 9));
    s.push_back(m.clone());
    bonded(&c, &s);
    c.client.set_lifecycle_config(&LifecycleConfig {
        probation_secs: 0,
        slash_deviation_bps: 2_000,
    });
    c.client.graduate_source(&m);
    c.round(&s, &[P, P + 10, P + 20, 5 * P]);

    c.client.offboard_source(&m, &Some(c.asset.clone()));
    assert!(c.emitted(
        SourceSlashedEvent {
            source: m.clone(),
            evidence_asset: c.asset.clone(),
            submitted_price: 5 * P,
            aggregate_price: P + 15,
            deviation_bps: 39_998,
            threshold_bps: 2_000,
            bond_forfeited: 100,
        }
        .to_xdr(&e, &c.client.address)
    ));
    assert_eq!(c.price(), P + 10, "recomputed without the removed source");

    // Fails if any derived state survives.
    e.as_contract(&c.client.address, || {
        let st = e.storage().persistent();
        assert!(!st.has(&DataKey::Submission(c.asset.clone(), m.clone())));
        assert!(!st.has(&DataKey::SubmissionLedger(c.asset.clone(), m.clone())));
        assert!(!st.has(&DataKey::LastSubmissionLedger(m.clone(), c.asset.clone())));
        assert_eq!(
            st.get::<_, i128>(&DataKey::SourceReputation(m.clone())),
            Some(0)
        );
        assert!(crate::source_lifecycle::get_record(&e, &m).is_none());
    });
    assert!(!c.client.is_source(&m));
    assert_eq!(c.client.get_source_deposited_bond(&m), 0);
    assert!(c
        .client
        .try_submit_price(&m, &c.asset, &(5 * P), &c.ts)
        .is_err());

    let honest = Vec::from_array(
        &e,
        [s.get_unchecked(0), s.get_unchecked(1), s.get_unchecked(2)],
    );
    c.round(&honest, &[P, P + 10, P + 20]);
    assert_eq!(c.price(), P + 10);
}

/// Without on-chain evidence of malfeasance, slashing is refused and nothing
/// is revoked.
#[test]
fn slashing_requires_evidence() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let s = c.sources(3);
    c.round(&s, &[P, P + 10, P + 20]);
    let victim = s.get_unchecked(2);
    assert_eq!(
        c.client
            .try_offboard_source(&victim, &Some(c.asset.clone())),
        Err(Ok(err(ErrorCode::InvalidConfiguration)))
    );
    assert!(c.client.is_source(&victim));
}

/// Neither the offboarded address nor its identity fingerprint can come back,
/// through onboarding, plain registration or key rotation.
#[test]
fn rotated_identity_cannot_launder_removal() {
    let e = Env::default();
    let c = Ctx::new(&e);
    let s = c.sources(1);
    let m = Address::generate(&e);
    let id = identity(&e, 7);
    c.client.onboard_source(&m, &String::from_str(&e, "M"), &id);
    c.client.offboard_source(&m, &None);

    let fresh = Address::generate(&e);
    let revoked = Err(Ok(err(ErrorCode::IdentityRevoked)));
    assert_eq!(
        c.client
            .try_onboard_source(&fresh, &String::from_str(&e, "M2"), &id),
        revoked
    );
    assert_eq!(
        c.client.try_add_source(&m, &String::from_str(&e, "M")),
        revoked
    );
    assert_eq!(
        c.client.try_rotate_source_key(&s.get_unchecked(0), &m),
        revoked
    );
}

/// Graduation requires both the elapsed probation period and a posted bond.
#[test]
fn onboarding_checklist_gates_graduation() {
    let e = Env::default();
    let mut c = Ctx::new(&e);
    let m = Address::generate(&e);
    c.client
        .onboard_source(&m, &String::from_str(&e, "M"), &identity(&e, 1));
    let list = c.client.get_onboarding_checklist(&m);
    assert!(list.identity_verified && !list.probation_complete && !list.graduated);
    assert!(c.client.try_graduate_source(&m).is_err());

    bonded(&c, &Vec::from_array(&e, [m.clone()]));
    c.advance(crate::source_lifecycle::DEFAULT_PROBATION_SECS);
    let list = c.client.get_onboarding_checklist(&m);
    assert!(list.bond_posted && list.probation_complete);
    c.client.graduate_source(&m);
    assert!(c.client.get_onboarding_checklist(&m).graduated);
}
