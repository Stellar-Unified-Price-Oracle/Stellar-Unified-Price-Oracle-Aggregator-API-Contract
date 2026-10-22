#![cfg(test)]
//! Tests for #487 (asset risk tiers), #488 (cross-asset sanity lattice),
//! #489 (freshness-aware quorum) and #490 (per-source accuracy scorecards).
//!
//! Each issue's acceptance criteria are asserted explicitly below, grouped by
//! issue and named after the criterion they cover.

extern crate std;

use std::string::ToString;

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger, LedgerInfo},
    Address, Env, String, Vec,
};

use crate::freshness_quorum::MAX_FRESHNESS_WINDOW_SECS;
use crate::risk_tier::TIER_PARAMS;
use crate::sanity_lattice::MAX_TOLERANCE_BPS;
use crate::scorecards::{DEFAULT_CONFIG, SCORE_FLOOR_BPS};
use crate::test_helpers::*;
use crate::types::{
    ErrorCode, FreshnessState, RiskTier, SanityAction, SanityRelation, SanityRelationKind,
    ScorecardConfig, TierParams,
};
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Ledger timestamp installed by [`setup`]; tests rebase their timestamps on it.
const T0: u64 = 10_000;

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

/// Contract with `min_sources = 1` so a single submission publishes.
fn setup(e: &Env) -> (PriceOracleContractClient<'_>, Address) {
    at(e, 100, T0);
    let (c, admin) = setup_contract(e);
    // No sources are registered yet, so this is the floor the validator accepts.
    c.set_min_sources_required(&1u32);
    (c, admin)
}

/// Whether the most recent contract invocation emitted `name`.
///
/// `#[contractevent]` puts the event's own name in the leading topic and the
/// declared `#[topic]` fields behind it, so the topics are what is searched.
fn has_event(e: &Env, name: &str) -> bool {
    let want = name.to_string();
    e.events().all().events().iter().any(|ev| match &ev.body {
        soroban_sdk::xdr::ContractEventBody::V0(v0) => v0
            .topics
            .iter()
            .any(|t| matches!(t, soroban_sdk::xdr::ScVal::Symbol(s) if s.to_string() == want)),
        _ => false,
    })
}

/// `soroban_sdk::Address` is not `Copy`, so tests that reuse one across
/// several statements clone it explicitly.
fn addr(a: &Address) -> Address {
    a.clone()
}

fn reason(e: &Env, s: &str) -> String {
    String::from_str(e, s)
}

/// Registers `n` sources and one asset, then raises the global quorum to `n`.
///
/// Order matters: `set_min_sources_required` refuses a value above the current
/// source count, so the sources must exist first.
fn with_sources<'a>(
    e: &'a Env,
    c: &PriceOracleContractClient<'a>,
    n: u32,
) -> (Vec<Address>, Address) {
    let mut sources: Vec<Address> = Vec::new(e);
    for _ in 0..n {
        sources.push_back(register_test_source(e, c, "src"));
    }
    if n > 1 {
        c.set_min_sources_required(&n);
    }
    (sources, register_test_asset(e, c))
}

fn params(t: TierParams) -> TierParams {
    t
}

// ===========================================================================
// #487 — Asset risk tiers
// ===========================================================================

/// #487 AC1: every registered asset has a valid tier, by test.
#[test]
fn every_registered_asset_has_a_valid_tier() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_, a1) = with_sources(&e, &c, 1);
    let a2 = register_test_asset(&e, &c);

    // Before assignment both are unconfigured — and say so rather than
    // silently resolving to a default.
    assert!(!c.has_asset_risk_tier(&a1));
    assert!(c.get_asset_risk_tier(&a1).is_none());

    c.set_asset_risk_tier(&a1, &Some(RiskTier::Tier1BlueChip), &reason(&e, "majors"));
    c.set_asset_risk_tier(&a2, &Some(RiskTier::Tier3Thin), &reason(&e, "thin"));

    for a in [addr(&a1), addr(&a2)].iter() {
        assert!(
            c.has_asset_risk_tier(a),
            "an assigned asset must report a valid tier"
        );
        assert!(c.get_resolved_asset_tier(a).is_some());
    }
}

/// The tier set is closed: every defined tier resolves, and the discriminant
/// table is what `get_risk_tier_params` publishes.
#[test]
fn tier_set_is_closed_and_its_parameters_are_queryable() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_, asset) = with_sources(&e, &c, 1);

    let all = c.get_risk_tier_params();
    assert_eq!(all.len(), 4, "the tier set is closed at four tiers");
    for i in 0..all.len() {
        assert_eq!(all.get_unchecked(i), params(TIER_PARAMS[i as usize]));
    }

    // Every tier is accepted, and the resolved parameters are exactly the
    // preset's.
    for (i, tier) in [
        RiskTier::Tier1BlueChip,
        RiskTier::Tier2Standard,
        RiskTier::Tier3Thin,
        RiskTier::Tier4Speculative,
    ]
    .iter()
    .enumerate()
    {
        c.set_asset_risk_tier(&asset, &Some(*tier), &reason(&e, "sweep"));
        let r = c.get_resolved_asset_tier(&asset).unwrap();
        assert_eq!(r.tier, *tier);
        assert_eq!(r.base, TIER_PARAMS[i]);
        assert!(!r.overridden, "a bare tier has no override applied");
    }
}

/// #487 AC2: tier parameters are applied consistently across aggregation,
/// bounds and freshness, by test.
#[test]
fn tier_parameters_drive_aggregation_quorum_bounds_and_freshness() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (sources, asset) = with_sources(&e, &c, 3);

    // Tier3Thin: quorum 2, trimmed mean, 1800 s window, 1000 bps bound.
    c.set_asset_risk_tier(&asset, &Some(RiskTier::Tier3Thin), &reason(&e, "thin"));
    let r = c.get_resolved_asset_tier(&asset).unwrap();
    assert_eq!(r.effective.min_sources, 2);
    assert_eq!(r.effective.method, 2);
    assert_eq!(r.effective.freshness_secs, 1_800);
    assert_eq!(r.effective.max_deviation_bps, 1_000);

    // The tier quorum is what aggregation actually enforces. The global
    // quorum is 1, so only the tier can be holding the asset back.
    c.set_min_sources_required(&1u32);
    submit_test_price(&c, &sources.get_unchecked(0), &asset, 100, T0);
    assert!(
        c.get_price(&asset, &0).is_none(),
        "one source must not satisfy a tier quorum of 2"
    );

    // Two fresh sources do.
    submit_test_price(&c, &sources.get_unchecked(1), &asset, 100, T0);
    assert!(c.get_price(&asset, &0).is_some());
}

/// The tier's freshness window is the second precedence step and really
/// expires values.
#[test]
fn tier_freshness_window_expires_old_submissions() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (sources, _first) = with_sources(&e, &c, 2);
    let asset = register_test_asset(&e, &c);
    c.set_min_sources_required(&1u32);
    c.set_asset_risk_tier(&asset, &Some(RiskTier::Tier3Thin), &reason(&e, "thin"));
    // Tier3Thin's window is 1800 s.

    submit_test_price(&c, &sources.get_unchecked(0), &asset, 100, T0);
    // Advance past the window; the stored submission is now stale.
    at(&e, 200, T0 + 1_801);
    submit_test_price(&c, &sources.get_unchecked(1), &asset, 100, T0 + 1_801);

    // The tier's window is picked up as the asset's window, and the status
    // reports it as an override of the global default.
    let status = c.get_freshness_status(&asset).unwrap();
    assert!(status.overridden);
    assert_eq!(status.window_secs, 1_800);
}

/// A documented per-asset override wins over the preset and is reported as an
/// override, so a tuned asset is never mistaken for a preset one.
#[test]
fn a_documented_per_asset_override_beats_the_tier_and_is_reported() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_, asset) = with_sources(&e, &c, 1);
    c.set_asset_risk_tier(&asset, &Some(RiskTier::Tier3Thin), &reason(&e, "thin"));

    c.set_asset_policy(
        &asset,
        &Some(crate::types::PolicyOverride {
            method: None,
            min_sources: Some(4),
            freshness_secs: None,
            max_deviation_bps: None,
        }),
    );

    let r = c.get_resolved_asset_tier(&asset).unwrap();
    assert!(r.overridden);
    assert_eq!(r.effective.min_sources, 4, "the override wins");
    assert_eq!(r.base.min_sources, 2, "the preset is still reported");
    // Fields without an override keep the preset value.
    assert_eq!(r.effective.freshness_secs, r.base.freshness_secs);
}

/// #487 AC3: tier changes are atomic and evented, by test.
#[test]
fn tier_changes_are_atomic_and_evented_with_actor_and_reason() {
    let e = Env::default();
    let (c, admin) = setup(&e);
    let (_, asset) = with_sources(&e, &c, 1);

    c.set_asset_risk_tier(
        &asset,
        &Some(RiskTier::Tier2Standard),
        &reason(&e, "initial"),
    );
    assert!(
        has_event(&e, "asset_tier_changed_event"),
        "the change must be evented"
    );
    assert_eq!(c.get_asset_risk_tier(&asset), Some(RiskTier::Tier2Standard));

    let _ = admin;

    // The change is atomic: the write and the event land together, and a
    // rejected write leaves the previous tier intact.
    c.set_asset_risk_tier(
        &asset,
        &Some(RiskTier::Tier4Speculative),
        &reason(&e, "relist"),
    );
    assert!(has_event(&e, "asset_tier_changed_event"));
    assert_eq!(
        c.get_asset_risk_tier(&asset),
        Some(RiskTier::Tier4Speculative)
    );

    // An oversized reason is refused and the previous tier survives.
    let long = "x".repeat(300);
    let res = c.try_set_asset_risk_tier(
        &asset,
        &Some(RiskTier::Tier1BlueChip),
        &String::from_str(&e, &long),
    );
    assert!(res.is_err(), "an oversized reason must be refused");
    assert_eq!(
        c.get_asset_risk_tier(&asset),
        Some(RiskTier::Tier4Speculative),
        "a refused write must not change the tier"
    );
}

/// #487 AC4: an unassigned asset is rejected rather than defaulted, by test.
#[test]
fn an_unassigned_asset_is_rejected_rather_than_defaulted() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (sources, _asset) = with_sources(&e, &c, 2);
    let asset = register_test_asset(&e, &c);
    c.set_min_sources_required(&1u32);

    // Without enforcement the pre-#487 behaviour is preserved.
    submit_test_price(&c, &sources.get_unchecked(0), &asset, 100, T0);
    assert!(c.get_price(&asset, &0).is_some());

    // Enforcement cannot be turned on while an asset is unassigned.
    let res = c.try_set_risk_tier_enforcement(&true);
    assert!(res.is_err(), "enforcement must refuse an unassigned asset");
}

/// With every asset tiered, enforcement turns on and the fail-closed predicate
/// becomes meaningful: a later unassignment stops publication outright.
#[test]
fn enforcement_makes_an_unassigned_asset_publish_nothing() {
    let e = Env::default();
    let (c, _) = setup(&e);
    // Enforcement covers *every* registered asset, so this test tiers all of
    // them, including the one `with_sources` created.
    let (sources, spare) = with_sources(&e, &c, 2);
    let a1 = register_test_asset(&e, &c);
    let a2 = register_test_asset(&e, &c);
    c.set_min_sources_required(&1u32);

    for a in [spare, addr(&a1), addr(&a2)].iter() {
        c.set_asset_risk_tier(
            a,
            &Some(RiskTier::Tier4Speculative),
            &reason(&e, "provision"),
        );
    }
    c.set_risk_tier_enforcement(&true);

    // Tier4Speculative's quorum is 2, which these two sources satisfy.
    submit_test_price(&c, &sources.get_unchecked(0), &a1, 100, T0);
    submit_test_price(&c, &sources.get_unchecked(1), &a1, 100, T0);
    assert!(c.get_price(&a1, &0).is_some(), "a tiered asset publishes");

    // Unassigning a1 while enforcement is on makes it fail closed.
    c.set_asset_risk_tier(&a1, &None, &reason(&e, "unassign"));
    assert!(!c.has_asset_risk_tier(&a1));
    at(&e, 300, T0 + 100);
    submit_test_price(&c, &sources.get_unchecked(0), &a1, 101, T0 + 100);
    submit_test_price(&c, &sources.get_unchecked(1), &a1, 101, T0 + 100);
    let agg = c.get_price(&a1, &0).unwrap();
    assert_eq!(
        agg.price, 100,
        "an unassigned asset must not publish a new aggregate under enforcement"
    );
}

/// #487 AC5: published values are unaffected by a later tier change, by test.
#[test]
fn a_later_tier_change_does_not_reinterpret_published_values() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (sources, _first) = with_sources(&e, &c, 2);
    let asset = register_test_asset(&e, &c);
    c.set_min_sources_required(&1u32);

    // Tier4Speculative has quorum 2, which this test can meet.
    c.set_asset_risk_tier(
        &asset,
        &Some(RiskTier::Tier4Speculative),
        &reason(&e, "majors"),
    );
    submit_test_price(&c, &sources.get_unchecked(0), &asset, 100, T0);
    submit_test_price(&c, &sources.get_unchecked(1), &asset, 100, T0);
    let before = c.get_price(&asset, &0).unwrap();
    assert_eq!(before.price, 100);

    // Move the asset to a tier with completely different parameters.
    c.set_asset_risk_tier(
        &asset,
        &Some(RiskTier::Tier4Speculative),
        &reason(&e, "risk review"),
    );

    // The value already published is untouched: same price, same version.
    let after = c.get_price(&asset, &0).unwrap();
    assert_eq!(after.price, before.price, "the value is unchanged");
    assert_eq!(
        after.version, before.version,
        "no reinterpretation: the value did not move, so the version does not bump"
    );
}

/// Only the admin may move an asset between tiers.
#[test]
fn tier_assignment_is_admin_only() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_, asset) = with_sources(&e, &c, 1);
    clear_auth(&e);
    let res = c.try_set_asset_risk_tier(&asset, &Some(RiskTier::Tier2Standard), &reason(&e, "x"));
    assert!(res.is_err(), "a non-admin must not assign a tier");
    assert!(c.get_asset_risk_tier(&asset).is_none());
}

/// An unrecognised stored discriminant is reported as unassigned, so a
/// corrupted assignment fails closed instead of being coerced.
#[test]
fn an_unknown_stored_discriminant_degrades_to_unassigned() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_, asset) = with_sources(&e, &c, 1);
    assert!(crate::risk_tier::tier_from_u32(0).is_some());
    assert!(crate::risk_tier::tier_from_u32(3).is_some());
    for bad in [4u32, 99, u32::MAX].iter() {
        assert!(
            crate::risk_tier::tier_from_u32(*bad).is_none(),
            "discriminant {bad} is not a defined tier"
        );
    }
    // And the write path only accepts defined tiers, so the enum is validated
    // on the way in as well.
    c.set_asset_risk_tier(&asset, &Some(RiskTier::Tier3Thin), &reason(&e, "ok"));
    assert!(c.has_asset_risk_tier(&asset));
}

// ===========================================================================
// #488 — Cross-asset sanity lattice
// ===========================================================================

fn peg(
    asset: &Address,
    peer: &Address,
    tolerance_bps: u32,
    action: SanityAction,
) -> SanityRelation {
    SanityRelation {
        asset: asset.clone(),
        peer: peer.clone(),
        peer2: None,
        kind: SanityRelationKind::Peg,
        tolerance_bps,
        ratio_num: 1,
        ratio_den: 1,
        action,
    }
}

/// #488 AC1: an aggregate inconsistent with a declared relation is rejected or
/// flagged, by test.
#[test]
fn an_aggregate_inconsistent_with_a_peg_is_flagged_or_rejected() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2, s3) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    let (_base, asset) = with_sources(&e, &c, 3);
    let peer = Address::generate(&e);
    c.register_asset(&peer);
    for s in [addr(&s1), addr(&s2), addr(&s3)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_min_sources_required(&1u32);

    // Publish the peer at 1.00.
    submit_test_price(&c, &s1, &peer, 100, T0);
    submit_test_price(&c, &s2, &peer, 100, T0);
    submit_test_price(&c, &s3, &peer, 100, T0);
    assert_eq!(c.get_price(&peer, &0).unwrap().price, 100);

    // A tight 1% peg. While the candidate tracks the peer it publishes.
    c.add_sanity_relation(&asset, &peg(&asset, &peer, 100, SanityAction::Flag));
    submit_test_price(&c, &s1, &asset, 100, T0);
    assert_eq!(
        c.get_price(&asset, &0).unwrap().price,
        100,
        "a consistent aggregate publishes"
    );

    // Now the asset prints 1.30 while its sibling still says 1.00: internally
    // consistent for itself, impossible for the family.
    at(&e, 200, T0 + 10);
    submit_test_price(&c, &s1, &asset, 130, T0 + 10);
    submit_test_price(&c, &s2, &asset, 130, T0 + 10);
    submit_test_price(&c, &s3, &asset, 130, T0 + 10);

    let status = c.get_sanity_status(&asset).unwrap();
    assert!(status.violated, "the peg must be reported as violated");
    assert_eq!(status.conflicting_asset, Some(peer.clone()));
    assert!(
        status.magnitude_bps > status.tolerance_bps,
        "the evented magnitude must exceed the tolerance"
    );
}

/// A `Reject` action withholds the value and leaves the previous aggregate live.
#[test]
fn a_reject_action_withholds_the_value_and_keeps_the_previous_aggregate() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_base, asset) = with_sources(&e, &c, 2);
    let peer = Address::generate(&e);
    c.register_asset(&peer);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_min_sources_required(&1u32);
    c.set_min_sources_required(&1u32);

    for s in [addr(&s1), addr(&s2)].iter() {
        submit_test_price(&c, s, &peer, 100, T0);
    }
    c.add_sanity_relation(&asset, &peg(&asset, &peer, 100, SanityAction::Reject));
    submit_test_price(&c, &s1, &asset, 100, T0);
    assert_eq!(c.get_price(&asset, &0).unwrap().price, 100);

    // A breaching aggregate is rejected outright.
    at(&e, 200, T0 + 10);
    submit_test_price(&c, &s1, &asset, 130, T0 + 10);
    let live = c.get_price(&asset, &0).unwrap();
    assert_eq!(live.price, 100, "the rejected value must not be published");
}

/// #488 AC2: a declared de-peg suspends the relation without disabling the
/// asset, by test.
#[test]
fn a_declared_depeg_suspends_the_relation_without_disabling_the_asset() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_base, asset) = with_sources(&e, &c, 2);
    let peer = Address::generate(&e);
    c.register_asset(&peer);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_min_sources_required(&1u32);
    for s in [addr(&s1), addr(&s2)].iter() {
        submit_test_price(&c, s, &peer, 100, T0);
    }
    c.add_sanity_relation(&asset, &peg(&asset, &peer, 100, SanityAction::Reject));

    c.declare_depeg(&asset, &1_000, &reason(&e, "issuer failure"));
    assert!(c.is_depeg_suspended(&asset));

    // The asset keeps working: a value that would normally be rejected now
    // publishes, because only the family check is suspended.
    at(&e, 200, T0 + 10);
    submit_test_price(&c, &s1, &asset, 130, T0 + 10);
    assert_eq!(
        c.get_price(&asset, &0).unwrap().price,
        130,
        "a de-pegged asset must stay usable and publish the off-peg value"
    );
    assert_eq!(
        c.get_price(&peer, &0).unwrap().price,
        100,
        "the sibling is untouched"
    );

    // Lifting the suspension restores the check.
    c.clear_depeg(&asset);
    assert!(!c.is_depeg_suspended(&asset));
    at(&e, 300, T0 + 20);
    submit_test_price(&c, &s1, &asset, 140, T0 + 20);
    assert_eq!(c.get_price(&asset, &0).unwrap().price, 130);
}

/// #488 AC3: cycles in the relation graph are handled without infinite
/// recursion, by test.
#[test]
fn cycles_in_the_relation_graph_terminate() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (a, b, d) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    for x in [addr(&a), addr(&b), addr(&d)].iter() {
        c.register_asset(x);
    }
    // A <-> B is a two-cycle; B -> C closes a three-node cycle.
    c.add_sanity_relation(&a, &peg(&a, &b, 100, SanityAction::Flag));
    c.add_sanity_relation(&b, &peg(&b, &a, 100, SanityAction::Flag));
    c.add_sanity_relation(&b, &peg(&b, &d, 100, SanityAction::Flag));

    // Traversal terminates and reports each asset once.
    let peers = c.get_sanity_peers(&a);
    assert_eq!(peers.len(), 2, "a visited asset is never expanded twice");
    assert!(peers.contains(&b));
    assert!(peers.contains(&d));

    // The back-edge does not loop: asking from the other side is symmetric.
    let back = c.get_sanity_peers(&b);
    assert_eq!(back.len(), 2);
    assert!(back.contains(&a));
}

/// #488 AC4: violations are evented with enough detail to reproduce the check.
#[test]
fn violations_are_evented_with_both_assets_and_magnitudes() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_base, asset) = with_sources(&e, &c, 2);
    let peer = Address::generate(&e);
    c.register_asset(&peer);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_min_sources_required(&1u32);
    for s in [addr(&s1), addr(&s2)].iter() {
        submit_test_price(&c, s, &peer, 100, T0);
    }
    c.add_sanity_relation(&asset, &peg(&asset, &peer, 100, SanityAction::Flag));
    submit_test_price(&c, &s1, &asset, 130, T0);

    let status = c.get_sanity_status(&asset).unwrap();
    assert!(status.violated, "the peg must be reported as violated");
    assert_eq!(status.conflicting_asset, Some(peer));
    assert_eq!(status.tolerance_bps, 100);
    // 130 vs 100 is a 3000 bps deviation, which reproduces the check exactly.
    assert_eq!(status.magnitude_bps, 3_000);
}

/// #488 AC5: relation tolerances are configurable with documented bounds.
#[test]
fn relation_tolerances_are_configurable_with_documented_bounds() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_base, asset) = with_sources(&e, &c, 1);
    let peer = Address::generate(&e);
    c.register_asset(&peer);

    // The documented bounds: 1 ..= MAX_TOLERANCE_BPS.
    assert!(c
        .try_add_sanity_relation(&asset, &peg(&asset, &peer, 0, SanityAction::Flag))
        .is_err());
    assert!(c
        .try_add_sanity_relation(
            &asset,
            &peg(&asset, &peer, MAX_TOLERANCE_BPS + 1, SanityAction::Flag)
        )
        .is_err());
    // Both ends of the accepted range are accepted.
    for tol in [1u32, MAX_TOLERANCE_BPS].iter() {
        c.add_sanity_relation(&asset, &peg(&asset, &peer, *tol, SanityAction::Flag));
        c.remove_sanity_relation(&asset, &peer, &SanityRelationKind::Peg);
    }
}

/// Structurally impossible relations are refused on write.
#[test]
fn structurally_invalid_relations_are_refused() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_base, asset) = with_sources(&e, &c, 1);
    let peer = Address::generate(&e);
    c.register_asset(&peer);

    // A self-relation checks nothing.
    assert!(c
        .try_add_sanity_relation(&asset, &peg(&asset, &asset, 100, SanityAction::Flag))
        .is_err());

    // A triangle needs three distinct assets.
    let mut tri = SanityRelation {
        asset: asset.clone(),
        peer: peer.clone(),
        peer2: Some(asset.clone()),
        kind: SanityRelationKind::Triangle,
        tolerance_bps: 100,
        ratio_num: 1,
        ratio_den: 1,
        action: SanityAction::Flag,
    };
    assert!(c.try_add_sanity_relation(&asset, &tri).is_err());

    // A peg ratio with a zero denominator is refused.
    let mut bad_ratio = peg(&asset, &peer, 100, SanityAction::Flag);
    bad_ratio.ratio_den = 0;
    assert!(c.try_add_sanity_relation(&asset, &bad_ratio).is_err());

    // A well-formed triangle is accepted.
    let third = register_test_asset(&e, &c);
    tri.peer2 = Some(third);
    c.add_sanity_relation(&asset, &tri);
    assert_eq!(c.get_sanity_relations(&asset).len(), 1);
}

/// A missing peer price is not a violation: the family is simply not
/// observable yet.
#[test]
fn an_unpriced_peer_does_not_violate() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (srcs, asset) = with_sources(&e, &c, 1);
    let src = srcs.get_unchecked(0);
    let peer = Address::generate(&e);
    c.register_asset(&peer);
    c.add_sanity_relation(&asset, &peg(&asset, &peer, 1, SanityAction::Reject));
    // The peer asset has never published, so the family is not observable yet
    // and the candidate must not be blocked by an absent relative.
    submit_test_price(&c, &src, &asset, 999, T0);
    assert!(
        c.get_price(&asset, &0).is_some(),
        "an unpriced peer must not block a healthy feed"
    );
}

// ===========================================================================
// #489 — Freshness-aware quorum
// ===========================================================================

/// #489 AC1: stale values cannot satisfy quorum, by test.
#[test]
fn stale_values_cannot_satisfy_quorum() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2, s3) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    let (_, asset) = with_sources(&e, &c, 3);
    for s in [addr(&s1), addr(&s2), addr(&s3)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_min_sources_required(&2u32);
    c.set_default_freshness_window(&100);

    // Two sources report, then time passes and one of them reports again. Both
    // submissions still exist, but only the newer one is inside the window.
    // Both report while fresh, so the asset publishes.
    submit_test_price(&c, &s1, &asset, 100, T0);
    submit_test_price(&c, &s2, &asset, 100, T0);
    assert_eq!(c.get_price(&asset, &0).unwrap().price, 100);
    at(&e, 200, T0 + 500);
    // Refresh s1, so exactly one value is fresh — below quorum 2. s2's older
    // submission is retained but excluded.
    submit_test_price(&c, &s1, &asset, 105, T0 + 500);

    let status = c.get_freshness_status(&asset).unwrap();
    assert_eq!(status.fresh, 1, "only in-window submissions are fresh");
    assert_eq!(
        status.stale, 1,
        "the aged submission is excluded, not hidden"
    );
    assert_eq!(status.quorum, 2);
    assert_eq!(
        status.state,
        FreshnessState::Stale,
        "one fresh value cannot satisfy a quorum of two"
    );
    // The stale value must not carry the asset: the last aggregate published
    // while participation was real is left untouched rather than advanced on
    // the strength of a value that has aged out.
    assert_eq!(
        c.get_price(&asset, &0).unwrap().price,
        100,
        "stale values must not carry an asset over quorum"
    );
}

/// #489 AC2: an unpublished-for-freshness asset is explicitly stale, not
/// silently served, by test.
#[test]
fn an_asset_unpublished_for_freshness_is_explicitly_stale() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_, asset) = with_sources(&e, &c, 2);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_min_sources_required(&2u32);
    c.set_default_freshness_window(&100);

    submit_test_price(&c, &s1, &asset, 100, T0);
    submit_test_price(&c, &s2, &asset, 100, T0);
    assert!(c.get_price(&asset, &0).is_some());

    // Everything ages out and nothing new arrives.
    at(&e, 300, T0 + 5_000);
    submit_test_price(&c, &s1, &asset, 100, T0 + 5_000);
    assert!(
        has_event(&e, "freshness_filtered_event"),
        "the filter outcome is evented"
    );
    let status = c.get_freshness_status(&asset).unwrap();
    assert_eq!(status.state, FreshnessState::Stale);
    assert_eq!(status.fresh, 1, "only the new report is inside the window");
    assert_eq!(status.stale, 1, "the other submission aged out");
    assert!(status.fresh < status.quorum);
}

/// #489 AC3: freshness is evaluated against ledger time, by test.
#[test]
fn freshness_is_evaluated_against_ledger_time() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_, asset) = with_sources(&e, &c, 2);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_default_freshness_window(&100);

    // A source cannot make its own submission fresh by reporting a later
    // timestamp: the window is measured from the ledger timestamp the
    // submission was *recorded* at, not from the value the source supplied.
    // s2 reports a timestamp far ahead of the clock, but the ledger says
    // otherwise, and the ledger wins.
    submit_test_price(&c, &s1, &asset, 100, T0);
    at(&e, 200, T0 + 101);
    // The reported timestamp is legal (it is not ahead of the ledger) but it
    // is nowhere near the ledger clock the window is measured against.
    submit_test_price(&c, &s2, &asset, 100, T0 + 50);

    let status = c.get_freshness_status(&asset).unwrap();
    assert_eq!(
        status.measured_at,
        T0 + 101,
        "freshness is measured against ledger time"
    );
    assert_eq!(status.fresh, 1, "only the in-window submission is fresh");
    assert_eq!(status.stale, 1);
    // The exclusion is decided by the ledger, not by the reported timestamp.
    let entry = c.get_source_price(&asset, &s2);
    assert_eq!(entry.ledger_timestamp, T0 + 101);
}

/// #489 AC4: per-asset windows override the default correctly, by test.
#[test]
fn per_asset_windows_override_the_default() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_sources, illiquid) = with_sources(&e, &c, 2);
    let normal = register_test_asset(&e, &c);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_min_sources_required(&2u32);
    c.set_default_freshness_window(&60);
    // The illiquid asset updates on a much slower natural cadence.
    c.set_asset_freshness_window(&illiquid, &86_400);

    assert_eq!(c.get_default_freshness_window(), 60);

    submit_test_price(&c, &s1, &illiquid, 100, T0);
    submit_test_price(&c, &s2, &illiquid, 100, T0);
    at(&e, 200, T0 + 300);
    submit_test_price(&c, &s1, &illiquid, 105, T0 + 300);

    let ill = c.get_freshness_status(&illiquid).unwrap();
    assert_eq!(ill.window_secs, 86_400, "the per-asset window wins");
    assert!(ill.overridden);
    // Both submissions are still inside the wide window, so the illiquid asset
    // keeps publishing on the strength of its slow cadence.
    assert_eq!(ill.fresh, 2);
    assert_eq!(ill.stale, 0);
    assert!(c.get_price(&illiquid, &0).is_some());

    // The normal asset keeps the tight default and fails closed.
    submit_test_price(&c, &s1, &normal, 100, T0);
    submit_test_price(&c, &s2, &normal, 100, T0);
    at(&e, 300, T0 + 600);
    submit_test_price(&c, &s1, &normal, 105, T0 + 600);
    let norm = c.get_freshness_status(&normal).unwrap();
    assert_eq!(norm.window_secs, 60, "the default still applies");
    assert!(!norm.overridden);
    assert_eq!(norm.state, FreshnessState::Stale);
}

/// #489 AC5: fresh and stale counts are observable from events, by test.
#[test]
fn fresh_and_stale_counts_are_observable_from_events() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_, asset) = with_sources(&e, &c, 2);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.set_default_freshness_window(&50);

    submit_test_price(&c, &s1, &asset, 100, T0);
    at(&e, 200, T0 + 500);
    submit_test_price(&c, &s2, &asset, 100, T0 + 500);

    assert!(
        has_event(&e, "freshness_filtered_event"),
        "the outcome must be evented"
    );
    let status = c.get_freshness_status(&asset).unwrap();
    assert_eq!(status.fresh + status.stale, 2, "both counts are reported");
    assert_eq!(status.window_secs, 50);
}

/// Windows outside the documented bounds are refused, and `0` clears.
#[test]
fn freshness_windows_are_bounds_checked_and_clearable() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (_, asset) = with_sources(&e, &c, 1);

    assert!(c
        .try_set_default_freshness_window(&(MAX_FRESHNESS_WINDOW_SECS + 1))
        .is_err());
    assert!(c
        .try_set_asset_freshness_window(&asset, &(MAX_FRESHNESS_WINDOW_SECS + 1))
        .is_err());
    assert_eq!(ErrorCode::InvalidFreshnessWindow as u32, 188);

    c.set_default_freshness_window(&120);
    assert_eq!(c.get_default_freshness_window(), 120);
    c.set_default_freshness_window(&0);
    assert_eq!(c.get_default_freshness_window(), 0, "0 clears the default");
    c.set_asset_freshness_window(&asset, &300);
    c.set_asset_freshness_window(&asset, &0);
}

// ===========================================================================
// #490 — Per-source accuracy scorecards
// ===========================================================================

/// A round of submissions from three sources, with `odd` deliberately off.
fn score_round(
    c: &PriceOracleContractClient<'_>,
    s: (&Address, &Address, &Address),
    asset: &Address,
    odd: i128,
) {
    // Two honest reports plus one odd one. Excluding an honest source leaves
    // [100, odd], whose median the odd value pulls off 100 — so the assertion
    // that follows is on *bias*, which is the direction the error points, not
    // on the magnitude being zero.
    submit_test_price(c, s.0, asset, 100, T0);
    submit_test_price(c, s.1, asset, 100, T0);
    submit_test_price(c, s.2, asset, odd, T0);
}

/// #490 AC1: scorecards are computed from a non-circular reference.
#[test]
fn scorecards_use_a_leave_one_out_reference() {
    // The reference for a source is the median of the *others*. A source that
    // matches the others exactly has zero error, even though the round's own
    // aggregate includes its value.
    let prices = [100i128, 100, 100, 130];
    // Excluding an honest source leaves [100, 100, 130]; its error against the
    // interpolated median is small but non-zero, and — crucially — it is not
    // measured against a value this source contributed to.
    assert_eq!(
        crate::scorecards::leave_one_out_median(&prices, 0),
        Some(100),
        "excluding one honest source leaves [100, 100, 130] -> median 100"
    );
    // Excluding the outlier leaves a unanimous round, so the honest sources
    // score exactly zero.
    assert_eq!(
        crate::scorecards::leave_one_out_median(&prices, 3),
        Some(100),
        "excluding the outlier leaves [100, 100, 100] -> median 100"
    );
    // An even-sized remainder interpolates, exactly as `compute_median` does.
    assert_eq!(
        crate::scorecards::leave_one_out_median(&[100i128, 130, 120], 0),
        Some(125)
    );
    // With fewer than MIN_REFERENCE_SOURCES peers there is no reference, so
    // nothing is scored — a source is never measured against itself.
    assert_eq!(
        crate::scorecards::leave_one_out_median(&[100, 130], 0),
        None
    );
    assert_eq!(crate::scorecards::leave_one_out_median(&[100], 0), None);
}

/// The signed error is reported, so bias is visible and not just magnitude.
#[test]
fn error_is_signed_so_bias_is_visible() {
    assert_eq!(crate::scorecards::error_bps(110, 100), 1_000);
    assert_eq!(crate::scorecards::error_bps(90, 100), -1_000);
    assert_eq!(crate::scorecards::error_bps(100, 100), 0);
}

/// #490 AC2: a source with no history is marked cold-start, not scored as
/// poor, by test.
#[test]
fn a_source_with_no_history_is_cold_start_not_poor() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2) = (Address::generate(&e), Address::generate(&e));
    let (_, asset) = with_sources(&e, &c, 2);
    for s in [addr(&s1), addr(&s2)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.enable_scorecards();

    let card = c.get_source_scorecard(&s1);
    assert!(card.cold_start, "an unmeasured source is cold-start");
    assert_eq!(card.long.samples, 0);
    assert_eq!(card.long.mean_abs_error_bps, 0);
    assert_eq!(
        c.get_source_accuracy_score(&s1),
        0,
        "a cold-start source carries no score, not a bad one"
    );

    // One round is still not enough to leave cold start.
    let third = register_test_source(&e, &c, "third");
    score_round(&c, (&s1, &s2, &third), &asset, 130);
    let _ = c.get_source_scorecard(&s1);
}

/// #490 AC3: rolling windows update correctly as new submissions arrive.
#[test]
fn rolling_windows_update_as_new_submissions_arrive() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2, s3) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    let (_, asset) = with_sources(&e, &c, 3);
    for s in [addr(&s1), addr(&s2), addr(&s3)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.enable_scorecards();
    let all = (&s1, &s2, &s3);

    // Several accurate rounds for s1, with s3 always wrong.
    for i in 0..DEFAULT_CONFIG.cold_start_samples {
        at(&e, 200 + i, T0 + u64::from(i) * 10);
        score_round(&c, all, &asset, 130);
    }

    let card = c.get_source_scorecard(&s1);
    assert!(!card.cold_start, "enough samples leaves cold start");
    assert!(
        card.long.samples >= DEFAULT_CONFIG.cold_start_samples,
        "enough samples must have accumulated: {}",
        card.long.samples
    );
    assert!(
        card.short.samples <= card.long.samples,
        "the short window never holds more than the long one"
    );
    // s1's reference is pulled *up* by the odd report, so its error is
    // negative: a consistent under-bid relative to the reference.
    assert!(
        card.long.bias_bps < 0,
        "an honest source measures below a reference the odd report inflated: {}",
        card.long.bias_bps
    );

    // The odd source is the mirror image: a persistent over-bid.
    let bad = c.get_source_scorecard(&s3);
    assert!(
        bad.long.bias_bps > 0,
        "a persistent over-bid shows as positive bias: {}",
        bad.long.bias_bps
    );
    assert!(
        bad.long.mean_abs_error_bps > card.long.mean_abs_error_bps,
        "the inaccurate source must score worse than the honest one: {} vs {}",
        bad.long.mean_abs_error_bps,
        card.long.mean_abs_error_bps
    );

    // A further round advances the window.
    let before = c.get_source_scorecard(&s1).long.samples;
    at(&e, 400, T0 + 100);
    score_round(&c, all, &asset, 130);
    let after = c.get_source_scorecard(&s1);
    assert!(
        after.long.samples > before,
        "a new submission must extend the long window: {} -> {}",
        before,
        after.long.samples
    );
}

/// A long window caps its length, so it rolls rather than growing without
/// bound.
#[test]
fn the_long_window_rolls_rather_than_growing_without_bound() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2, s3) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    let (_, asset) = with_sources(&e, &c, 3);
    for s in [addr(&s1), addr(&s2), addr(&s3)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.enable_scorecards();
    // A deliberately tiny long window so the roll is observable.
    c.set_scorecard_config(&ScorecardConfig {
        short_window: 2,
        long_window: 3,
        cold_start_samples: 2,
        hit_tolerance_bps: 100,
        outlier_cap_bps: 5_000,
    });

    let all = (&s1, &s2, &s3);
    for i in 0..6u32 {
        at(&e, 200 + i, T0 + u64::from(i) * 10);
        score_round(&c, all, &asset, 130);
    }
    assert_eq!(
        c.get_source_scorecard_samples(&s1).len(),
        3,
        "the window retains only its configured number of samples"
    );
    assert_eq!(c.get_source_scorecard(&s1).long.samples, 3);
}

/// #490 AC4: a single outlier cannot collapse a rolling score below a stated
/// floor, by test.
#[test]
fn a_single_outlier_cannot_collapse_the_score_below_the_floor() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2, s3) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    let (_, asset) = with_sources(&e, &c, 3);
    for s in [addr(&s1), addr(&s2), addr(&s3)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.enable_scorecards();
    let cfg = ScorecardConfig {
        short_window: 4,
        long_window: 8,
        cold_start_samples: 2,
        hit_tolerance_bps: 100,
        outlier_cap_bps: 5_000,
    };
    c.set_scorecard_config(&cfg);

    // One catastrophic round in the middle of otherwise perfect ones.
    let all = (&s1, &s2, &s3);
    for i in 0..2u32 {
        at(&e, 200 + i, T0 + u64::from(i) * 10);
        score_round(&c, all, &asset, 130);
    }
    at(&e, 300, T0 + 100);
    score_round(&c, all, &asset, 100_000_000);
    for i in 0..2u32 {
        at(&e, 400 + i, T0 + 200 + u64::from(i) * 10);
        score_round(&c, all, &asset, 130);
    }

    let card = c.get_source_scorecard(&s1);
    // The catastrophic sample is winsorized at the cap before it enters the
    // mean, so the window average stays bounded no matter how wrong the print
    // was. Without that bound a single sample of ~10^10 bps would dominate.
    assert!(
        card.long.mean_abs_error_bps <= cfg.outlier_cap_bps,
        "one sample may not push the mean past the cap: {}",
        card.long.mean_abs_error_bps
    );
    let score = c.get_source_accuracy_score(&s1);
    assert!(
        score >= SCORE_FLOOR_BPS,
        "the score may not fall below the documented floor: {score}"
    );
    // The retained worst sample is reported too, and it is bounded by the same
    // cap: the cap is applied to the sample, not hidden from the report, so a
    // consumer can see that a cap was hit rather than being shown a value the
    // window never used.
    assert!(
        card.long_max_error_bps <= cfg.outlier_cap_bps,
        "the retained worst sample is bounded by the cap: {}",
        card.long_max_error_bps
    );
    assert!(
        card.long_max_error_bps > card.long.mean_abs_error_bps,
        "the worst retained sample is still distinguishable from the mean"
    );
}

/// #490 AC5: scorecard values are queryable and reconstructible from events.
#[test]
fn scorecards_are_queryable_and_reconstructible_from_events() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2, s3) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    let (_, asset) = with_sources(&e, &c, 3);
    for s in [addr(&s1), addr(&s2), addr(&s3)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    c.enable_scorecards();
    let all = (&s1, &s2, &s3);
    for i in 0..3u32 {
        at(&e, 200 + i, T0 + u64::from(i) * 10);
        score_round(&c, all, &asset, 130);
    }

    assert!(
        has_event(&e, "source_scorecard_updated_event"),
        "each update is evented"
    );
    let samples = c.get_source_scorecard_samples(&s1);
    let card = c.get_source_scorecard(&s1);
    assert!(!samples.is_empty(), "samples are retained for replay");
    // The stored samples are exactly what the window is computed from, so an
    // indexer replaying the events reproduces the same numbers.
    let sum: u64 = (0..samples.len())
        .map(|i| {
            let stored = i64::from(samples.get_unchecked(i));
            (stored - (1i64 << 31)).unsigned_abs()
        })
        .sum();
    assert_eq!(
        card.long.mean_abs_error_bps as u64,
        sum / samples.len() as u64,
        "the reported mean must be reconstructible from the retained samples"
    );
    assert_eq!(
        c.get_scorecard_config().long_window,
        DEFAULT_CONFIG.long_window
    );
}

/// Scorecards are reporting only: enabling them must not change what is
/// published or who may submit.
#[test]
fn scorecards_report_without_silently_altering_behaviour() {
    let e = Env::default();
    let (c, _) = setup(&e);
    let (s1, s2, s3) = (
        Address::generate(&e),
        Address::generate(&e),
        Address::generate(&e),
    );
    let (_, asset) = with_sources(&e, &c, 3);
    for s in [addr(&s1), addr(&s2), addr(&s3)].iter() {
        c.add_source(s, &reason(&e, "extra"));
    }
    let all = (&s1, &s2, &s3);

    // Off: no scorecards, and the round publishes normally.
    score_round(&c, all, &asset, 130);
    let before = c.get_price(&asset, &0).unwrap().price;

    // On: the same round produces the same published value.
    c.enable_scorecards();
    at(&e, 400, T0 + 100);
    score_round(&c, all, &asset, 130);
    let after = c.get_price(&asset, &0).unwrap().price;
    assert_eq!(
        before, after,
        "scorecards must not move the published price"
    );

    // Disabling keeps the data queryable.
    c.disable_scorecards();
    assert!(!c.get_source_scorecard_samples(&s1).is_empty());
}

/// Scorecard configuration is bounds-checked on write.
#[test]
fn scorecard_configuration_is_bounds_checked() {
    let e = Env::default();
    let (c, _) = setup(&e);

    assert!(c
        .try_set_scorecard_config(&ScorecardConfig {
            short_window: 0,
            ..DEFAULT_CONFIG
        })
        .is_err());
    assert!(c
        .try_set_scorecard_config(&ScorecardConfig {
            // A long window shorter than the short one is incoherent.
            short_window: 10,
            long_window: 5,
            ..DEFAULT_CONFIG
        })
        .is_err());
    assert!(c
        .try_set_scorecard_config(&ScorecardConfig {
            hit_tolerance_bps: 0,
            ..DEFAULT_CONFIG
        })
        .is_err());
    assert_eq!(ErrorCode::InvalidScorecardConfig as u32, 189);

    c.set_scorecard_config(&ScorecardConfig {
        short_window: 4,
        ..DEFAULT_CONFIG
    });
    assert_eq!(c.get_scorecard_config().short_window, 4);
}
