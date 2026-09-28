#![cfg(test)]
//! Tests for the data-quality track: #491 (robust outlier pre-filtering),
//! #492 (submission-to-aggregate latency analytics), #493 (aggregate
//! provenance) and #494 (pairwise disagreement index).

extern crate std;

use std::string::ToString;

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger},
    Address, Env, FromVal, String, Symbol, Vec,
};

use crate::disagreement;
use crate::latency;
use crate::outlier_filter::{
    self, DETECTOR_IQR, DETECTOR_MAD, DETECTOR_NONE, MAX_SENSITIVITY_BPS, MIN_SOURCES_FLOOR,
};
use crate::test_helpers::*;
use crate::types::{DataKey, LatencySample, OutlierConfig};
use crate::{PriceOracleContractClient, ProvenanceRecord};

/// Deploys an oracle with `n` sources and one asset, `min_sources = 1` so a
/// single submission is enough to publish. Returns (client, asset, sources).
fn env_with_sources(e: &Env, n: usize) -> (PriceOracleContractClient<'_>, Address, Vec<Address>) {
    // These tests are about data-quality accounting, not the per-invocation
    // footprint cap, which a wide source set would otherwise trip.
    e.cost_estimate().disable_resource_limits();
    ledger_default(e, 1, 5);
    let (client, _admin) = setup_contract(e);
    client.set_min_sources_required(&1u32);
    client.set_max_history_length(&50u32);
    let asset = register_test_asset(e, &client);
    let mut sources = Vec::new(e);
    for _ in 0..n {
        let s = register_test_source(e, &client, "S");
        sources.push_back(s);
    }
    (client, asset, sources)
}

/// The current aggregate price, or `0` when nothing has been published.
fn aggregate_price(client: &PriceOracleContractClient<'_>, asset: &Address) -> i128 {
    client.get_price(asset, &0u64).map_or(0, |p| p.price)
}

/// Ledger of the last aggregate of `asset`, read from contract storage.
fn stored_last_aggregate_ledger(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    asset: &Address,
) -> u32 {
    e.as_contract(&client.address, || latency::last_aggregate_ledger(e, asset))
}

/// Event names (topic 0) published by the oracle at `client` so far, as
/// SDK strings so they can be compared without an allocation.
///
/// The Soroban test host scopes events to the invocation that produced them,
/// so this must be captured immediately after the call being asserted on —
/// a later `client.…` call clears the log.
fn event_names(e: &Env, client: &Address) -> std::vec::Vec<String> {
    e.events()
        .all()
        .filter_by_contract(client)
        .events()
        .iter()
        .filter_map(|ev| match &ev.body {
            soroban_sdk::xdr::ContractEventBody::V0(v0) => match v0.topics.first() {
                Some(soroban_sdk::xdr::ScVal::Symbol(sym)) => {
                    Some(String::from_str(e, &sym.to_string()))
                }
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// Whether `names` contains an event called `name`.
fn has(e: &Env, names: &[String], name: &str) -> bool {
    let want = String::from_str(e, name);
    names.iter().any(|n| *n == want)
}
// ══════════════════════════════════════════════════════════════════════════
// #491 — Robust outlier pre-filtering (MAD / IQR)
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn gross_outlier_is_excluded_and_clean_median_unchanged() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 5);
    client.set_outlier_config(
        &asset,
        &Some(OutlierConfig {
            detector: DETECTOR_MAD,
            sensitivity_bps: outlier_filter::DEFAULT_MAD_SENSITIVITY_BPS,
            min_sources: MIN_SOURCES_FLOOR,
        }),
    );

    // Five inliers within ~1% of each other.
    let inliers = [100_000i128, 100_500, 99_500, 100_200, 100_100];
    for (i, p) in inliers.iter().enumerate() {
        client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &5);
    }
    // A sixth source sends a value 100x off.
    let outlier = register_test_source(&e, &client, "Bad");
    client.submit_price(&outlier, &asset, &10_000_000, &5);
    let names = event_names(&e, &client.address);

    let exclusions = client.get_outlier_exclusions(&asset);
    assert_eq!(exclusions.len(), 1, "exactly the gross outlier is removed");
    let ex = exclusions.get_unchecked(0);
    assert_eq!(ex.source, outlier);
    assert_eq!(ex.price, 10_000_000);
    assert!(ex.score_bps >= outlier_filter::DEFAULT_MAD_SENSITIVITY_BPS);

    // No inlier was excluded, so the aggregate is exactly the clean median.
    assert_eq!(aggregate_price(&client, &asset), 100_100);

    // Every exclusion is evented, so an operator can audit the round.
    assert!(has(&e, &names, "outlier_excluded_event"));
}

#[test]
fn filtering_is_disabled_below_the_source_count_floor() {
    let e = Env::default();
    // 3 sources with a floor of 4: the estimators are unstable there, so the
    // round is aggregated unfiltered.
    let (client, asset, sources) = env_with_sources(&e, 3);
    let cfg = OutlierConfig {
        detector: DETECTOR_MAD,
        sensitivity_bps: outlier_filter::DEFAULT_MAD_SENSITIVITY_BPS,
        min_sources: MIN_SOURCES_FLOOR,
    };
    client.set_outlier_config(&asset, &Some(cfg.clone()));
    client.submit_price(&sources.get_unchecked(0), &asset, &100, &5);
    client.submit_price(&sources.get_unchecked(1), &asset, &100, &5);
    client.submit_price(&sources.get_unchecked(2), &asset, &100_000, &5);

    assert!(!outlier_filter::filtering_active(&cfg, 3));
    assert!(client.get_outlier_exclusions(&asset).is_empty());
    // The clean majority still sets the aggregate.
    assert_eq!(aggregate_price(&client, &asset), 100);
}

#[test]
fn source_count_floor_is_configurable_and_validated() {
    let e = Env::default();
    let (client, asset, _s) = env_with_sources(&e, 1);
    let mk = |detector, sensitivity_bps, min_sources| {
        Some(OutlierConfig {
            detector,
            sensitivity_bps,
            min_sources,
        })
    };
    // Below the hard floor, above the ceiling, unknown detector, and a
    // sensitivity over 100% are all rejected.
    assert!(client
        .try_set_outlier_config(&asset, &mk(DETECTOR_MAD, 35_000, MIN_SOURCES_FLOOR - 1))
        .is_err());
    assert!(client
        .try_set_outlier_config(&asset, &mk(DETECTOR_MAD, 35_000, 65))
        .is_err());
    assert!(client
        .try_set_outlier_config(&asset, &mk(3, 35_000, 5))
        .is_err());
    assert!(client
        .try_set_outlier_config(&asset, &mk(DETECTOR_MAD, MAX_SENSITIVITY_BPS + 1, 5))
        .is_err());

    // A per-asset override in bounds is accepted and readable.
    let cfg =
        mk(DETECTOR_IQR, outlier_filter::DEFAULT_IQR_SENSITIVITY_BPS, 6).expect("in-bounds config");
    client.set_outlier_config(&asset, &Some(cfg.clone()));
    assert!(has(
        &e,
        &event_names(&e, &client.address),
        "outlier_config_changed_event"
    ));
    assert_eq!(client.get_outlier_config(&asset), cfg);
    // Clearing reverts to the disabled default.
    client.set_outlier_config(&asset, &None);
    assert_eq!(client.get_outlier_config(&asset).detector, DETECTOR_NONE);
}

#[test]
fn legitimate_volatility_within_tolerance_is_not_filtered() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 6);
    client.set_outlier_config(
        &asset,
        &Some(OutlierConfig {
            detector: DETECTOR_MAD,
            sensitivity_bps: outlier_filter::DEFAULT_MAD_SENSITIVITY_BPS,
            min_sources: MIN_SOURCES_FLOOR,
        }),
    );
    // A fast-moving market: ~4% spread, a genuine move rather than a bad feed.
    let prices = [100_000i128, 101_000, 99_000, 100_500, 99_500, 100_200];
    for (i, p) in prices.iter().enumerate() {
        client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &5);
    }
    assert!(
        client.get_outlier_exclusions(&asset).is_empty(),
        "a 4% spread is inside the configured tolerance"
    );
}

#[test]
fn filtering_disabled_by_default() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 5);
    assert_eq!(client.get_outlier_config(&asset).detector, DETECTOR_NONE);
    let prices = [100i128, 100, 100, 100, 100_000];
    for (i, p) in prices.iter().enumerate() {
        client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &5);
    }
    assert!(client.get_outlier_exclusions(&asset).is_empty());
}

#[test]
fn majority_guard_keeps_a_lopsided_round_intact() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 6);
    // A 1 bps sensitivity would exclude more than half the round; the majority
    // guard then degrades to the unfiltered median rather than to a
    // single-source aggregate.
    client.set_outlier_config(
        &asset,
        &Some(OutlierConfig {
            detector: DETECTOR_MAD,
            sensitivity_bps: 1,
            min_sources: MIN_SOURCES_FLOOR,
        }),
    );
    // 4 of 6 sit far from the centre, so a 1 bps sensitivity would drop more
    // than half the round.
    let prices = [100i128, 200_000, 300_000, 400_000, 500_000, 600_000];
    for (i, p) in prices.iter().enumerate() {
        client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &5);
    }
    assert!(client.get_outlier_exclusions(&asset).is_empty());
    assert!(aggregate_price(&client, &asset) > 0);
}

// ══════════════════════════════════════════════════════════════════════════
// #492 — Submission-to-aggregate latency analytics
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn latency_report_states_units_and_window_and_is_reproducible() {
    let e = Env::default();
    // min_sources 2 so the first submission is held rather than published.
    let (client, asset, sources) = env_with_sources(&e, 2);
    client.set_min_sources_required(&2u32);
    let slow = sources.get_unchecked(0);

    // `slow` submits at ledger 1 and is only counted when the round at ledger
    // 4 publishes, so it waits 3 ledgers.
    ledger_default(&e, 1, 5);
    client.submit_price(&slow, &asset, &100, &5);
    ledger_default(&e, 4, 20);
    client.submit_price(&sources.get_unchecked(1), &asset, &100, &20);
    let names = event_names(&e, &client.address);

    let report = client.get_latency_report(&slow, &asset);
    // Units are explicit: every duration is in ledgers, with the conversion
    // constant reported alongside.
    assert_eq!(report.seconds_per_ledger, latency::SECONDS_PER_LEDGER);
    assert_eq!(report.never_counted, 0);
    assert_eq!(report.max_samples, latency::MAX_SAMPLES);
    assert!(report.samples > 0);
    assert_eq!(report.max_ledgers, 3, "submitted at 1, counted at 4");

    // Deferral is reported separately from the source's own latency.
    assert!(report.avg_deferral_ledgers > 0);
    assert!(
        report.max_ledgers != report.avg_deferral_ledgers,
        "queue latency is measured apart from publication deferral"
    );

    // Percentiles are reproducible from the raw window.
    let mut counted: std::vec::Vec<u32> = report
        .window
        .iter()
        .filter(|s: &LatencySample| s.counted)
        .map(|s: LatencySample| s.latency_ledgers)
        .collect();
    counted.sort_unstable();
    assert_eq!(report.p50_ledgers, latency::percentile(&counted, 50));
    assert_eq!(report.p90_ledgers, latency::percentile(&counted, 90));
    assert_eq!(report.max_ledgers, *counted.last().unwrap());
    let newest = report.window.get_unchecked(report.window.len() - 1);
    assert!(has(&e, &names, "submission_latency_event"));
}

#[test]
fn never_counted_submissions_are_distinguishable_from_slow_ones() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 2);
    // min_sources 2: this source's first submission is held, not published,
    // so the replacement below is what removes it from the round.
    client.set_min_sources_required(&2u32);
    let s = sources.get_unchecked(0);

    // Two submissions in the same ledger: the first is replaced before any
    // aggregate could count it.
    ledger_default(&e, 1, 5);
    client.submit_price(&s, &asset, &100, &5);
    client.submit_price(&s, &asset, &101, &5);
    let names = event_names(&e, &client.address);

    let report = client.get_latency_report(&s, &asset);
    assert_eq!(
        report.never_counted, 1,
        "the submission replaced before any aggregate counted it is recorded"
    );
    // A never-counted sample carries no inclusion ledger, which is what makes
    // it separable from a counted-but-slow one.
    let never = report
        .window
        .iter()
        .find(|x: &LatencySample| !x.counted)
        .expect("a never-counted sample is stored");
    assert_eq!(never.inclusion_ledger, 0);
    assert!(has(&e, &names, "submission_never_counted_event"));
}

#[test]
fn latency_storage_growth_is_bounded() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 1);
    let s = sources.get_unchecked(0);
    // Far more rounds than the window holds.
    for i in 1..(latency::MAX_SAMPLES as u32 * 4) {
        ledger_default(&e, i, i as u64 * 5);
        client.submit_price(&s, &asset, &(100 + i as i128), &(i as u64 * 5));
    }
    assert_eq!(
        client.get_latency_samples(&s, &asset).len(),
        latency::MAX_SAMPLES
    );
    let report = client.get_latency_report(&s, &asset);
    assert_eq!(report.samples, latency::MAX_SAMPLES);
    assert_eq!(report.max_samples, latency::MAX_SAMPLES);
}

#[test]
fn deferral_is_measured_from_the_previous_publication() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 1);
    let s = sources.get_unchecked(0);

    ledger_default(&e, 1, 5);
    client.submit_price(&s, &asset, &100, &5);
    assert_eq!(stored_last_aggregate_ledger(&e, &client, &asset), 1);

    // The next publication lands 10 ledgers later; the deferral of that round
    // is 10, while this source's own queue latency is 0 (it submitted there).
    ledger_default(&e, 11, 55);
    client.submit_price(&s, &asset, &100, &55);
    assert_eq!(stored_last_aggregate_ledger(&e, &client, &asset), 11);

    // This source is always counted in the ledger it submits, so its queue
    // latency is 0 while the publication deferral is the oracle's own doing.
    // The two are reported separately precisely so they cannot be confused.
    let report = client.get_latency_report(&s, &asset);
    assert_eq!(report.max_ledgers, 0, "this source was never queued late");
    assert!(
        report.avg_deferral_ledgers > 0,
        "the publication was deferred even though the source was on time"
    );
}

// ══════════════════════════════════════════════════════════════════════════
// #493 — Aggregate provenance
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn every_published_aggregate_has_retrievable_provenance() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 3);
    let prices = [100i128, 102, 98];
    for (i, p) in prices.iter().enumerate() {
        client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &5);
    }
    let names = event_names(&e, &client.address);

    let rec: ProvenanceRecord = client.get_provenance(&asset, &1u32);
    assert_eq!(rec.ledger, 1);
    assert_eq!(rec.price, aggregate_price(&client, &asset));
    assert_eq!(rec.num_sources, 3);
    assert_eq!(rec.contributors.len(), 3);
    assert_eq!(rec.deferral_ledgers.len(), 3);
    // Every contributor is named, with its price and submission ledger.
    for i in 0..3u32 {
        let c = rec.contributors.get_unchecked(i);
        assert!(sources.iter().any(|s| s == c.source));
        assert!(c.submission_ledger <= rec.ledger);
        assert!(prices.iter().any(|p| *p == c.price));
    }
    // The id is the commitment, and the chain verifies.
    assert_eq!(rec.id, rec.hash);
    assert!(client.verify_provenance(&asset, &1u32));
    assert!(has(&e, &names, "provenance_recorded_event"));
}

#[test]
fn provenance_is_correct_after_a_correction() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 2);
    ledger_default(&e, 1, 5);
    client.submit_price(&sources.get_unchecked(0), &asset, &100, &5);
    client.submit_price(&sources.get_unchecked(1), &asset, &100, &5);
    let first = client.get_provenance(&asset, &1u32);

    // A correction: a new publication at a later ledger with a new value.
    ledger_default(&e, 9, 45);
    client.submit_price(&sources.get_unchecked(0), &asset, &140, &45);
    client.submit_price(&sources.get_unchecked(1), &asset, &142, &45);
    let second = client.get_provenance(&asset, &9u32);

    // The later record reflects the correction and chains to the earlier one.
    assert_eq!(second.price, 141);
    assert_eq!(second.previous_hash, first.hash);
    assert_ne!(second.hash, first.hash);
    assert!(client.verify_provenance(&asset, &1u32));
    assert!(client.verify_provenance(&asset, &9u32));

    // The earlier record is immutable and still names the prices as they were.
    assert_eq!(client.get_provenance(&asset, &1u32).hash, first.hash);
    assert!(client
        .get_provenance(&asset, &1u32)
        .contributors
        .iter()
        .all(|c| c.price == 100));
}

#[test]
fn tampered_provenance_is_detectable() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 2);
    client.submit_price(&sources.get_unchecked(0), &asset, &100, &5);
    client.submit_price(&sources.get_unchecked(1), &asset, &100, &5);
    assert!(client.verify_provenance(&asset, &1u32));

    // Forge the record: raise the price, keep the original commitment.
    let key = DataKey::Provenance(asset.clone(), 1u32);
    e.as_contract(&client.address, || {
        let mut rec: ProvenanceRecord = e.storage().persistent().get(&key).unwrap();
        rec.price = 999_999;
        e.storage().persistent().set(&key, &rec);
    });
    assert!(!client.verify_provenance(&asset, &1u32));
    // The forgery is still readable — the query returns what is stored, and
    // verification is what tells a consumer not to trust it.
    assert_eq!(client.get_provenance(&asset, &1u32).price, 999_999);
}

#[test]
fn provenance_is_pruned_with_the_price_history() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 1);
    client.set_max_history_length(&2u32);
    let s = sources.get_unchecked(0);

    for i in 1..6u32 {
        ledger_default(&e, i, i as u64 * 5);
        client.submit_price(&s, &asset, &(100 + i as i128), &(i as u64 * 5));
    }
    // History keeps only the newest 2 ledgers, so ledgers 1..=3 are gone —
    // and their provenance went with them.
    for pruned in 1..=3u32 {
        assert!(
            client.try_get_provenance(&asset, &pruned).is_err(),
            "ledger {pruned} was pruned from history"
        );
    }
    for retained in 4..=5u32 {
        let rec = client.get_provenance(&asset, &retained);
        assert_eq!(rec.ledger, retained);
        assert!(client.verify_provenance(&asset, &retained));
    }
}

#[test]
fn provenance_storage_is_bounded_by_the_history_window() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 1);
    client.set_max_history_length(&3u32);
    let s = sources.get_unchecked(0);
    for i in 1..20u32 {
        ledger_default(&e, i, i as u64 * 5);
        client.submit_price(&s, &asset, &(100 + i as i128), &(i as u64 * 5));
    }
    let history: Vec<u32> = e.as_contract(&client.address, || {
        e.storage()
            .persistent()
            .get(&DataKey::PriceHistoryLedgers(asset.clone()))
            .unwrap_or_else(|| Vec::new(&e))
    });
    assert_eq!(history.len(), 3, "provenance cannot outlive this window");
    let stored = e.as_contract(&client.address, || {
        (1..=19u32)
            .filter(|l| {
                e.storage()
                    .persistent()
                    .has(&DataKey::Provenance(asset.clone(), *l))
            })
            .count()
    });
    assert_eq!(stored, 3, "one provenance record per retained ledger");
}

// ══════════════════════════════════════════════════════════════════════════
// #494 — Pairwise disagreement index
// ══════════════════════════════════════════════════════════════════════════

#[test]
fn lone_dissent_is_distinguishable_from_a_broad_split() {
    // Five tight inliers around 100 with one dissenter far away: only the
    // n-1 pairs the dissenter takes part in are inflated, so the median of the
    // pairs stays low while max_bps spikes.
    let (lone_index, lone_max) = disagreement::compute(&[100, 101, 99, 100, 100, 5_000]);
    // A genuine two-way split inflates most pairs, so the index rises with it.
    let (split_index, split_max) = disagreement::compute(&[100, 100, 100, 140, 140, 140]);

    assert!(
        lone_index < split_index,
        "a lone dissenter must not read like a broad split ({lone_index} vs {split_index})"
    );
    assert!(
        lone_max > split_max,
        "the lone dissenter owns the single largest deviation ({lone_max} vs {split_max})"
    );
    // The split is broad, so its index is close to its own max; the lone
    // dissent's index is a small fraction of its max.
    assert!(split_index * 2 > split_max);
    assert!(lone_index * 2 < lone_max);
}

#[test]
fn small_source_sets_are_handled_without_division_by_zero() {
    // Zero or one source forms no pair at all: the index is 0 by definition.
    assert_eq!(disagreement::compute(&[]), (0, 0));
    assert_eq!(disagreement::compute(&[100]), (0, 0));
    // Two sources form exactly one pair, so index and max coincide and the
    // deviation is relative to their midpoint.
    let (idx, max) = disagreement::compute(&[100, 110]);
    assert_eq!(idx, max);
    assert!(idx > 0 && idx < 10_000);
    // A degenerate (zero-median) set cannot be normalised, so it scores 0
    // rather than dividing by zero.
    assert_eq!(disagreement::compute(&[0, 0]), (0, 0));
}

#[test]
fn identical_prices_score_zero() {
    for n in 1..20usize {
        let v: std::vec::Vec<i128> = std::vec![123_456i128; n];
        assert_eq!(disagreement::compute(&v), (0, 0));
    }
}

#[test]
fn index_is_emitted_and_queryable() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 4);
    assert!(client.get_disagreement_index(&asset).is_none());

    let prices = [100i128, 101, 99, 100];
    for (i, p) in prices.iter().enumerate() {
        client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &5);
    }
    let names = event_names(&e, &client.address);
    let idx = client.get_disagreement_index(&asset).unwrap();
    assert_eq!(idx.asset, asset);
    assert_eq!(idx.ledger, 1);
    assert_eq!(idx.num_sources, 4);
    assert!(!idx.low_sample);
    assert!(
        !idx.above_baseline,
        "the first round has no baseline to exceed"
    );
    assert!(idx.index_bps > 0);
    assert!(has(&e, &names, "disagreement_index_event"));
}

#[test]
fn low_sample_is_flagged_below_three_sources() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 2);
    client.submit_price(&sources.get_unchecked(0), &asset, &100, &5);
    client.submit_price(&sources.get_unchecked(1), &asset, &110, &5);
    let idx = client.get_disagreement_index(&asset).unwrap();
    assert!(idx.low_sample);
    assert_eq!(idx.num_sources, 2);
    assert!(idx.index_bps > 0, "the single pair still scores");
}

#[test]
fn spike_against_rolling_baseline_is_flagged() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 4);
    // Several calm rounds establish a low baseline.
    for r in 1..=3u32 {
        ledger_default(&e, r, r as u64 * 5);
        for (i, p) in [100i128, 100, 100, 101].iter().enumerate() {
            client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &(r as u64 * 5));
        }
        assert!(
            !client
                .get_disagreement_index(&asset)
                .unwrap()
                .above_baseline
        );
    }
    // A genuine volatility spike: the index more than doubles the baseline.
    // This is a spike, not an error, which is why the baseline is reported
    // alongside the value.
    ledger_default(&e, 4, 20);
    for (i, p) in [100i128, 130, 100, 100].iter().enumerate() {
        client.submit_price(&sources.get_unchecked(i as u32), &asset, p, &20);
    }
    let idx = client.get_disagreement_index(&asset).unwrap();
    assert!(idx.above_baseline);
    assert!(
        idx.baseline_bps > 0,
        "the baseline travels with the reading"
    );
    assert!(idx.index_bps > idx.baseline_bps * 2);
}

#[test]
fn baseline_window_is_bounded() {
    let e = Env::default();
    let (client, asset, sources) = env_with_sources(&e, 2);
    for r in 1..(disagreement::BASELINE_WINDOW as u32 * 3) {
        ledger_default(&e, r, r as u64 * 5);
        client.submit_price(
            &sources.get_unchecked(0),
            &asset,
            &(100 + r as i128),
            &(r as u64 * 5),
        );
        client.submit_price(
            &sources.get_unchecked(1),
            &asset,
            &(100 + r as i128),
            &(r as u64 * 5),
        );
    }
    let hist = e.as_contract(&client.address, || disagreement::get_history(&e, &asset));
    assert_eq!(hist.len(), disagreement::BASELINE_WINDOW);
}

proptest! {
    /// The index is a pure ratio, so rescaling every price by a decimal
    /// factor (8 -> 18 decimals, and any integer power of ten) leaves it
    /// unchanged.
    #[test]
    fn index_is_scale_invariant(
        prices in prop::collection::vec(1i128..1_000_000i128, 2..12),
        scale in 1u32..=6,
    ) {
        let scaled: std::vec::Vec<i128> =
            prices.iter().map(|p| p * 10i128.pow(scale)).collect();
        let base = disagreement::compute(&prices);
        let rescaled = disagreement::compute(&scaled);
        // At most a couple of basis points of integer-rounding drift.
        let tol = 2i64;
        let d_index = (base.0 as i64 - rescaled.0 as i64).abs();
        let d_max = (base.1 as i64 - rescaled.1 as i64).abs();
        prop_assert!(d_index <= tol, "index drifted by {} bps", d_index);
        prop_assert!(d_max <= tol, "max drifted by {} bps", d_max);
    }
}
