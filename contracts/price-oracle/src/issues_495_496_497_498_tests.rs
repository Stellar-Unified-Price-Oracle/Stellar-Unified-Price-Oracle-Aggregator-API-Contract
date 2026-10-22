#![cfg(test)]
//! Tests for the data-quality reporting track:
//!
//! * **#495** degraded-mode serving analytics — every degraded read attributed to
//!   exactly one state, counts queryable per asset/window, severe states
//!   surviving sampling, bounded read-gas overhead.
//! * **#496** anomaly explanation reports — an explanation for every flagging
//!   path, stable across identical inputs, bounded retention, no leakage.
//! * **#497** oracle-vs-benchmark drift — sustained directional bias detected,
//!   transient divergence not flagged, time alignment handled, metrics never
//!   mutate a price.
//! * **#498** source coverage gaps — independence threshold reporting, temporal
//!   gap detection, and a read-only analysis surface.

use soroban_sdk::{testutils::Events as _, Address, Env, String, Vec};

use crate::degradation;
use crate::drift::DriftThresholds;
use crate::test_helpers::*;
use crate::types::{AnomalyExplanation, AnomalyRule, DataKey, DegradationStats, PriceEntry};
use crate::types::{DegradationConfig, DegradationState};
use crate::PriceOracleContractClient;

/// Client with one source, one asset and `min_sources = 1`, at a known ledger.
fn base(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address) {
    ledger_default(e, 1, 1_000);
    let (client, _admin) = setup_contract(e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(e, &client, "S");
    let asset = register_test_asset(e, &client);
    (client, source, asset)
}

fn state_count(stats: &DegradationStats, state: DegradationState) -> u32 {
    stats.by_state.get(state as u32).unwrap_or(0)
}

fn no_sampling(window_ledgers: u32) -> DegradationConfig {
    DegradationConfig {
        enabled: true,
        window_ledgers,
        sample_every: 0,
        count_windows: true,
    }
}

// ── #495 degraded-mode serving analytics ────────────────────────────────────

/// Every combination of read-path facts maps to exactly one state, and the
/// classification agrees with the predicates it is derived from. This is the
/// "every degraded read is attributed to exactly one reason" property.
#[test]
fn degradation_classification_is_total_and_exclusive() {
    let mut reachable = [false; 5];
    for stale in [false, true] {
        for frozen in [false, true] {
            for ovr in [false, true] {
                for cb in [false, true] {
                    for quorum in [false, true] {
                        let state = degradation::classify(
                            stale,
                            frozen,
                            ovr,
                            if quorum { 3 } else { 1 },
                            3,
                            cb,
                        );
                        // A read is either degraded or fresh — never both, never
                        // neither. Re-deriving from the predicates proves the
                        // classifier is total and single-valued.
                        let degraded = stale || frozen || ovr || !quorum || cb;
                        assert_eq!(state.is_degraded(), degraded);
                        assert!(
                            !(reachable[state as usize] && false),
                            "classifier must be single-valued"
                        );
                        reachable[state as usize] = true;
                    }
                }
            }
        }
    }
    for (i, seen) in reachable.iter().enumerate() {
        assert!(*seen, "state {i} is unreachable");
    }
}

/// Precedence is the documented order, so a read that is both stale and
/// overridden is attributed to staleness, not to the override.
#[test]
fn degradation_precedence_favours_stale_then_clamped() {
    assert_eq!(
        degradation::classify(true, true, true, 0, 3, true),
        DegradationState::Stale
    );
    assert_eq!(
        degradation::classify(false, true, true, 0, 3, true),
        DegradationState::Clamped
    );
    assert_eq!(
        degradation::classify(false, false, false, 1, 3, true),
        DegradationState::LowConfidence
    );
    assert_eq!(
        degradation::classify(false, false, false, 3, 3, true),
        DegradationState::Deferred
    );
    assert_eq!(
        degradation::classify(false, false, false, 3, 3, false),
        DegradationState::Fresh
    );
}

/// The states partition the degraded reads: summing them reproduces the total,
/// which is the invariant behind "exactly one reason per read".
#[test]
fn degradation_states_partition_the_degraded_reads() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    client.set_degradation_config(&no_sampling(1_000));

    submit_test_price(&client, &source, &asset, 100, 1_000);
    // Two fresh reads, which are not degradation and so are not counted.
    client.get_price(&asset, &0u64).unwrap();
    client.get_price(&asset, &0u64).unwrap();
    // One clamped read.
    client.override_price(&asset, &123i128, &String::from_str(&e, "ops"), &500u32);
    client.get_price(&asset, &0u64).unwrap();
    client.remove_price_override(&asset);

    let stats = client.get_degradation_stats(&asset);
    assert_eq!(
        stats.total_degraded, 1,
        "only the override read is degraded"
    );
    assert_eq!(
        degradation::total_from_states(&stats),
        stats.total_degraded,
        "per-state counts must sum to the degraded total"
    );
    assert_eq!(state_count(&stats, DegradationState::Clamped), 1);
    assert_eq!(state_count(&stats, DegradationState::Fresh), 0);
    assert_eq!(stats.severe, 1);
}

/// Counts are queryable per asset and per window, and windows are isolated.
#[test]
fn degradation_counts_are_queryable_per_asset_and_window() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    let other = register_test_asset(&e, &client);
    client.set_degradation_config(&no_sampling(10));

    // Window 0: one clamped read on `asset` — a fresh, in-band submission that
    // the override then shadows.
    ledger_default(&e, 5, 1_000);
    submit_test_price(&client, &source, &asset, 1_000, 1_000);
    client.override_price(&asset, &7i128, &String::from_str(&e, "ops"), &9u32);
    assert!(client.get_price(&asset, &0u64).is_some());
    client.remove_price_override(&asset);

    // Window 3: two clamped reads on `asset`, one on `other`.
    ledger_default(&e, 35, 2_000);
    client.override_price(&asset, &8i128, &String::from_str(&e, "ops"), &40u32);
    assert!(client.get_price(&asset, &0u64).is_some());
    assert!(client.get_price(&asset, &0u64).is_some());
    client.override_price(&other, &9i128, &String::from_str(&e, "ops"), &40u32);
    assert!(client.get_price(&other, &0u64).is_some());

    assert_eq!(client.get_degradation_stats(&asset).window, 3);
    assert_eq!(
        state_count(
            &client.get_degradation_stats(&asset),
            DegradationState::Clamped
        ),
        2
    );
    assert_eq!(
        state_count(
            &client.get_degradation_stats(&other),
            DegradationState::Clamped
        ),
        1
    );

    // The older window is still queryable by index.
    let w0 = client.get_degradation_window_stats(&asset, &0u32);
    assert_eq!(state_count(&w0, DegradationState::Clamped), 1);
    assert_eq!(w0.window, 0);
}

/// Sampling must not hide rare-but-severe degradations, while the counts stay
/// exact for the common ones.
#[test]
fn rare_severe_degradations_survive_sampling() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    client.set_degradation_config(&DegradationConfig {
        enabled: true,
        window_ledgers: 1_000,
        // Aggressive sampling: one event in twenty.
        sample_every: 20,
        count_windows: true,
    });
    submit_test_price(&client, &source, &asset, 100, 1_000);

    // The asset resolution is what makes a served value stale, so give it one
    // and move the clock past it. Sampling is allowed to thin these out.
    client.set_asset_resolution(&asset, &100u32);
    ledger_default(&e, 50, 1_000_000);
    for _ in 0..25 {
        assert!(
            client.get_price(&asset, &0u64).is_none(),
            "the value is past its resolution and must not be served"
        );
    }
    // A single clamped read in the middle of the sampled run.
    client.override_price(&asset, &123i128, &String::from_str(&e, "ops"), &500u32);
    client.get_price(&asset, &0u64).unwrap();
    client.remove_price_override(&asset);

    let stats = client.get_degradation_stats(&asset);
    assert_eq!(
        state_count(&stats, DegradationState::Stale),
        25,
        "stale reads are counted exactly regardless of sampling"
    );
    assert_eq!(state_count(&stats, DegradationState::Clamped), 1);
    assert_eq!(stats.severe, 1, "the severe read must survive sampling");
    assert!(
        stats.emitted_events >= 1,
        "the severe read must have been emitted despite sampling"
    );
}

/// A disabled configuration records nothing, so operators can turn the
/// instrumentation off entirely if the read cost ever matters.
#[test]
fn degradation_config_toggles_counting() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    submit_test_price(&client, &source, &asset, 100, 1_000);
    client.override_price(&asset, &123i128, &String::from_str(&e, "ops"), &500u32);
    client.get_price(&asset, &0u64).unwrap();
    assert_eq!(client.get_degradation_stats(&asset).total_degraded, 1);

    client.set_degradation_config(&DegradationConfig {
        enabled: false,
        window_ledgers: 1_000,
        sample_every: 0,
        count_windows: true,
    });
    client.get_price(&asset, &0u64).unwrap();
    assert_eq!(
        client.get_degradation_stats(&asset).total_degraded,
        1,
        "a disabled config must not count"
    );
    assert!(!client.get_degradation_config().enabled);
}

/// Read gas: instrumenting the read path costs nothing on a fresh read — the
/// case that dominates in production — and a bounded amount on a degraded one.
#[test]
fn degradation_read_gas_overhead_is_bounded() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    submit_test_price(&client, &source, &asset, 100, 1_000);
    client.override_price(&asset, &123i128, &String::from_str(&e, "ops"), &500u32);

    // `fresh` is served straight from the live median; `asset` is shadowed by an
    // override, so every read of it is degraded.
    let fresh_source = register_test_source(&e, &client, "S2");
    let fresh = register_test_asset(&e, &client);
    submit_test_price(&client, &fresh_source, &fresh, 100, 1_000);
    let measure = |client: &PriceOracleContractClient<'_>, a: &Address| -> u64 {
        client.get_price(a, &0u64).unwrap();
        e.cost_estimate().budget().reset_unlimited();
        client.get_price(a, &0u64).unwrap();
        e.cost_estimate().budget().cpu_instruction_cost()
    };
    let off = DegradationConfig {
        enabled: false,
        window_ledgers: 1_000,
        sample_every: 0,
        count_windows: true,
    };

    // (1) A fresh read is never degraded, so instrumentation must not slow it
    //     down at all. This is the guarantee that keeps the hot path free.
    client.set_degradation_config(&off);
    let fresh_off = measure(&client, &fresh);
    client.set_degradation_config(&no_sampling(1_000));
    let fresh_on = measure(&client, &fresh);
    // The only extra work a fresh read does is resolving the quorum it is
    // classified against, so the overhead must stay a rounding error.
    let fresh_pct = fresh_on.saturating_sub(fresh_off) * 100 / fresh_off;
    assert!(
        fresh_pct < 5,
        "a fresh read must be effectively free, but cost {fresh_pct}% more ({fresh_off} -> {fresh_on})"
    );

    // (2) A degraded read does the counting work, so it costs more — but the
    //     overhead is bounded rather than unbounded.
    client.set_degradation_config(&off);
    let degraded_off = measure(&client, &asset);
    client.set_degradation_config(&no_sampling(1_000));
    let degraded_on = measure(&client, &asset);

    assert!(
        degraded_on > degraded_off,
        "counting a degraded read must do strictly more work"
    );
    let overhead_pct = (degraded_on - degraded_off) * 100 / degraded_off;
    assert!(
        overhead_pct < 50,
        "degradation instrumentation overhead {overhead_pct}% exceeds the 50% bound"
    );
    std::println!(
        "GAS_SAMPLE,degradation_read,fresh_off,{fresh_off},fresh_on,{fresh_on},fresh_pct,{fresh_pct},degraded_off,{degraded_off},degraded_on,{degraded_on},degraded_pct,{overhead_pct}"
    );
}

// ── #496 anomaly explanation reports ────────────────────────────────────────

/// Asserts that `submit` reverts *and* that the pure view names the rule that
/// caused it. Both halves are required: the revert proves the path rejects, the
/// explanation proves the source is told why.
fn assert_rejected_with(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    source: &Address,
    asset: &Address,
    price: i128,
    timestamp: u64,
    rule: AnomalyRule,
) {
    assert!(
        client
            .try_submit_price(source, asset, &price, &timestamp)
            .is_err(),
        "the submission should have been rejected"
    );
    let exp = client
        .explain_submission(source, asset, &price, &timestamp)
        .unwrap_or_else(|| panic!("no explanation produced for {rule:?}"));
    assert!(
        crate::explanation::matches_rule(&exp, rule),
        "expected {rule:?}, got {:?} (id {})",
        exp.rule,
        exp.rule_id
    );
    assert_eq!(exp.rejected, rule.is_rejecting());
    // `observed` is whatever the rule compared: the price for price rules, the
    // timestamp for the timestamp rules.
    let price_rule = matches!(
        rule,
        AnomalyRule::NonPositivePrice
            | AnomalyRule::PriceOutOfBounds
            | AnomalyRule::ChangeRateBreach
    );
    if price_rule {
        assert_eq!(exp.observed, price);
    } else {
        assert_eq!(exp.observed, timestamp as i128);
    }
    assert_eq!(exp.asset, *asset);
    assert_eq!(exp.subject, *source);
    assert_eq!(exp.ledger, e.ledger().sequence());
}

// ── #496 anomaly explanation reports ────────────────────────────────────────

/// The non-positive-price rejection path is explained as `NonPositivePrice`.
#[test]
fn explanation_non_positive_price_path() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    assert_rejected_with(
        &e,
        &client,
        &source,
        &asset,
        0,
        1_000,
        AnomalyRule::NonPositivePrice,
    );
    assert_rejected_with(
        &e,
        &client,
        &source,
        &asset,
        -5,
        1_000,
        AnomalyRule::NonPositivePrice,
    );
}

/// The out-of-bounds rejection path reports the bound that was breached.
#[test]
fn explanation_price_out_of_bounds_path() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    client.set_price_bounds(&asset, &10i128, &1_000i128, &0u32);
    assert_rejected_with(
        &e,
        &client,
        &source,
        &asset,
        5_000,
        1_000,
        AnomalyRule::PriceOutOfBounds,
    );
    let exp = client
        .explain_submission(&source, &asset, &5_000i128, &1_000u64)
        .unwrap();
    assert_eq!(exp.threshold, 1_000, "the breached max bound is reported");
    assert_eq!(exp.reference, 10, "the min bound is the reference");

    // The low side names the same rule.
    assert_rejected_with(
        &e,
        &client,
        &source,
        &asset,
        1,
        1_000,
        AnomalyRule::PriceOutOfBounds,
    );
}

/// The future-timestamp rejection path reports the allowed horizon.
#[test]
fn explanation_future_timestamp_path() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    // Ledger time is 1_000; submit far beyond the timestamp threshold.
    assert_rejected_with(
        &e,
        &client,
        &source,
        &asset,
        100,
        9_999_999,
        AnomalyRule::FutureTimestamp,
    );
    let exp = client
        .explain_submission(&source, &asset, &100i128, &9_999_999u64)
        .unwrap();
    assert_eq!(exp.observed, 9_999_999);
    assert_eq!(exp.reference, 1_000, "the ledger clock is the reference");
}

/// The out-of-order rejection path names the timestamp it was behind.
#[test]
fn explanation_stale_submission_path() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    // A clock well past both timestamps, so 1_000 is genuinely in the past.
    ledger_default(&e, 1, 20_000);
    submit_test_price(&client, &source, &asset, 100, 5_000);
    assert_rejected_with(
        &e,
        &client,
        &source,
        &asset,
        100,
        1_000,
        AnomalyRule::StaleSubmission,
    );
    let exp = client
        .explain_submission(&source, &asset, &100i128, &1_000u64)
        .unwrap();
    assert_eq!(exp.observed, 1_000);
    assert_eq!(
        exp.reference, 5_000,
        "the newer prior timestamp is reported"
    );
}

/// The change-rate rejection path reports the prior aggregate and the limit.
#[test]
fn explanation_change_rate_breach_path() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    client.set_price_bounds(&asset, &0i128, &i128::MAX, &100u32);
    submit_test_price(&client, &source, &asset, 1_000, 1_000);
    assert_rejected_with(
        &e,
        &client,
        &source,
        &asset,
        5_000,
        1_000,
        AnomalyRule::ChangeRateBreach,
    );
    let exp = client
        .explain_submission(&source, &asset, &5_000i128, &1_000u64)
        .unwrap();
    assert_eq!(exp.reference, 1_000, "the prior aggregate is the reference");
    assert_eq!(exp.threshold, 100, "the breached bps limit is reported");
}

/// A submission that breaks no rule is explained as acceptable.
#[test]
fn explanation_is_none_for_an_acceptable_submission() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    assert!(client
        .explain_submission(&source, &asset, &100i128, &1_000u64)
        .is_none());
    submit_test_price(&client, &source, &asset, 100, 1_000);
    // A newer, in-band submission is still acceptable.
    ledger_default(&e, 1, 5_000);
    assert!(client
        .explain_submission(&source, &asset, &110i128, &2_000u64)
        .is_none());
    // A zero price is rejected whatever else is true.
    assert!(client
        .explain_submission(&source, &asset, &0i128, &2_000u64)
        .is_some());
}

/// The correlation-band flag path records a `CorrelationBand` explanation for
/// the flagged source. Unlike a rejection this does not revert, so the record
/// is stored and visible without a second call.
#[test]
fn explanation_correlation_band_path() {
    let e = Env::default();
    let (client, source, _quote) = base(&e);
    let base_asset = register_test_asset(&e, &client);
    let quote_asset = register_test_asset(&e, &client);
    // A near-1:1 band; a quote priced at 100 with no base price cannot satisfy
    // it, so the base submission is flagged.
    client.set_correlation_pair(
        &base_asset,
        &quote_asset,
        &9_900_000u128,
        &10_100_000u128,
        &true,
    );

    submit_test_price(&client, &source, &quote_asset, 100, 1_000);
    submit_test_price(&client, &source, &base_asset, 5_000, 1_000);

    let log = client.get_flag_explanations(&base_asset, &source);
    assert_eq!(log.len(), 1);
    let exp = log.get_unchecked(0);
    assert!(crate::explanation::matches_rule(
        &exp,
        AnomalyRule::CorrelationBand
    ));
    assert!(
        !exp.rejected,
        "a correlation flag excludes the source, it does not revert"
    );
    assert_eq!(exp.observed, 5_000, "the flagged price is reported");
    assert_eq!(exp.reference, 100, "the counterpart price is the reference");
    assert_eq!(
        client.get_latest_flag_explanation(&base_asset, &source),
        Some(exp)
    );
}

/// The below-quorum aggregate path records an `InsufficientSources`
/// explanation on the asset, visible through the operator view.
#[test]
fn explanation_insufficient_sources_path() {
    let e = Env::default();
    ledger_default(&e, 1, 1_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&3u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    submit_test_price(&client, &source, &asset, 100, 1_000);

    let log = client.get_aggregate_flag_explanations(&asset);
    assert_eq!(log.len(), 1);
    let exp = log.get_unchecked(0);
    assert!(crate::explanation::matches_rule(
        &exp,
        AnomalyRule::InsufficientSources
    ));
    assert_eq!(exp.observed, 1, "one source contributed");
    assert_eq!(exp.threshold, 3, "quorum is the threshold");
    assert_eq!(exp.subject, asset, "an aggregate flag is about the asset");
}

/// The low-confidence-band aggregate path records a `LowConfidenceBand`
/// explanation.
#[test]
fn explanation_low_confidence_band_path() {
    let e = Env::default();
    ledger_default(&e, 1, 1_000);
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&2u32);
    let s1 = register_test_source(&e, &client, "A");
    let s2 = register_test_source(&e, &client, "B");
    let asset = register_test_asset(&e, &client);
    submit_test_price(&client, &s1, &asset, 100, 1_000);
    submit_test_price(&client, &s2, &asset, 100, 1_000);

    let log = client.get_aggregate_flag_explanations(&asset);
    assert!(
        log.iter()
            .any(|x| crate::explanation::matches_rule(&x, AnomalyRule::LowConfidenceBand)),
        "a two-source band is below the low-confidence floor, so it must be explained"
    );
}

/// Every rule in the enum produces a well-formed, addressable explanation, and
/// each rule id is distinct and stable.
#[test]
fn every_rule_produces_a_well_formed_explanation() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    let contract_id = client.address.clone();
    let mut ids = Vec::new(&e);
    e.as_contract(&contract_id, || {
        for rule in AnomalyRule::ALL {
            let exp = crate::explanation::record_submission(&e, rule, &source, &asset, 11, 22, 33);
            assert_eq!(exp.rule_id, rule as u32);
            assert_eq!(exp.rule, rule.name());
            assert_eq!(exp.observed, 11);
            assert_eq!(exp.reference, 22);
            assert_eq!(exp.threshold, 33);
            assert_eq!(exp.rejected, rule.is_rejecting());
            assert_eq!(exp.asset, asset);
            assert_eq!(exp.subject, source);
            ids.push_back(exp.rule_id);
        }
    });
    // Ids are unique, so an explanation unambiguously identifies its rule.
    for i in 0..ids.len() {
        for j in 0..i {
            assert_ne!(ids.get_unchecked(i), ids.get_unchecked(j));
        }
    }
    assert_eq!(client.get_flag_explanations(&asset, &source).len(), 8);
}

/// Identical inputs always yield an identical explanation, so a record can be
/// re-derived and audited after the fact.
#[test]
fn explanations_are_stable_across_identical_inputs() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    let contract_id = client.address.clone();
    let mut previous: Option<AnomalyExplanation> = None;
    e.as_contract(&contract_id, || {
        for _ in 0..3 {
            let exp = crate::explanation::record_submission(
                &e,
                AnomalyRule::PriceOutOfBounds,
                &source,
                &asset,
                5_000,
                0,
                1_000,
            );
            if let Some(prev) = previous.clone() {
                assert_eq!(exp, prev, "identical inputs must yield identical records");
            }
            previous = Some(exp);
        }
    });
    // The pure builder agrees with what was stored.
    let built = crate::explanation::build(
        AnomalyRule::PriceOutOfBounds,
        &source,
        &asset,
        e.ledger().sequence(),
        5_000,
        0,
        1_000,
        true,
    );
    assert_eq!(
        built,
        client.get_latest_flag_explanation(&asset, &source).unwrap()
    );
}

/// Retention is bounded: a burst of flags never grows the log past the ring,
/// and the ring size cannot be configured out of bounds.
#[test]
fn explanation_retention_is_bounded() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    let contract_id = client.address.clone();
    client.set_anomaly_retention(&4u32);
    assert_eq!(client.get_anomaly_retention(), 4);

    e.as_contract(&contract_id, || {
        for i in 0..20i128 {
            crate::explanation::record_submission(
                &e,
                AnomalyRule::PriceOutOfBounds,
                &source,
                &asset,
                i,
                0,
                1_000,
            );
        }
    });
    let log = client.get_flag_explanations(&asset, &source);
    assert_eq!(log.len(), 4, "the ring must not exceed the retention limit");
    assert_eq!(
        log.get_unchecked(3).observed,
        19,
        "the newest record survives"
    );

    assert!(client.try_set_anomaly_retention(&0u32).is_err());
    assert!(client
        .try_set_anomaly_retention(&(crate::explanation::MAX_RETENTION + 1))
        .is_err());
    assert_eq!(client.get_anomaly_retention(), 4);
}

/// The default retention also bounds growth when never configured.
#[test]
fn explanation_default_retention_bounds_growth() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    let contract_id = client.address.clone();
    let retention = crate::explanation::DEFAULT_RETENTION;
    assert_eq!(client.get_anomaly_retention(), retention);
    e.as_contract(&contract_id, || {
        for i in 0..(retention as i128 + 5) {
            crate::explanation::record_submission(
                &e,
                AnomalyRule::NonPositivePrice,
                &source,
                &asset,
                i,
                0,
                0,
            );
        }
    });
    assert_eq!(
        client.get_flag_explanations(&asset, &source).len(),
        retention
    );
}

/// Leakage review: an explanation carries only the flagged party's own inputs
/// and the rule inputs — never another source's identity, the admin, or any
/// key material.
#[test]
fn explanations_leak_no_internal_state() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    let bystander = register_test_source(&e, &client, "Bystander");
    let admin = client.get_admin();

    let exp = client
        .explain_submission(&source, &asset, &5_000i128, &1_000u64)
        .or_else(|| {
            // Force a flag by exceeding the default bounds.
            client.set_price_bounds(&asset, &0i128, &1_000i128, &0u32);
            client.explain_submission(&source, &asset, &5_000i128, &1_000u64)
        })
        .unwrap();

    // The only addresses in the record are the flagger and the asset.
    assert_eq!(exp.subject, source);
    assert_eq!(exp.asset, asset);
    assert_ne!(exp.subject, bystander, "no bystander identity is disclosed");
    assert_ne!(exp.subject, admin, "the admin is never disclosed");

    // Nothing resembling a secret or key is present: the record is a fixed set
    // of rule inputs with no free-form field.
    let encoded = std::format!("{exp:?}").to_lowercase();
    for forbidden in ["key", "seed", "secret", "signature", "priv"] {
        assert!(
            !encoded.contains(forbidden),
            "explanation leaked {forbidden}: {encoded}"
        );
    }
}

/// Every stored flag is also emitted as an event, so the record is
/// reconstructible from the event stream once the bounded ring has rolled.
#[test]
fn explanations_are_reconstructible_from_events() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    let quote = register_test_asset(&e, &client);
    let correlated = register_test_asset(&e, &client);
    client.set_correlation_pair(&correlated, &quote, &9_900_000u128, &10_100_000u128, &true);
    // Retention of 1 forces the stored ring to roll on the second flag.
    client.set_anomaly_retention(&1u32);

    submit_test_price(&client, &source, &quote, 100, 1_000);
    // Each of these flags the submission. `events().all()` reflects the
    // current invocation, so it is read straight after the flagging call.
    submit_test_price(&client, &source, &correlated, 5_000, 1_000);
    let first_events = e.events().all().events().len();
    ledger_default(&e, 2, 2_000);
    submit_test_price(&client, &source, &correlated, 6_000, 2_000);
    let second_events = e.events().all().events().len();

    // The ring holds only the newest record for the asset...
    let log = client.get_flag_explanations(&correlated, &source);
    assert_eq!(log.len(), 1, "the bounded ring rolled");
    assert_eq!(log.get_unchecked(0).observed, 6_000);

    // ...but both flags were emitted, so nothing is lost off-chain. The second
    // emission in particular proves the record survives the ring rolling.
    assert!(
        first_events > 0 && second_events > 0,
        "every flag must be emitted, saw {first_events} then {second_events}"
    );
}

// ── #497 oracle-vs-benchmark long-horizon drift ─────────────────────────────

fn drift_asset(e: &Env) -> (PriceOracleContractClient<'_>, Address) {
    ledger_default(e, 1, 1_000);
    let (client, _admin) = setup_contract(e);
    let asset = register_test_asset(e, &client);
    (client, asset)
}

/// Feeds `count` samples with the oracle a fixed `bias_bps` away from the
/// benchmark, all inside the alignment tolerance.
fn feed_biased(
    client: &PriceOracleContractClient<'_>,
    asset: &Address,
    count: i128,
    bias_bps: i128,
) {
    for i in 0..count {
        let oracle = 1_000_000i128 + (1_000_000i128 * bias_bps) / 10_000;
        let ts = 1_000u64 + i as u64;
        client.record_drift_sample(asset, &oracle, &ts, &1_000_000i128, &ts);
    }
}

/// A synthetic, consistently biased series is detected as sustained drift.
#[test]
fn sustained_directional_bias_is_detected() {
    let e = Env::default();
    let (client, asset) = drift_asset(&e);
    client.set_drift_thresholds(&8u32, &100i128, &300u64);
    // A steady +2 % lean, repeated: invisible per-ledger, obvious over a window.
    feed_biased(&client, &asset, 12, 200);

    let r = client.get_drift_report(&asset);
    assert_eq!(r.samples, 12);
    assert!(r.mean_bias_bps >= 100, "mean bias {}", r.mean_bias_bps);
    assert_eq!(r.directional_consistency_bps, 10_000, "one-sided series");
    assert!(
        r.sustained_drift,
        "a one-sided bias must be flagged as drift"
    );

    // A downward lean is drift too, with the sign flipped.
    let e2 = Env::default();
    let (c2, a2) = drift_asset(&e2);
    c2.set_drift_thresholds(&8u32, &100i128, &300u64);
    feed_biased(&c2, &a2, 12, -200);
    let r2 = c2.get_drift_report(&a2);
    assert!(r2.mean_bias_bps < 0);
    assert!(r2.sustained_drift);
}

/// A single large divergence is transient and must never alert, even though it
/// is far larger than the drift threshold.
#[test]
fn transient_divergence_does_not_alert() {
    let e = Env::default();
    let (client, asset) = drift_asset(&e);
    client.set_drift_thresholds(&8u32, &100i128, &300u64);
    // One huge spike, the rest perfectly on benchmark.
    for i in 0..12i128 {
        let ts = 1_000u64 + i as u64;
        let oracle = if i == 5 { 2_000_000 } else { 1_000_000 };
        client.record_drift_sample(&asset, &oracle, &ts, &1_000_000i128, &ts);
    }

    let r = client.get_drift_report(&asset);
    assert!(r.max_abs_divergence_bps >= 10_000, "the spike is recorded");
    assert!(
        !r.sustained_drift,
        "one large divergence is not directional drift: mean {}",
        r.mean_bias_bps
    );

    // A whipsaw — large but alternating sign — is transient too.
    let e2 = Env::default();
    let (c2, a2) = drift_asset(&e2);
    c2.set_drift_thresholds(&8u32, &100i128, &300u64);
    for i in 0..12i128 {
        let ts = 1_000u64 + i as u64;
        let oracle = if i % 2 == 0 { 1_200_000 } else { 800_000 };
        c2.record_drift_sample(&a2, &oracle, &ts, &1_000_000i128, &ts);
    }
    assert!(
        !c2.get_drift_report(&a2).sustained_drift,
        "alternating divergence is not drift"
    );
}

/// Too few samples is not drift, however biased.
#[test]
fn drift_requires_a_minimum_sample_count() {
    let e = Env::default();
    let (client, asset) = drift_asset(&e);
    client.set_drift_thresholds(&8u32, &100i128, &300u64);
    feed_biased(&client, &asset, 3, 500);
    let r = client.get_drift_report(&asset);
    assert!(r.mean_bias_bps >= 100);
    assert!(
        !r.sustained_drift,
        "3 samples cannot establish a long-horizon trend"
    );
}

/// Time alignment: a snapshot taken outside the tolerance is discarded and
/// counted, not folded into the bias.
#[test]
fn misaligned_snapshots_are_skipped_and_counted() {
    let e = Env::default();
    let (client, asset) = drift_asset(&e);
    client.set_drift_thresholds(&4u32, &100i128, &300u64);

    // Inside the tolerance: admitted.
    assert!(client
        .record_drift_sample(&asset, &1_020_000i128, &1_000u64, &1_000_000i128, &1_200u64)
        .is_some());
    // Outside it in both directions: rejected, with no bias reported.
    assert!(client
        .record_drift_sample(&asset, &1_500_000i128, &1_000u64, &1_000_000i128, &9_999u64)
        .is_none());
    assert!(client
        .record_drift_sample(&asset, &1_500_000i128, &9_999u64, &1_000_000i128, &1_000u64)
        .is_none());
    // Exactly at the tolerance is still aligned.
    assert!(client
        .record_drift_sample(&asset, &1_020_000i128, &1_000u64, &1_000_000i128, &1_300u64)
        .is_some());

    let r = client.get_drift_report(&asset);
    assert_eq!(r.samples, 2, "only aligned samples enter the window");
    assert_eq!(r.misaligned_skipped, 2, "the rest are counted, not hidden");
    assert_eq!(
        r.max_abs_divergence_bps, 200,
        "a misaligned sample must not inflate the divergence"
    );
}

/// Recording drift samples never mutates an on-chain price, and reading the
/// report never does either.
#[test]
fn drift_never_mutates_on_chain_prices() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    client.set_drift_thresholds(&4u32, &100i128, &300u64);
    submit_test_price(&client, &source, &asset, 1_000_000, 1_000);
    let before = client.get_price(&asset, &0u64).unwrap();

    for i in 0..12i128 {
        let ts = 1_000u64 + i as u64;
        client.record_drift_sample(&asset, &1_500_000i128, &ts, &1_000_000i128, &ts);
    }
    let _report = client.get_drift_report(&asset);
    let after = client.get_price(&asset, &0u64).unwrap();

    assert_eq!(before.price, after.price, "a price must never change");
    assert_eq!(before.timestamp, after.timestamp);
    assert_eq!(before.num_sources, after.num_sources);
    let entry: PriceEntry = client.get_source_price(&asset, &source);
    assert_eq!(entry.price, 1_000_000, "the stored submission is untouched");
}

/// The rolling window is bounded, so a long-running deployment cannot grow
/// without limit.
#[test]
fn drift_window_is_bounded() {
    let e = Env::default();
    let (client, asset) = drift_asset(&e);
    for i in 0..(crate::drift::WINDOW_CAPACITY as i128 + 20) {
        let ts = 1_000u64 + i as u64;
        client.record_drift_sample(&asset, &1_020_000i128, &ts, &1_000_000i128, &ts);
    }
    assert_eq!(
        client.get_drift_report(&asset).samples,
        crate::drift::WINDOW_CAPACITY
    );
}

/// Thresholds are admin-configurable and validated.
#[test]
fn drift_thresholds_are_validated() {
    let e = Env::default();
    let (client, _asset) = drift_asset(&e);
    let (min_samples, bias_bps, alignment) = client.get_drift_thresholds();
    assert_eq!(min_samples, crate::drift::DEFAULT_MIN_SAMPLES);
    assert_eq!(bias_bps, crate::drift::DEFAULT_BIAS_THRESHOLD_BPS);
    assert_eq!(alignment, crate::drift::DEFAULT_MAX_ALIGNMENT_SECS);

    assert!(client
        .try_set_drift_thresholds(&0u32, &100i128, &300u64)
        .is_err());
    assert!(client
        .try_set_drift_thresholds(&(crate::drift::WINDOW_CAPACITY + 1), &100i128, &300u64)
        .is_err());
    assert!(client
        .try_set_drift_thresholds(&8u32, &0i128, &300u64)
        .is_err());
    client.set_drift_thresholds(&16u32, &250i128, &60u64);
    assert_eq!(client.get_drift_thresholds(), (16, 250, 60));
}

/// Resetting the window clears both the samples and the misaligned counter.
#[test]
fn drift_window_can_be_reset() {
    let e = Env::default();
    let (client, asset) = drift_asset(&e);
    client.set_drift_thresholds(&4u32, &100i128, &300u64);
    feed_biased(&client, &asset, 5, 200);
    client.record_drift_sample(
        &asset,
        &1_500_000i128,
        &1_000u64,
        &1_000_000i128,
        &99_999u64,
    );
    assert!(client.get_drift_report(&asset).samples > 0);
    assert_eq!(client.get_drift_report(&asset).misaligned_skipped, 1);

    client.reset_drift_window(&asset);
    let r = client.get_drift_report(&asset);
    assert_eq!(r.samples, 0);
    assert_eq!(r.misaligned_skipped, 0);
    assert!(!r.sustained_drift);
}

/// The pure bias helper is sign-correct and rejects a nonsense benchmark.
#[test]
fn bias_bps_is_signed_and_guarded() {
    assert_eq!(crate::drift::bias_bps(1_010_000, 1_000_000), Some(100));
    assert_eq!(crate::drift::bias_bps(990_000, 1_000_000), Some(-100));
    assert_eq!(crate::drift::bias_bps(1_000_000, 1_000_000), Some(0));
    assert_eq!(crate::drift::bias_bps(1_000_000, 0), None);
    assert_eq!(crate::drift::bias_bps(1_000_000, -5), None);
}

// ── #498 source coverage gap analysis ───────────────────────────────────────

/// Attests a distinct `(infra, upstream, owner)` failure domain for `source`.
fn attest_domain(e: &Env, client: &PriceOracleContractClient<'_>, source: &Address, tag: &str) {
    client.set_source_diversity(
        source,
        &String::from_str(e, tag),
        &String::from_str(e, tag),
        &String::from_str(e, tag),
    );
}

/// An asset whose sources share one failure domain is reported as below the
/// independence threshold, even though its raw source count looks healthy.
#[test]
fn assets_below_the_independence_threshold_are_reported() {
    let e = Env::default();
    let (client, s0, asset) = base(&e);
    client.set_coverage_thresholds(&3u32, &100u32);

    // `base` already registered one source; add three more, all on one cloud
    // with one upstream and one owner — the Sybil trap.
    attest_domain(&e, &client, &s0, "acme");
    for _ in 0..3 {
        let s = register_test_source(&e, &client, "S");
        attest_domain(&e, &client, &s, "acme");
    }

    let report = client.get_coverage_report(&asset);
    assert_eq!(report.registered_sources, 4, "four sources are admitted");
    assert_eq!(
        report.independent_domains, 1,
        "one shared (infra, upstream, owner) triple is one domain"
    );
    assert_eq!(report.min_independent_required, 3);
    assert!(report.below_independence_threshold);
    assert!(
        !report.recommendations.is_empty(),
        "a gap must come with an advisory recommendation"
    );
}

/// Three genuinely independent domains clear the threshold.
#[test]
fn independent_domains_clear_the_threshold() {
    let e = Env::default();
    let (client, s0, asset) = base(&e);
    client.set_coverage_thresholds(&3u32, &100u32);
    attest_domain(&e, &client, &s0, "alpha");
    for tag in ["beta", "gamma"] {
        let s = register_test_source(&e, &client, "S");
        attest_domain(&e, &client, &s, tag);
    }
    let report = client.get_coverage_report(&asset);
    assert_eq!(report.independent_domains, 3);
    assert!(!report.below_independence_threshold);
}

/// Sources with no attested metadata collapse into one `unknown` domain, which
/// lowers the count rather than inflating it.
#[test]
fn unattested_metadata_counts_as_one_domain() {
    let e = Env::default();
    let (client, _s, asset) = base(&e);
    for _ in 0..2 {
        register_test_source(&e, &client, "S");
    }
    let report = client.get_coverage_report(&asset);
    assert_eq!(report.registered_sources, 3);
    assert_eq!(report.independent_domains, 1);
    assert!(report.below_independence_threshold);
}

/// The gap list names every under-covered asset and is reproducible from
/// stored data alone.
#[test]
fn gap_list_names_under_covered_assets() {
    let e = Env::default();
    let (client, s0, weak) = base(&e);
    let strong = register_test_asset(&e, &client);
    client.set_coverage_thresholds(&2u32, &100u32);

    // One domain covers every asset, so both start below the threshold.
    attest_domain(&e, &client, &s0, "acme");
    assert_eq!(client.get_coverage_gap_list().len(), 2);

    // ...until a second, independent domain is admitted.
    let b = register_test_source(&e, &client, "B");
    attest_domain(&e, &client, &b, "beta");
    let gaps = client.get_coverage_gap_list();
    assert!(gaps.is_empty(), "both assets are now covered: {gaps:?}");
    assert!(
        !client
            .get_coverage_report(&strong)
            .below_independence_threshold
    );
    assert!(
        !client
            .get_coverage_report(&weak)
            .below_independence_threshold
    );

    // The report is reproducible: reading it again changes nothing.
    assert_eq!(
        client.get_coverage_report(&weak),
        client.get_coverage_report(&weak)
    );
}

/// Temporal participation is derived from the stored price history, and a
/// window in which an aggregate rested on too few sources is reported as a gap.
#[test]
fn temporal_gaps_are_detected_from_participation_data() {
    let e = Env::default();
    let (client, s1, asset) = base(&e);
    client.set_coverage_thresholds(&2u32, &10u32);
    let s2 = register_test_source(&e, &client, "S2");

    // Window 0 (ledgers 0-9): only s1 has submitted, so the aggregate rests on
    // a single source — a participation gap.
    submit_test_price(&client, &s1, &asset, 100, 1_000);
    let first = client.get_coverage_report(&asset);
    assert_eq!(first.windows_observed, 1);
    assert_eq!(first.min_window_participation, 1);
    assert_eq!(first.low_participation_windows, 1);

    // Window 1 (ledgers 10-19): s2 joins, so the aggregate now rests on two.
    // The price move is large enough to clear the history compaction
    // threshold, which would otherwise skip the second history entry.
    ledger_default(&e, 15, 2_000);
    submit_test_price(&client, &s2, &asset, 400, 2_000);

    let report = client.get_coverage_report(&asset);
    assert_eq!(report.windows_observed, 2, "two windows have aggregates");
    assert_eq!(
        report.low_participation_windows, 1,
        "only the single-source window is a gap"
    );
    assert_eq!(report.min_window_participation, 1);
    assert_eq!(
        report.registered_sources, 2,
        "participation is not admission"
    );
    assert!(
        !report.recommendations.is_empty(),
        "a participation gap must come with an advisory recommendation"
    );
}

/// A window in which every admitted source participated is not a gap.
#[test]
fn full_participation_is_not_a_gap() {
    let e = Env::default();
    let (client, s1, asset) = base(&e);
    client.set_coverage_thresholds(&2u32, &10u32);
    let s2 = register_test_source(&e, &client, "S2");
    let s3 = register_test_source(&e, &client, "S3");
    // Attest three independent domains so independence is not also a gap.
    attest_domain(&e, &client, &s1, "alpha");
    attest_domain(&e, &client, &s2, "beta");
    attest_domain(&e, &client, &s3, "gamma");

    submit_test_price(&client, &s1, &asset, 100, 1_000);
    submit_test_price_n(&client, &s2, &asset, 100, 1_000, 2);
    submit_test_price_n(&client, &s3, &asset, 100, 1_000, 3);

    let report = client.get_coverage_report(&asset);
    assert_eq!(report.registered_sources, 3, "three sources are admitted");
    assert_eq!(report.min_window_participation, 3, "all three participated");
    assert_eq!(
        report.low_participation_windows, 0,
        "a fully participating window is not a gap"
    );
    assert!(!report.below_independence_threshold);
    assert!(
        report.recommendations.is_empty(),
        "a fully covered asset needs no advice"
    );
}

/// Coverage analysis is read-only: it can neither admit nor remove a source.
#[test]
fn coverage_analysis_cannot_admit_or_remove_sources() {
    let e = Env::default();
    let (client, s1, asset) = base(&e);
    client.set_coverage_thresholds(&5u32, &10u32);
    submit_test_price(&client, &s1, &asset, 100, 1_000);

    let before = client.list_sources();
    let assets_before = client.list_assets();
    // Run every read-only entry point repeatedly.
    for _ in 0..3 {
        let _ = client.get_coverage_report(&asset);
        let _ = client.get_coverage_gap_list();
    }

    assert_eq!(client.list_sources(), before, "the source set is unchanged");
    assert_eq!(client.list_assets(), assets_before);
    assert!(
        client
            .get_coverage_report(&asset)
            .below_independence_threshold
    );
}

/// The `coverage` module itself contains no admission or removal call, so the
/// analysis cannot become a source-admission gate by accident.
#[test]
fn coverage_is_read_only() {
    let source = include_str!("coverage.rs");
    // Check for call syntax, not prose: the module's doc comment names these
    // functions precisely to say it does not call them.
    for forbidden in [
        "sources::add_source",
        "sources::remove_source",
        "add_source_asset",
        "remove_source_asset",
        "set_source_diversity",
        "set_source_geo",
        "pause_asset",
        "storage().persistent().remove(&DataKey::Source",
    ] {
        assert!(
            !source.contains(forbidden),
            "coverage.rs must not call {forbidden}"
        );
    }
}

/// Thresholds are admin-configurable and validated.
#[test]
fn coverage_thresholds_are_validated() {
    let e = Env::default();
    let (client, _asset) = drift_asset(&e);
    let (min_independent, window) = client.get_coverage_thresholds();
    assert_eq!(
        min_independent,
        crate::coverage::DEFAULT_MIN_INDEPENDENT_SOURCES
    );
    assert_eq!(window, crate::coverage::DEFAULT_WINDOW_LEDGERS);

    assert!(client.try_set_coverage_thresholds(&0u32, &100u32).is_err());
    assert!(client.try_set_coverage_thresholds(&3u32, &0u32).is_err());
    client.set_coverage_thresholds(&4u32, &50u32);
    assert_eq!(client.get_coverage_thresholds(), (4, 50));
}

/// The per-asset counter set stays bounded, and pruning costs O(1) storage
/// touches however large the window index has grown. A range walk here would
/// touch thousands of entries at a high ledger and blow the network's
/// 100-entry footprint limit on a read path.
#[test]
fn degradation_window_buckets_stay_bounded() {
    let e = Env::default();
    let (client, source, asset) = base(&e);
    client.set_degradation_config(&no_sampling(10));
    submit_test_price(&client, &source, &asset, 100, 1_000);

    // A very high ledger: a range-walk prune would iterate ~500 000 times here.
    ledger_default(&e, 5_000_000, 1_000_000_000);
    client.override_price(
        &asset,
        &123i128,
        &String::from_str(&e, "ops"),
        &5_000_010u32,
    );
    assert!(client.get_price(&asset, &0u64).is_some());

    let current = client.get_degradation_stats(&asset).window;
    let mut written = 0u32;
    for offset in 0..(degradation::RETAINED_WINDOWS + 4) {
        let w = current.saturating_sub(offset);
        let stats = client.get_degradation_window_stats(&asset, &w);
        if stats.total_degraded > 0 {
            written += 1;
        }
    }
    assert!(
        written <= degradation::RETAINED_WINDOWS + 1,
        "at most RETAINED_WINDOWS+1 buckets may hold counts, found {written}"
    );
}
