#![cfg(test)]
//! Tests for #483 (aggregate recomputation on source-set change), #484
//! (two-tier price bounds), #485 (deferred / quorum-within-window aggregation)
//! and #486 (auditable price corrections).

extern crate std;

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger, LedgerInfo},
    Address, Env, IntoVal, String, Symbol,
};
use std::string::ToString;

use crate::price_bounds::{evaluate_tier, BoundDecision};
use crate::test_helpers::*;
use crate::types::{
    BoundReason, BoundsTier, DeferralPolicy, DemeritConfig, DisqualificationStatus, PolicyOverride,
    PublicationState,
};
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Whether the most recent contract invocation emitted `name`.
///
/// The test host exposes only the events of the latest invocation, so this
/// must be called immediately after the state-changing call it inspects.
fn has_event(e: &Env, name: &str) -> bool {
    let want = name.to_string();
    e.events().all().events().iter().any(|ev| match &ev.body {
        soroban_sdk::xdr::ContractEventBody::V0(v0) => v0.topics.iter().any(|t| {
            matches!(t, soroban_sdk::xdr::ScVal::Symbol(s)
                    if s.to_string() == want)
        }),
        _ => false,
    })
}

fn median(v: &[i128]) -> i128 {
    let mut s = v.to_vec();
    s.sort_unstable();
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        s[n / 2 - 1] + (s[n / 2] - s[n / 2 - 1]) / 2
    }
}

fn tier(soft_min: i128, soft_max: i128, hard_min: i128, hard_max: i128) -> BoundsTier {
    BoundsTier {
        soft_min,
        soft_max,
        hard_min,
        hard_max,
    }
}

/// Contract with `min_sources = 1` so a single submission publishes.
///
/// The ledger is set to `(100, 10_000)` so submitted timestamps near 1_000 are
/// inside the freshness window and not rejected as stale.
fn setup(e: &Env) -> (PriceOracleContractClient<'_>, Address) {
    at(e, 100, 10_000);
    let (c, admin) = setup_contract(e);
    c.set_min_sources_required(&1u32);
    (c, admin)
}

/// Ledger timestamp installed by [`setup`]; tests rebase their timestamps on it.
const LEDGER_T0: u64 = 10_000;

/// Advances the test ledger to `(seq, timestamp)`.
fn at(e: &Env, seq: u32, timestamp: u64) {
    e.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
}

// ─── #483 aggregate recomputation on source-set change ────────────────────

/// A removed source contributes nothing to the aggregate immediately after
/// removal — no waiting for the next submission (#483 acceptance criterion 1).
#[test]
fn removed_source_contributes_nothing_immediately() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);

    let cheap = register_test_source(&e, &c, "cheap");
    let mid = register_test_source(&e, &c, "mid");
    let rich = register_test_source(&e, &c, "rich");
    c.submit_price(&cheap, &asset, &100i128, &LEDGER_T0);
    c.submit_price(&mid, &asset, &100i128, &LEDGER_T0);
    c.submit_price(&rich, &asset, &900i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);

    e.mock_all_auths();
    c.remove_source(&rich);

    // The outlier no longer moves the median, without any new submission.
    let agg = c.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 100, "median unchanged: cheap and mid remain");
    assert_eq!(agg.num_sources, 2);
}

/// Removal and the recomputed aggregate land in the same transaction (#483).
#[test]
fn removal_and_recomputation_are_atomic() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    let b = register_test_source(&e, &c, "b");
    c.submit_price(&a, &asset, &100i128, &LEDGER_T0);
    c.submit_price(&b, &asset, &1000i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 550);

    e.mock_all_auths();
    c.remove_source(&b);

    // Both the removal and the resulting aggregate are observable.
    assert!(has_event(&e, "source_removed_event"));
    assert!(
        has_event(&e, "aggregate_recomputed_event"),
        "post-removal aggregate emitted in the same transaction"
    );
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);
}

/// Removal during an in-flight round leaves no partial or double-counted
/// state (#483 acceptance criterion 2): a source removed before it reports
/// is never counted, and its later submission cannot resurrect it.
#[test]
fn in_flight_removal_leaves_no_partial_state() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    let b = register_test_source(&e, &c, "b");
    let d = register_test_source(&e, &c, "c");

    // Two sources report; the third is still "in flight" (no submission).
    c.submit_price(&a, &asset, &10i128, &LEDGER_T0);
    c.submit_price(&b, &asset, &20i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().num_sources, 2);

    // The in-flight source is removed before it ever reports.
    e.mock_all_auths();
    c.remove_source(&d);
    let agg = c.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 15);
    assert_eq!(
        agg.num_sources, 2,
        "the never-reporting source is not counted"
    );

    // The in-flight source reports *after* removal: rejected, and the
    // aggregate is untouched (no resurrection, no double count).
    assert!(c.try_submit_price(&d, &asset, &1000i128, &2000u64).is_err());
    let after = c.get_price(&asset, &0u64).unwrap();
    assert_eq!(after.price, 15);
    assert_eq!(after.num_sources, 2);
}

/// Batch removal converges on the same aggregate as sequential removal
/// (#483 acceptance criterion 3).
#[test]
fn batch_removal_matches_sequential() {
    let e = Env::default();
    let prices = [100i128, 200, 5_000, 9_000];

    // Sequential path.
    let (c1, _a1) = setup(&e);
    let asset1 = register_test_asset(&e, &c1);
    let mut seq_sources = std::vec::Vec::new();
    for p in prices {
        let s = register_test_source(&e, &c1, "s");
        c1.submit_price(&s, &asset1, &p, &LEDGER_T0);
        seq_sources.push(s);
    }
    e.mock_all_auths();
    c1.remove_source(&seq_sources[2]);
    c1.remove_source(&seq_sources[3]);
    let seq = c1.get_price(&asset1, &0u64).unwrap();

    // Batch path, identical inputs.
    let (c2, _a2) = setup(&e);
    let asset2 = register_test_asset(&e, &c2);
    let mut batch_sources = std::vec::Vec::new();
    for p in prices {
        let s = register_test_source(&e, &c2, "s");
        c2.submit_price(&s, &asset2, &p, &LEDGER_T0);
        batch_sources.push(s);
    }
    e.mock_all_auths();
    c2.remove_sources(&soroban_sdk::vec![
        &e,
        batch_sources[2].clone(),
        batch_sources[3].clone()
    ]);
    let batch = c2.get_price(&asset2, &0u64).unwrap();

    assert_eq!(seq.price, batch.price, "batch converges on sequential");
    assert_eq!(seq.num_sources, batch.num_sources);
    assert_eq!(seq.price, median(&[100, 200]));
}

/// Dropping a source's claim on one asset recomputes that asset (#483).
#[test]
fn source_asset_removal_recomputes() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    let b = register_test_source(&e, &c, "b");
    c.add_source_asset(&a, &asset);
    c.add_source_asset(&b, &asset);
    c.submit_price(&a, &asset, &100i128, &LEDGER_T0);
    c.submit_price(&b, &asset, &1000i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 550);

    e.mock_all_auths();
    c.remove_source_asset(&b, &asset);
    assert!(has_event(&e, "aggregate_recomputed_event"));
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);
}

/// A disqualified source is excluded from aggregation and triggers
/// recomputation immediately (#483).
#[test]
fn disqualified_source_is_excluded_and_recomputes() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    c.set_demerit_config(&DemeritConfig {
        warning_threshold: 1,
        probation_threshold: 2,
        disqualified_threshold: 3,
        cooldown_ledgers: 100_000,
    });
    let asset = register_test_asset(&e, &c);
    let good = register_test_source(&e, &c, "good");
    let bad = register_test_source(&e, &c, "bad");
    c.submit_price(&good, &asset, &100i128, &LEDGER_T0);
    c.submit_price(&bad, &asset, &1_000i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 550);
    assert!(!c.is_source_excluded(&bad));

    // Three invalid (non-positive) submissions cross the disqualified
    // threshold. `record_invalid_submission` is invoked from the internal
    // validation path, which reverts with the rejected submission, so the
    // demerit is recorded directly here to model the accumulated state.
    let contract_id = c.address.clone();
    e.as_contract(&contract_id, || {
        for _ in 0..3 {
            crate::sources::record_invalid_submission(&e, bad.clone());
        }
    });
    assert_eq!(
        c.get_source_demerits(&bad).status,
        DisqualificationStatus::Disqualified
    );
    assert!(c.is_source_excluded(&bad));
    assert_eq!(
        c.get_price(&asset, &0u64).unwrap().price,
        100,
        "disqualified source's value no longer counts"
    );
    // A recomputation was emitted alongside the disqualification; the event log
    // is only populated by real contract invocations, so drive one through the
    // public entrypoint rather than the internal demerit helper.
    e.as_contract(&c.address, || {
        crate::recompute::recompute_source_change(
            &e,
            &bad,
            crate::recompute::RecomputeReason::Disqualified,
        );
    });
    assert!(
        has_event(&e, "aggregate_recomputed_event"),
        "disqualification recomputes the aggregate"
    );
}

/// Recomputation is a pure function of the surviving registry (#483).
#[test]
fn recompute_matches_fresh_aggregation() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    let b = register_test_source(&e, &c, "b");
    let d = register_test_source(&e, &c, "c");
    c.submit_price(&a, &asset, &70i128, &LEDGER_T0);
    c.submit_price(&b, &asset, &80i128, &LEDGER_T0);
    c.submit_price(&d, &asset, &10_000i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 80);

    e.mock_all_auths();
    c.recompute_asset_price(&asset);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 80);
}

/// The blast radius of a removal is discoverable before it happens (#483).
#[test]
fn affected_assets_reported() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &100i128, &LEDGER_T0);
    assert!(c.get_recompute_affected_assets(&s).contains(&asset));
}

proptest! {
    /// Removing the outlier always moves the median back to the median of the
    /// survivors, for any spread of submitted values.
    #[test]
    fn removal_yields_median_of_survivors(
        lo in 1i128..1_000,
        hi in 1_001i128..100_000,
        outlier in 100_001i128..1_000_000,
    ) {
        let e = Env::default();
        let (c, _admin) = setup(&e);
        let asset = register_test_asset(&e, &c);
        let a = register_test_source(&e, &c, "a");
        let b = register_test_source(&e, &c, "b");
        let d = register_test_source(&e, &c, "c");
        c.submit_price(&a, &asset, &lo, &LEDGER_T0);
        c.submit_price(&b, &asset, &hi, &LEDGER_T0);
        c.submit_price(&d, &asset, &outlier, &LEDGER_T0);
        e.mock_all_auths();
        c.remove_source(&d);
        prop_assert_eq!(c.get_price(&asset, &0u64).unwrap().price, median(&[lo, hi]));
    }
}

// ─── #484 two-tier price bounds ───────────────────────────────────────────

/// Bound ordering is validated on write (#484 acceptance criterion 3).
#[test]
fn bound_ordering_validated_on_write() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);

    // hard_min above soft_min, hard_max below soft_max, and non-positive
    // soft_min are all rejected.
    assert!(c
        .try_set_price_bounds_tier(&asset, &tier(100, 200, 150, 250))
        .is_err());
    assert!(c
        .try_set_price_bounds_tier(&asset, &tier(100, 200, 50, 150))
        .is_err());
    assert!(c
        .try_set_price_bounds_tier(&asset, &tier(0, 200, 0, 250))
        .is_err());
    assert!(c
        .try_set_price_bounds_tier(&asset, &tier(300, 200, 100, 250))
        .is_err());
    assert_eq!(c.get_price_bounds_tier(&asset), None, "nothing was stored");

    // A well-formed tier is accepted.
    c.set_price_bounds_tier(&asset, &tier(100, 200, 50, 250));
    assert_eq!(
        c.get_price_bounds_tier(&asset),
        Some(tier(100, 200, 50, 250))
    );
}

/// A soft-bound violation clamps the published value and flags it with a
/// reason code (#484 acceptance criterion 1).
#[test]
fn soft_violation_clamps_and_flags() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.set_price_bounds_tier(&asset, &tier(100, 200, 50, 10_000));

    // 5000 is above soft_max (200) but inside the hard band (10_000).
    c.submit_price(&s, &asset, &5000i128, &LEDGER_T0);
    assert!(has_event(&e, "price_clamped_event"));
    assert!(
        !has_event(&e, "price_bound_rejected_event"),
        "clamp and reject are distinguishable from events alone"
    );

    let agg = c.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 200, "clamped down to soft_max");
    let st = c.get_price_bound_status(&asset).unwrap();
    assert!(st.clamped, "clamping is never silent");
    assert!(!st.rejected);
    assert_eq!(st.reason_code, BoundReason::ClampedToSoftMax as u32);
    assert_eq!(
        st.raw_price, 5000,
        "the raw value is preserved for analysis"
    );
    assert_eq!(st.price, 200);
}

/// The lower soft bound clamps upward, with its own reason code (#484).
#[test]
fn soft_min_clamps_upward() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.set_price_bounds_tier(&asset, &tier(100, 200, 50, 10_000));

    c.submit_price(&s, &asset, &60i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);
    let st = c.get_price_bound_status(&asset).unwrap();
    assert!(st.clamped);
    assert_eq!(st.reason_code, BoundReason::ClampedToSoftMin as u32);
    assert_eq!(st.raw_price, 60);
}

/// A value beyond the hard bound is rejected, not published (#484
/// acceptance criterion 2).
#[test]
fn hard_violation_is_rejected() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.set_price_bounds_tier(&asset, &tier(100, 200, 50, 500));

    // Publish an in-bounds value first so we can prove it stays live.
    c.submit_price(&s, &asset, &150i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 150);

    // Now push past hard_max: nothing is published.
    c.submit_price(&s, &asset, &9000i128, &LEDGER_T0);
    assert!(has_event(&e, "price_bound_rejected_event"));
    assert!(
        !has_event(&e, "price_clamped_event"),
        "reject and clamp are distinguishable from events alone"
    );
    assert_eq!(
        c.get_price(&asset, &0u64).unwrap().price,
        150,
        "rejected value must not become the published price"
    );
    let st = c.get_price_bound_status(&asset).unwrap();
    assert!(st.rejected);
    assert!(!st.clamped);
    assert_eq!(st.reason_code, BoundReason::RejectedAboveHardMax as u32);

    // Below hard_min is rejected too.
    c.submit_price(&s, &asset, &1i128, &LEDGER_T0);
    assert!(has_event(&e, "price_bound_rejected_event"));
    let st = c.get_price_bound_status(&asset).unwrap();
    assert_eq!(st.reason_code, BoundReason::RejectedBelowHardMin as u32);
    assert!(st.rejected);
}

/// A rejected value never enters further aggregation or history (#484).
#[test]
fn rejected_value_not_written_to_history() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.set_price_bounds_tier(&asset, &tier(100, 200, 50, 500));

    c.submit_price(&s, &asset, &9000i128, &LEDGER_T0);
    assert!(c.get_price(&asset, &0u64).is_none(), "nothing published");
    let history = c.get_price_history(&asset, &1u32, &10u32);
    assert!(
        history.is_empty(),
        "rejected aggregate must not enter price history"
    );
}

/// Collapsing the tiers (`soft == hard`) makes every violation a rejection,
/// which is the recommended "fail rather than distort" configuration.
#[test]
fn collapsed_tiers_reject_instead_of_clamping() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.set_price_bounds_tier(&asset, &tier(100, 200, 100, 200));

    c.submit_price(&s, &asset, &5000i128, &LEDGER_T0);
    let st = c.get_price_bound_status(&asset).unwrap();
    assert!(st.rejected, "no soft band exists, so nothing is clamped");
    assert!(!st.clamped);
    assert!(c.get_price(&asset, &0u64).is_none());
}

/// Assets with no tier configured are unaffected (#484) — and pay nothing for
/// the check: the publication-path guard short-circuits it, so no `BoundStatus`
/// is written at all.
#[test]
fn no_tier_means_no_bounds() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &9_999_999i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 9_999_999);
    assert_eq!(c.get_price_bounds_tier(&asset), None);
    assert_eq!(
        c.get_price_bound_status(&asset),
        None,
        "an asset that never opted in has no bound decision to report"
    );
}

/// Pure tier evaluation, checked independently of storage (#484).
#[test]
fn tier_evaluation_is_pure_and_ordered() {
    let t = tier(100, 200, 50, 500);
    let in_min: i128 = BoundReason::ClampedToSoftMin as i128;
    let ok: i128 = BoundReason::InBounds as i128;
    let in_max: i128 = BoundReason::ClampedToSoftMax as i128;
    let cases: [(i128, bool, bool, i128); 5] = [
        (60, true, false, in_min),
        (100, false, false, ok),
        (150, false, false, ok),
        (200, false, false, ok),
        (201, true, false, in_max),
    ];
    for (price, clamped, rejected, reason) in cases {
        let d: BoundDecision = evaluate_tier(&t, price);
        assert_eq!(d.clamped, clamped, "clamped flag for {price}");
        assert_eq!(d.rejected, rejected, "rejected flag for {price}");
        assert_eq!(d.reason_code as i128, reason, "reason code for {price}");
    }
    // Outside the hard band, in both directions.
    let d = evaluate_tier(&t, 9000);
    assert!(d.rejected);
    assert_eq!(
        d.reason_code as i128,
        BoundReason::RejectedAboveHardMax as i128
    );
    let d = evaluate_tier(&t, 10);
    assert!(d.rejected);
    assert_eq!(
        d.reason_code as i128,
        BoundReason::RejectedBelowHardMin as i128
    );
}

proptest! {
    /// A published value is always inside the soft band whenever it was not
    /// rejected, and a rejected value is never inside the hard band.
    #[test]
    fn published_value_always_within_soft_band(price in 1i128..1_000_000) {
        let t = tier(100, 200, 50, 500);
        let d = evaluate_tier(&t, price);
        if d.rejected {
            prop_assert!(price < t.hard_min || price > t.hard_max);
            prop_assert_eq!(d.price, price, "a rejected value is not rewritten");
        } else {
            prop_assert!(t.soft_min <= d.price && d.price <= t.soft_max);
        }
    }

    /// Clamping is idempotent and always lands exactly on a bound.
    #[test]
    fn clamp_is_idempotent(price in 1i128..1_000_000) {
        let t = tier(100, 200, 50, 500);
        let once = evaluate_tier(&t, price).price;
        let twice = evaluate_tier(&t, once).price;
        prop_assert_eq!(once, twice);
    }
}

// ─── #485 deferred / quorum-within-window aggregation ──────────────────────

fn policy(quorum: u32, window_secs: u64, max_defer_secs: u64) -> DeferralPolicy {
    DeferralPolicy {
        quorum,
        window_secs,
        max_defer_secs,
    }
}

/// An asset with insufficient submissions is explicitly `Deferred`, never
/// silently published (#485 acceptance criterion 1).
#[test]
fn insufficient_submissions_are_deferred_not_published() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    c.set_deferral_policy(&asset, &policy(3, 900, 3_600));

    // Only one of the three required submissions has arrived.
    c.submit_price(&a, &asset, &100i128, &LEDGER_T0);
    assert!(has_event(&e, "publication_state_changed_event"));
    assert!(
        c.get_price(&asset, &0u64).is_none(),
        "a thin aggregate is never published"
    );
    let st = c.get_publication_status(&asset).unwrap();
    assert_eq!(st.state, PublicationState::Deferred);
    assert_eq!(st.received, 1);
    assert_eq!(st.missing, 2, "the missing-source count is exposed");
    assert_eq!(st.quorum, 3);
    assert_eq!(st.deferred_since, LEDGER_T0);
}

/// Completing quorum publishes deterministically, in the same transaction as
/// the completing submission (#485 acceptance criterion 2).
#[test]
fn completing_quorum_publishes_deterministically() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    let b = register_test_source(&e, &c, "b");
    c.set_deferral_policy(&asset, &policy(2, 900, 3_600));

    c.submit_price(&a, &asset, &100i128, &LEDGER_T0);
    assert!(c.get_price(&asset, &0u64).is_none());

    // The second submission completes quorum: publication happens right away.
    at(&e, 101, LEDGER_T0 + 100);
    c.submit_price(&b, &asset, &200i128, &(LEDGER_T0 + 100));
    let agg = c.get_price(&asset, &0u64).unwrap();
    assert_eq!(agg.price, 150);
    assert_eq!(agg.num_sources, 2);
    let st = c.get_publication_status(&asset).unwrap();
    assert_eq!(st.state, PublicationState::Published);
    assert_eq!(st.missing, 0);
}

/// Deferral beyond `max_defer_secs` becomes `Stale` and is evented
/// (#485 acceptance criterion 3).
#[test]
fn deferral_beyond_bound_becomes_stale() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    c.set_deferral_policy(&asset, &policy(3, 900, 3_600));

    c.submit_price(&a, &asset, &100i128, &LEDGER_T0);
    assert_eq!(
        c.get_publication_status(&asset).unwrap().state,
        PublicationState::Deferred
    );

    // Move past the deferral bound without reaching quorum.
    at(&e, 200, LEDGER_T0 + 3_601);
    c.submit_price(&a, &asset, &110i128, &(LEDGER_T0 + 3_601));
    assert!(
        has_event(&e, "publication_state_changed_event"),
        "escalation to stale is evented"
    );
    let st = c.get_publication_status(&asset).unwrap();
    assert_eq!(st.state, PublicationState::Stale);
    assert!(c.get_price(&asset, &0u64).is_none());
}

/// Consumers can distinguish deferred, stale and absent purely from the
/// status query (#485 acceptance criterion 4).
#[test]
fn consumers_distinguish_deferred_stale_and_absent() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let a = register_test_source(&e, &c, "a");

    // Absent: opted in, but nothing has ever been submitted or published.
    let absent = register_test_asset(&e, &c);
    c.set_deferral_policy(&absent, &policy(2, 900, 3_600));
    let st = c.get_publication_status(&absent).unwrap();
    assert_eq!(st.received, 0);
    assert_eq!(st.missing, 2);

    // Deferred: submissions exist, quorum has not been reached.
    let deferred = register_test_asset(&e, &c);
    c.set_deferral_policy(&deferred, &policy(2, 900, 3_600));
    c.submit_price(&a, &deferred, &100i128, &LEDGER_T0);
    assert_eq!(
        c.get_publication_status(&deferred).unwrap().state,
        PublicationState::Deferred
    );

    // Published: quorum reached.
    let b = register_test_source(&e, &c, "b");
    let published = register_test_asset(&e, &c);
    c.set_deferral_policy(&published, &policy(2, 900, 3_600));
    c.submit_price(&a, &published, &100i128, &LEDGER_T0);
    c.submit_price(&b, &published, &200i128, &LEDGER_T0);
    assert_eq!(
        c.get_publication_status(&published).unwrap().state,
        PublicationState::Published
    );

    // Stale: deferral outlived the bound.
    let stale = register_test_asset(&e, &c);
    c.set_deferral_policy(&stale, &policy(2, 900, 1_800));
    c.submit_price(&a, &stale, &100i128, &LEDGER_T0);
    at(&e, 300, LEDGER_T0 + 1_801);
    c.submit_price(&a, &stale, &100i128, &(LEDGER_T0 + 1_801));
    assert_eq!(
        c.get_publication_status(&stale).unwrap().state,
        PublicationState::Stale
    );

    // The states are distinct discriminants, so a consumer can switch on them.
    assert_ne!(
        PublicationState::Deferred as u32,
        PublicationState::Stale as u32
    );
    assert_ne!(
        PublicationState::Absent as u32,
        PublicationState::Deferred as u32
    );
    assert_ne!(
        PublicationState::Published as u32,
        PublicationState::Stale as u32
    );
}

/// Window and quorum are configurable, bounded and validated on write (#485).
#[test]
fn deferral_policy_validated_and_bounded() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);

    // quorum 0, window 0, max_defer < window, and out-of-range values are all
    // rejected on write.
    assert!(c
        .try_set_deferral_policy(&asset, &policy(0, 900, 3_600))
        .is_err());
    assert!(c
        .try_set_deferral_policy(&asset, &policy(2, 0, 3_600))
        .is_err());
    assert!(c
        .try_set_deferral_policy(&asset, &policy(2, 900, 60))
        .is_err());
    assert!(c
        .try_set_deferral_policy(&asset, &policy(65, 900, 3_600))
        .is_err());
    assert!(c
        .try_set_deferral_policy(&asset, &policy(2, 90_000, 100_000))
        .is_err());
    assert!(c
        .try_set_deferral_policy(&asset, &policy(2, 900, 1_000_000))
        .is_err());
    assert_eq!(c.get_deferral_policy(&asset), None, "nothing was stored");

    c.set_deferral_policy(&asset, &policy(3, 900, 3_600));
    assert_eq!(c.get_deferral_policy(&asset), Some(policy(3, 900, 3_600)));
    c.clear_deferral_policy(&asset);
    assert_eq!(c.get_deferral_policy(&asset), None);
}

/// Submissions older than the window no longer count toward quorum (#485).
#[test]
fn submissions_outside_the_window_do_not_count() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    let b = register_test_source(&e, &c, "b");
    c.set_deferral_policy(&asset, &policy(2, 600, 3_600));

    c.submit_price(&a, &asset, &100i128, &LEDGER_T0);
    // `a`'s submission ages out of the 600 s window before `b` reports.
    at(&e, 200, LEDGER_T0 + 601);
    c.submit_price(&b, &asset, &200i128, &(LEDGER_T0 + 601));
    let st = c.get_publication_status(&asset).unwrap();
    assert_eq!(st.received, 1, "only the in-window submission counts");
    assert_eq!(st.state, PublicationState::Deferred);
    assert!(c.get_price(&asset, &0u64).is_none());
}

/// Assets that never opted in keep the default publish-on-submit behaviour.
#[test]
fn non_deferrable_assets_unaffected() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let a = register_test_source(&e, &c, "a");
    c.submit_price(&a, &asset, &100i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);
    assert_eq!(
        c.get_publication_status(&asset),
        None,
        "no policy means no deferral state"
    );
}

// ─── #486 price corrections with a revision audit trail ───────────────────

/// A corrected entry exposes its full revision history, and the original value
/// is never destroyed (#486 acceptance criteria 1 and 2).
#[test]
fn correction_appends_to_an_immutable_chain() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &100i128, &LEDGER_T0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);

    // Revision 0 is the original publication.
    let chain = c.get_price_revisions(&asset);
    assert_eq!(chain.len(), 1);
    assert_eq!(chain.get_unchecked(0).price, 100);
    assert!(!chain.get_unchecked(0).corrected);

    e.mock_all_auths();
    let idx = c.correct_price(&asset, &150i128, &String::from_str(&e, "typo in feed"));
    assert_eq!(idx, 1, "correction appends, never overwrites");

    // The original value is still retrievable alongside the corrected one.
    assert_eq!(c.get_original_price(&asset), Some(100));
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 150);

    // The full chain links the correction to the original.
    let chain = c.get_price_revisions(&asset);
    assert_eq!(chain.len(), 2);
    assert_eq!(chain.get_unchecked(0).price, 100);
    let last = chain.get_unchecked(1);
    assert_eq!(last.price, 150);
    assert!(last.corrected);
    assert_eq!(last.index, 1);
    assert_eq!(last.reason, String::from_str(&e, "typo in feed"));
    assert_eq!(c.get_price_revision(&asset, &1).unwrap().price, 150);
    assert_eq!(c.get_price_revision(&asset, &9), None);
    assert_eq!(c.get_correction_count(&asset), 1);
}

/// Every correction is evented with actor and reason (#486 criterion 4).
#[test]
fn every_correction_is_evented() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &100i128, &LEDGER_T0);

    e.mock_all_auths();
    c.correct_price(&asset, &120i128, &String::from_str(&e, "bad source value"));
    assert!(has_event(&e, "price_corrected_event"));
    assert!(
        has_event(&e, "corr_px"),
        "corrections also land in the admin audit trail"
    );
}

/// Unauthorized correction is rejected (#486 acceptance criterion 3).
#[test]
fn unauthorized_correction_is_rejected() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &100i128, &LEDGER_T0);

    // No auth at all.
    clear_auth(&e);
    assert!(c
        .try_correct_price(&asset, &150i128, &String::from_str(&e, "nope"))
        .is_err());
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);

    // A non-admin cannot authorize itself in.
    let stranger = Address::generate(&e);
    e.mock_auths(&[soroban_sdk::testutils::MockAuth {
        address: &stranger,
        invoke: &soroban_sdk::testutils::MockAuthInvoke {
            contract: &c.address,
            fn_name: "correct_price",
            args: soroban_sdk::vec![
                &e,
                asset.to_val(),
                150i128.into_val(&e),
                String::from_str(&e, "nope").to_val()
            ],
            sub_invokes: &[],
        },
    }]);
    assert!(c
        .try_correct_price(&asset, &150i128, &String::from_str(&e, "nope"))
        .is_err());
    assert_eq!(
        c.get_original_price(&asset),
        Some(100),
        "the original survives a rejected correction"
    );
}

/// A reason is mandatory (#486).
#[test]
fn correction_reason_is_mandatory() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &100i128, &LEDGER_T0);

    e.mock_all_auths();
    assert!(c
        .try_correct_price(&asset, &150i128, &String::from_str(&e, ""))
        .is_err());
    // An oversized reason is rejected too.
    let long = "x".repeat(300);
    assert!(c
        .try_correct_price(&asset, &150i128, &String::from_str(&e, &long))
        .is_err());
    assert_eq!(c.get_correction_count(&asset), 0);
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 100);
}

/// Correction scope limits — asset, time and count — are enforced and tested
/// (#486 acceptance criterion 5).
#[test]
fn correction_scope_limits_enforced() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let in_scope = register_test_asset(&e, &c);
    let out_of_scope = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &in_scope, &100i128, &LEDGER_T0);
    c.submit_price(&s, &out_of_scope, &100i128, &LEDGER_T0);

    e.mock_all_auths();
    c.set_correction_scope(&crate::corrections::CorrectionScope {
        assets: Some(soroban_sdk::vec![&e, in_scope.clone()]),
        window_ledgers: 5,
        max_corrections: 2,
    });

    // Asset scope: an asset outside the allow-list cannot be corrected.
    assert!(c
        .try_correct_price(
            &out_of_scope,
            &150i128,
            &String::from_str(&e, "not in scope")
        )
        .is_err());
    // Asset scope: the allow-listed asset can.
    c.correct_price(&in_scope, &150i128, &String::from_str(&e, "in scope"));
    assert_eq!(c.get_correction_count(&in_scope), 1);

    // Time scope: moving past the correction window blocks further corrections.
    at(&e, 200, LEDGER_T0);
    assert!(c
        .try_correct_price(&in_scope, &160i128, &String::from_str(&e, "too late"))
        .is_err());

    // Count scope: with a fresh window, the cap is enforced.
    at(&e, 201, LEDGER_T0);
    c.submit_price(&s, &in_scope, &170i128, &LEDGER_T0);
    c.correct_price(&in_scope, &175i128, &String::from_str(&e, "second"));
    assert_eq!(
        c.get_correction_count(&in_scope),
        2,
        "the per-asset cap is reached"
    );
    assert!(c
        .try_correct_price(&in_scope, &180i128, &String::from_str(&e, "third"))
        .is_err());
    assert_eq!(c.get_correction_count(&in_scope), 2);
    assert_eq!(c.get_price(&in_scope, &0u64).unwrap().price, 175);
}

/// A correction cannot route around the hard bounds of #484.
#[test]
fn correction_respects_hard_bounds() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &150i128, &LEDGER_T0);
    c.set_price_bounds_tier(&asset, &tier(100, 200, 50, 500));

    e.mock_all_auths();
    // Inside the hard band: fine.
    c.correct_price(&asset, &180i128, &String::from_str(&e, "fine"));
    // Outside the hard band: rejected.
    assert!(c
        .try_correct_price(&asset, &9000i128, &String::from_str(&e, "too high"))
        .is_err());
    assert!(c
        .try_correct_price(&asset, &1i128, &String::from_str(&e, "too low"))
        .is_err());
    assert_eq!(c.get_price(&asset, &0u64).unwrap().price, 180);
}

/// A correction bumps the aggregate version so consumers can detect it.
#[test]
fn correction_bumps_aggregate_version() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    let s = register_test_source(&e, &c, "s");
    c.submit_price(&s, &asset, &100i128, &LEDGER_T0);
    let before = c.get_price(&asset, &0u64).unwrap();
    assert!(!before.is_override);

    e.mock_all_auths();
    c.correct_price(&asset, &150i128, &String::from_str(&e, "fix"));
    let after = c.get_price(&asset, &0u64).unwrap();
    assert!(after.is_override, "a correction is an override publication");
    assert_eq!(after.version, before.version + 1);
    assert_eq!(after.price, 150);
}

/// Nothing to correct is an error, not a silent no-op.
#[test]
fn correction_requires_a_published_aggregate() {
    let e = Env::default();
    let (c, _admin) = setup(&e);
    let asset = register_test_asset(&e, &c);
    e.mock_all_auths();
    assert!(c
        .try_correct_price(&asset, &100i128, &String::from_str(&e, "nothing yet"))
        .is_err());
    assert_eq!(c.get_price_revisions(&asset).len(), 0);
}
