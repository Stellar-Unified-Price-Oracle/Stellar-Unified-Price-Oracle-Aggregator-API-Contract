//! #399 — Source diversity acceptance tests.
//!
//! Proves the hardened property: nominally distinct sources that share one
//! failure domain score LOW on effective independence while the raw count
//! stays HIGH, and alerts fire on that synthetic correlated scenario.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::test_helpers::{register_test_source, setup_contract};

fn set_geo(
    e: &Env,
    client: &crate::PriceOracleContractClient<'_>,
    src: &Address,
    region: &str,
    provider: &str,
    jurisdiction: &str,
    infra: &str,
    upstream: &str,
    owner: &str,
) {
    client.set_source_geo(
        src,
        &crate::SourceGeoMetadata {
            region: String::from_str(e, region),
            provider: String::from_str(e, provider),
            jurisdiction: String::from_str(e, jurisdiction),
            infra: String::from_str(e, infra),
            upstream: String::from_str(e, upstream),
            owner: String::from_str(e, owner),
        },
    );
}

#[test]
fn test_diversity_correlated_set_scores_low_while_raw_stays_high() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Ten nominally distinct sources: distinct names/regions/jurisdictions,
    // but ONE shared infra + ONE shared upstream + ONE shared owner.
    // This is the Sybil / nominal-diversity trap from the issue.
    let regions = [
        "US", "EU", "AP", "SA", "AF", "US2", "EU2", "AP2", "SA2", "AF2",
    ];
    let jurisdictions = ["US", "DE", "SG", "BR", "NG", "US", "FR", "JP", "AR", "KE"];
    for i in 0..10 {
        let name = std::format!("Sybil{}", i);
        let src = register_test_source(&e, &client, &name);
        set_geo(
            &e,
            &client,
            &src,
            regions[i as usize],
            "AWS",
            jurisdictions[i as usize],
            "aws-us-east-1a",
            "single-coinbase-feed",
            "single-operator-llc",
        );
    }

    let report = client.get_source_diversity();
    // Raw count looks healthy…
    assert_eq!(report.raw_count, 10u32);
    // …but effective independence collapses to one failure domain.
    assert_eq!(report.effective_independent_count, 1u32);
    assert!(report.effective_independent_count < report.raw_count);
    // Shared axes are fully concentrated.
    assert_eq!(report.infra_hhi, 10000u32);
    assert_eq!(report.upstream_hhi, 10000u32);
    assert_eq!(report.owner_hhi, 10000u32);
    assert_eq!(report.largest_domain_size, 10u32);
    // Default thresholds (min 3 effective) flag this as low diversity.
    assert!(report.is_low_diversity);
}

#[test]
fn test_diversity_independent_set_scores_high() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Six genuinely independent sources: distinct on all three failure axes.
    let infras = [
        "aws-us-east-1",
        "gcp-europe-west1",
        "azure-eastasia",
        "baremetal-nyc",
        "hetzner-fsn1",
        "do-sgp1",
    ];
    let upstreams = [
        "coinbase",
        "kraken",
        "binance",
        "bitstamp",
        "self-operated",
        "chainlink",
    ];
    let owners = ["op-a", "op-b", "op-c", "op-d", "op-e", "op-f"];
    for i in 0..6 {
        let name = std::format!("Indep{}", i);
        let src = register_test_source(&e, &client, &name);
        set_geo(
            &e,
            &client,
            &src,
            "R",
            "P",
            "J",
            infras[i as usize],
            upstreams[i as usize],
            owners[i as usize],
        );
    }

    let report = client.get_source_diversity();
    assert_eq!(report.raw_count, 6u32);
    assert_eq!(report.effective_independent_count, 6u32);
    assert_eq!(report.largest_domain_size, 1u32);
    assert!(!report.is_low_diversity);
    assert!(!client.check_diversity_alert());
}

#[test]
fn test_diversity_alert_fires_on_synthetic_correlated_scenario() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Explicit thresholds so the test does not depend on defaults.
    client.set_diversity_thresholds(&3u32, &5000u32);
    let cfg = client.get_diversity_thresholds();
    assert_eq!(cfg.min_effective_sources, 3u32);
    assert_eq!(cfg.max_hhi_per_axis, 5000u32);

    // Five sources, cosmetic geo spread, one shared failure domain.
    for i in 0..5 {
        let name = std::format!("Corr{}", i);
        let src = register_test_source(&e, &client, &name);
        let region = std::format!("R{}", i);
        set_geo(
            &e,
            &client,
            &src,
            &region,
            "AWS",
            "US",
            "aws-shared",
            "shared-feed",
            "shared-owner",
        );
    }

    let report = client.get_source_diversity();
    assert_eq!(report.raw_count, 5u32);
    assert_eq!(report.effective_independent_count, 1u32);

    // Alert MUST fire even though raw_count (5) >= min (3).
    assert!(client.check_diversity_alert());
    assert!(report.is_low_diversity);
    let breach = client.get_last_diversity_breach_ledger();
    assert!(breach.is_some());
}

#[test]
fn test_diversity_threshold_validation() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    // Zero minimum and over-range HHI must be rejected.
    assert!(client
        .try_set_diversity_thresholds(&0u32, &5000u32)
        .is_err());
    assert!(client
        .try_set_diversity_thresholds(&3u32, &10001u32)
        .is_err());
    assert!(client.try_set_diversity_thresholds(&3u32, &0u32).is_err());
}

#[test]
fn test_diversity_empty_set() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let report = client.get_source_diversity();
    assert_eq!(report.raw_count, 0u32);
    assert_eq!(report.effective_independent_count, 0u32);
    assert_eq!(report.largest_domain_size, 0u32);
}
