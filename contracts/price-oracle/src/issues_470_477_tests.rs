#![cfg(test)]
//! Tests for #470 (governance surface), #472 (freshness-weighted median),
//! #474 (per-asset policy) and #477 (TWAP observation cardinality).

use soroban_sdk::{testutils::Address as _, testutils::Events as _, Address, Env, String};

use crate::freshness_weight::{cap_weights, weight_for_age, weighted_median};
use crate::storage::compute_median;
use crate::test_helpers::*;
use crate::types::{FreshnessCurve, PolicyOverride};
use crate::{Asset, TwapMethod};

// ── #477 TWAP cardinality ────────────────────────────────────────────────────

fn twap_env(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address) {
    ledger_default(e, 1, 5);
    let (client, _admin) = setup_contract(e);
    client.set_min_sources_required(&1u32);
    client.set_max_history_length(&200u32);
    let source = register_test_source(e, &client, "S");
    let asset = register_test_asset(e, &client);
    (client, source, asset)
}
use crate::PriceOracleContractClient;

#[test]
fn twap_below_cardinality_floor_is_rejected() {
    let e = Env::default();
    let (client, source, asset) = twap_env(&e);
    submit_test_price(&client, &source, &asset, 100, 5);
    client.set_twap_min_cardinality(&3u32);
    assert!(client
        .try_get_twap_ex(&asset, &1u32, &TwapMethod::Arithmetic)
        .is_err());
    client.set_twap_min_cardinality(&1u32);
    let r = client
        .get_twap_ex(&asset, &1u32, &TwapMethod::Arithmetic)
        .unwrap();
    assert_eq!(r.cardinality, 1);
    assert!(r.concentrated);
}

#[test]
fn twap_floor_bounds_are_enforced() {
    let e = Env::default();
    let (client, _s, _a) = twap_env(&e);
    assert_eq!(client.get_twap_min_cardinality(), 1);
    assert!(client.try_set_twap_min_cardinality(&0u32).is_err());
    assert!(client.try_set_twap_min_cardinality(&65u32).is_err());
    client.set_twap_min_cardinality(&64u32);
    assert_eq!(client.get_twap_min_cardinality(), 64);
}

#[test]
fn single_ledger_push_cannot_move_twap_materially() {
    let e = Env::default();
    let (client, source, asset) = twap_env(&e);
    submit_test_price(&client, &source, &asset, 100, 5);
    ledger_default(&e, 50, 250);
    submit_test_price(&client, &source, &asset, 1_000, 250);

    let r = client
        .get_twap_ex(&asset, &50u32, &TwapMethod::Arithmetic)
        .unwrap();
    // The push holds 1 of 50 ledgers: 49 * 100 + 1 * 1000 = 5_900 / 50.
    assert_eq!(r.price, 118);
    assert_eq!(r.cardinality, 2);
    assert_eq!(r.max_weight_bps, 9_800);
    assert!(!r.concentrated);
}

#[test]
fn observation_spam_does_not_add_weight() {
    let e = Env::default();
    let (client, source, asset) = twap_env(&e);
    submit_test_price(&client, &source, &asset, 100, 5);
    ledger_default(&e, 25, 125);
    for _ in 0..5 {
        submit_test_price(&client, &source, &asset, 100, 125);
    }
    ledger_default(&e, 50, 250);
    submit_test_price(&client, &source, &asset, 100, 250);

    let r = client
        .get_twap_ex(&asset, &50u32, &TwapMethod::Arithmetic)
        .unwrap();
    // One observation per ledger regardless of submission count.
    assert!(r.cardinality <= 3);
    assert_eq!(r.price, 100);
}

// ── #472 freshness-weighted median ───────────────────────────────────────────

#[test]
fn equal_weights_equal_plain_median() {
    let e = Env::default();
    for prices in [[100i128, 300, 200, 400], [100, 300, 200, 500]] {
        let mut v = soroban_sdk::Vec::new(&e);
        for p in prices {
            v.push_back(p);
        }
        assert_eq!(weighted_median(&prices, &[7, 7, 7, 7]), compute_median(&v));
    }
    let odd = [5i128, 1, 3];
    let mut v = soroban_sdk::Vec::new(&e);
    for p in odd {
        v.push_back(p);
    }
    assert_eq!(weighted_median(&odd, &[1, 1, 1]), compute_median(&v));
}

#[test]
fn stale_outlier_cannot_outvote_fresh_quorum() {
    let curve = FreshnessCurve {
        window_secs: 300,
        min_weight: 100,
    };
    let fresh = weight_for_age(0, &curve);
    let stale = weight_for_age(10_000, &curve);
    assert_eq!(fresh, 1000);
    assert_eq!(stale, 100);
    let mut w = [fresh, fresh, fresh, stale];
    cap_weights(&mut w);
    assert_eq!(weighted_median(&[100, 100, 100, 900], &w), 100);
    // Even two fresh values beat two stale ones on the same footing.
    let mut w = [fresh, fresh, stale, stale];
    cap_weights(&mut w);
    assert_eq!(weighted_median(&[100, 100, 900, 900], &w), 100);
}

#[test]
fn no_single_weight_exceeds_half_of_total() {
    let mut w = [1000u32, 100, 100];
    cap_weights(&mut w);
    let total: u32 = w.iter().sum();
    assert!(w.iter().all(|x| x * 2 <= total));
    let mut w = [1000u32, 1000, 1000, 100, 100];
    cap_weights(&mut w);
    let total: u32 = w.iter().sum();
    assert!(w.iter().all(|x| x * 2 <= total));
}

#[test]
fn curve_bounds_and_defaults() {
    let e = Env::default();
    let (client, _s, asset) = twap_env(&e);
    let d = client.get_freshness_curve(&asset);
    assert_eq!((d.window_secs, d.min_weight), (300, 100));
    assert!(client
        .try_set_freshness_curve(&asset, &0u64, &100u32)
        .is_err());
    assert!(client
        .try_set_freshness_curve(&asset, &100_000u64, &100u32)
        .is_err());
    assert!(client
        .try_set_freshness_curve(&asset, &60u64, &0u32)
        .is_err());
    assert!(client
        .try_set_freshness_curve(&asset, &60u64, &1001u32)
        .is_err());
    client.set_freshness_curve(&asset, &60u64, &50u32);
    assert_eq!(client.get_freshness_curve(&asset).window_secs, 60);
}

#[test]
fn weighted_method_reports_raw_and_weighted_values() {
    let e = Env::default();
    ledger_default(&e, 1, 1_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&e, &client);
    let s1 = register_test_source(&e, &client, "A");
    let s2 = register_test_source(&e, &client, "B");
    let s3 = register_test_source(&e, &client, "C");
    // s1 submits early (stale), s2 and s3 submit now.
    submit_test_price(&client, &s1, &asset, 900, 1_000);
    ledger_default(&e, 2, 1_400);
    submit_test_price(&client, &s2, &asset, 100, 1_400);
    submit_test_price(&client, &s3, &asset, 110, 1_400);

    let agg = client.get_weighted_aggregate(&asset).unwrap();
    assert_eq!(agg.raw_median, 110);
    assert_eq!(agg.weights.len(), 3);
    assert_eq!(agg.weights.get(0).unwrap(), 100);
    assert_eq!(agg.weights.get(1).unwrap(), 1000);
    // Capped: the stale source keeps its low weight, fresh ones share 50 %.
    let total: u32 = agg.weights.iter().sum();
    assert!(agg.weights.iter().all(|w| w * 2 <= total));
    assert!(agg.weighted_median <= 110);

    client.set_aggregation_method(&4u32);
    submit_test_price(&client, &s2, &asset, 100, 1_400);
    assert!(client.lastprice(&Asset::Stellar(asset.clone())).is_some());
}

// ── #474 per-asset policy ────────────────────────────────────────────────────

fn ov(
    method: Option<u32>,
    min_sources: Option<u32>,
    freshness: Option<u64>,
    dev: Option<u32>,
) -> PolicyOverride {
    PolicyOverride {
        method,
        min_sources,
        freshness_secs: freshness,
        max_deviation_bps: dev,
    }
}

#[test]
fn policy_precedence_asset_then_class_then_global() {
    let e = Env::default();
    let (client, _s, asset) = twap_env(&e);
    let global = client.get_effective_policy(&asset);
    assert_eq!(global.method_layer, 0);
    assert_eq!(global.min_sources_layer, 0);
    assert_eq!(global.freshness_secs, 0);

    client.set_asset_class(&asset, &Some(7u32));
    client.set_class_policy(&7u32, &Some(ov(Some(1), Some(3), Some(600), None)));
    let p = client.get_effective_policy(&asset);
    assert_eq!((p.method, p.method_layer), (1, 1));
    assert_eq!((p.min_sources, p.min_sources_layer), (3, 1));
    assert_eq!((p.freshness_secs, p.freshness_layer), (600, 1));
    assert_eq!(p.max_deviation_layer, 0);

    client.set_asset_policy(&asset, &Some(ov(Some(2), None, None, Some(500))));
    let p = client.get_effective_policy(&asset);
    assert_eq!((p.method, p.method_layer), (2, 2));
    assert_eq!((p.min_sources, p.min_sources_layer), (3, 1));
    assert_eq!((p.max_deviation_bps, p.max_deviation_layer), (500, 2));

    // Clearing falls back through the documented chain.
    client.set_asset_policy(&asset, &None);
    assert_eq!(client.get_effective_policy(&asset).method_layer, 1);
    client.set_class_policy(&7u32, &None);
    assert_eq!(client.get_effective_policy(&asset).method_layer, 0);
}

#[test]
fn invalid_policy_rejected_at_write() {
    let e = Env::default();
    let (client, _s, asset) = twap_env(&e);
    for bad in [
        ov(Some(5), None, None, None),
        ov(None, Some(0), None, None),
        ov(None, None, Some(0), None),
        ov(None, None, Some(604_801), None),
        ov(None, None, None, Some(0)),
        ov(None, None, None, Some(10_001)),
    ] {
        assert!(client
            .try_set_asset_policy(&asset, &Some(bad.clone()))
            .is_err());
        assert!(client.try_set_class_policy(&1u32, &Some(bad)).is_err());
    }
    assert!(client.get_asset_policy(&asset).is_none());
    let unregistered = Address::generate(&e);
    assert!(client
        .try_set_asset_policy(&unregistered, &Some(ov(None, Some(2), None, None)))
        .is_err());
}

#[test]
fn policy_quorum_governs_aggregation_and_does_not_rewrite_history() {
    let e = Env::default();
    ledger_default(&e, 1, 1_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&e, &client);
    let s1 = register_test_source(&e, &client, "A");
    let s2 = register_test_source(&e, &client, "B");

    submit_test_price(&client, &s1, &asset, 100, 1_000);
    let before = client.lastprice(&Asset::Stellar(asset.clone())).unwrap();
    assert_eq!(before.price, 100);

    // Raising the quorum does not reinterpret the aggregate already stored.
    client.set_asset_policy(&asset, &Some(ov(None, Some(2), None, None)));
    assert_eq!(
        client.lastprice(&Asset::Stellar(asset.clone())).unwrap(),
        before
    );

    // A second source satisfies the stricter quorum for the next aggregation.
    ledger_default(&e, 2, 1_005);
    submit_test_price(&client, &s2, &asset, 200, 1_005);
    assert_eq!(
        client
            .lastprice(&Asset::Stellar(asset.clone()))
            .unwrap()
            .price,
        150
    );
}

#[test]
fn policy_deviation_bound_filters_outliers() {
    let e = Env::default();
    ledger_default(&e, 1, 1_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    client.set_aggregation_method(&1u32); // mean, so outliers would show
    let asset = register_test_asset(&e, &client);
    let s1 = register_test_source(&e, &client, "A");
    let s2 = register_test_source(&e, &client, "B");
    let s3 = register_test_source(&e, &client, "C");
    client.set_asset_policy(&asset, &Some(ov(None, None, None, Some(1_000))));
    submit_test_price(&client, &s1, &asset, 100, 1_000);
    submit_test_price(&client, &s2, &asset, 102, 1_000);
    submit_test_price(&client, &s3, &asset, 10_000, 1_000);
    assert_eq!(
        client
            .lastprice(&Asset::Stellar(asset.clone()))
            .unwrap()
            .price,
        101
    );
}

#[test]
fn policy_changes_emit_events() {
    let e = Env::default();
    let (client, _s, asset) = twap_env(&e);
    client.set_asset_policy(&asset, &Some(ov(Some(1), None, None, None)));
    assert!(!e.events().all().events().is_empty());
    client.set_asset_class(&asset, &Some(1u32));
    assert!(!e.events().all().events().is_empty());
    client.set_class_policy(&1u32, &Some(ov(None, Some(2), None, None)));
    assert!(!e.events().all().events().is_empty());
}

// ── #470 governance surface ──────────────────────────────────────────────────

fn first_auth_is(e: &Env, expected: &Address) {
    let auths = e.auths();
    assert!(!auths.is_empty());
    assert_eq!(&auths[0].0, expected);
}

#[test]
fn governor_cannot_change_its_own_authority() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let governor = Address::generate(&e);
    client.set_external_governor(&governor);
    first_auth_is(&e, &admin);
    client.allow_governor_op(&String::from_str(&e, "x"));
    first_auth_is(&e, &admin);
    client.disallow_governor_op(&String::from_str(&e, "x"));
    first_auth_is(&e, &admin);
    client.reauthorize_governor();
    first_auth_is(&e, &admin);
    client.clear_external_governor();
    first_auth_is(&e, &admin);
    assert_ne!(admin, governor);
}

#[test]
fn governor_cannot_be_the_oracle_itself() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    assert!(client.try_set_external_governor(&client.address).is_err());
}

#[test]
fn governor_upgrade_requires_reauthorization() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let governor = Address::generate(&e);
    client.set_external_governor(&governor);
    let op = String::from_str(&e, "update_sources");
    client.allow_governor_op(&op);
    assert!(client.is_governor_op_allowed(&op));
    let epoch = client.get_governor_epoch();

    // Simulated upgrade of the external contract: admin resets trust.
    client.reauthorize_governor();
    assert_eq!(client.get_governor_epoch(), epoch + 1);
    assert!(!client.is_governor_op_allowed(&op));

    client.allow_governor_op(&op);
    assert!(client.is_governor_op_allowed(&op));
}

#[test]
fn replacing_or_clearing_governor_drops_inherited_trust() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let op = String::from_str(&e, "update_sources");
    client.set_external_governor(&Address::generate(&e));
    client.allow_governor_op(&op);
    client.set_external_governor(&Address::generate(&e));
    assert!(!client.is_governor_op_allowed(&op));

    client.allow_governor_op(&op);
    client.clear_external_governor();
    assert!(!client.is_governor_op_allowed(&op));
}

#[test]
fn unreachable_or_hostile_governor_does_not_block_oracle_operation() {
    let e = Env::default();
    ledger_default(&e, 1, 1_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    // A governor address that is not a contract at all: the oracle never calls it.
    client.set_external_governor(&Address::generate(&e));
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    submit_test_price(&client, &source, &asset, 100, 1_000);
    assert_eq!(
        client
            .lastprice(&Asset::Stellar(asset.clone()))
            .unwrap()
            .price,
        100
    );
}

#[test]
fn admin_outranks_governor_on_overlapping_parameters() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let governor = Address::generate(&e);
    client.set_external_governor(&governor);
    client.allow_governor_op(&String::from_str(&e, "set_min_sources"));
    // The parameter is still changed only under admin authority.
    client.set_min_sources_required(&1u32);
    first_auth_is(&e, &admin);
}
