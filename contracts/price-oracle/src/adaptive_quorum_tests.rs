//! Tests for the #482 volatility-bucketed adaptive quorum.
//!
//! The properties under test: a round's quorum is fixed at start, calm and
//! volatile regimes select the documented quorums, downgrades require
//! hysteresis, and one observation cannot force a downgrade.

#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, Vec};

use crate::test_helpers::{register_test_asset, setup_contract};
use crate::{PriceOracleContractClient, VolatilityBuckets};

/// Boundaries at 50 and 500 bps; quorums 1 / 3 / 5. A downgrade needs
/// `relax_after` consecutive observations in the calmer bucket.
fn test_config(env: &Env, relax_after: u32) -> VolatilityBuckets {
    let mut boundaries: Vec<u32> = Vec::new(env);
    boundaries.push_back(50);
    boundaries.push_back(500);
    let mut quorums: Vec<u32> = Vec::new(env);
    quorums.push_back(1);
    quorums.push_back(3);
    quorums.push_back(5);
    VolatilityBuckets {
        boundaries,
        quorums,
        relax_after,
        min_samples: 2,
        window: 16,
    }
}

fn setup<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address, Address) {
    let (client, _admin) = setup_contract(e);
    let asset = register_test_asset(e, &client);
    let contract = Address::generate(e);
    (client, asset, contract)
}

/// Calm prices select the cheapest quorum.
#[test]
fn calm_regime_selects_the_lowest_quorum() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    // The first observation only seeds the reference price, so two further
    // observations are needed to reach `min_samples`.
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &100_000i128);
    let regime = client.observe_volatility(&asset, &100_000i128);
    assert_eq!(regime.bucket, 0, "a flat price is the calmest bucket");
    assert_eq!(regime.samples, 2);
    assert_eq!(regime.quorum, 1);
    assert_eq!(regime.volatility_bps, 0);
    assert_eq!(client.get_effective_quorum(&asset), 1);
}

/// A large move selects the top bucket and its quorum.
#[test]
fn volatile_regime_selects_the_highest_quorum() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    // Seed, then repeated 10 % moves. The mean absolute return is 1 000 bps,
    // at or above the 500 bps boundary.
    client.observe_volatility(&asset, &100_000i128);
    let regime = client.observe_volatility(&asset, &110_000i128);
    client.observe_volatility(&asset, &100_000i128);
    let regime = client.observe_volatility(&asset, &110_000i128);
    assert!(
        regime.bucket >= 1,
        "sustained 10 % moves must not read as calm, got bucket {}",
        regime.bucket
    );
    assert!(regime.quorum > 1);

    client.observe_volatility(&asset, &100_000i128);
    let regime = client.observe_volatility(&asset, &110_000i128);
    assert_eq!(regime.bucket, 2, "sustained 10 % moves are the top bucket");
    assert_eq!(regime.quorum, 5);
}

/// Tightening is immediate: volatility rising needs no confirmation.
#[test]
fn tightening_the_quorum_is_immediate() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &100_000i128);
    assert_eq!(client.get_quorum_regime(&asset).quorum, 1);

    // One big move, and the quorum is up on that same observation.
    let regime = client.observe_volatility(&asset, &200_000i128);
    assert!(
        regime.quorum > 1,
        "raising the quorum must not wait for hysteresis"
    );
}

/// Relaxing the quorum requires the calmer bucket to hold.
#[test]
fn bucket_downgrade_requires_hysteresis() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    // Drive the asset firmly into the top bucket.
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &200_000i128);
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &200_000i128);
    client.observe_volatility(&asset, &200_000i128);
    let hot = client.get_quorum_regime(&asset);
    assert_eq!(hot.bucket, 2);
    assert_eq!(hot.quorum, 5);

    // Now go completely flat. The window still holds the large returns, so the
    // estimate cannot drop below the boundary yet and the bucket must hold.
    let mut saw_downgrade = false;
    for _ in 0..20 {
        let r = client.observe_volatility(&asset, &200_000i128);
        if r.bucket < 2 {
            saw_downgrade = true;
        }
    }
    assert!(
        saw_downgrade,
        "a sustained calm market should eventually relax the bucket"
    );
    assert!(
        client.get_effective_quorum(&asset) < 5,
        "a sustained calm market should eventually relax the quorum"
    );
}

/// A single calm observation cannot force a downgrade.
#[test]
fn a_single_submission_cannot_force_a_downgrade() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 5));

    // Establish a volatile regime.
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &300_000i128);
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &300_000i128);
    client.observe_volatility(&asset, &300_000i128);
    assert_eq!(client.get_quorum_regime(&asset).quorum, 5);

    // One flat observation. The bucket must not move, and the quorum must not
    // drop — an attacker who can report a calm price once must not be able to
    // talk the quorum down.
    let r = client.observe_volatility(&asset, &300_000i128);
    assert_eq!(
        r.quorum, 5,
        "one calm observation relaxed the quorum to {}",
        r.quorum
    );
}

/// A round's quorum is fixed at start and unaffected by a mid-round change.
#[test]
fn round_quorum_is_fixed_at_start() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    // Build a volatile regime so the pinned quorum is high.
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &300_000i128);
    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &300_000i128);
    client.observe_volatility(&asset, &300_000i128);

    let pinned = client.pin_round_quorum(&asset, &7u32);
    assert_eq!(pinned.round, 7);
    assert_eq!(pinned.quorum, 5);

    // Now the market goes flat and the regime relaxes over several rounds.
    for _ in 0..30 {
        client.observe_volatility(&asset, &300_000i128);
    }
    let current = client.get_effective_quorum(&asset);
    assert!(
        current < pinned.quorum,
        "the regime should have relaxed below the pinned quorum"
    );

    // The round in flight is unaffected.
    let still = client.get_round_quorum(&asset, &7u32).expect("pinned");
    assert_eq!(
        still.quorum, pinned.quorum,
        "a mid-round regime change altered an in-flight round's quorum"
    );
    assert_eq!(still.bucket, pinned.bucket);
}

/// Too few observations to classify is an explicit error, not a silent default.
#[test]
fn too_few_samples_is_an_explicit_error() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_effective_quorum(&asset)
    }));
    assert!(result.is_err(), "an unmeasured regime must not be guessed");

    // The getter still reports the shortfall rather than panicking.
    let regime = client.get_quorum_regime(&asset);
    assert_eq!(regime.samples, 0);
    assert_eq!(regime.volatility_bps, 0);
}

/// An invalid bucket configuration is rejected.
#[test]
fn invalid_configuration_is_rejected() {
    let e = Env::default();
    let (client, _asset, _contract) = setup(&e);

    // Too few quorums: two boundaries need three.
    let mut c = test_config(&e, 3);
    let mut two: Vec<u32> = Vec::new(&e);
    two.push_back(1);
    two.push_back(3);
    c.quorums = two;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_adaptive_quorum_config(&c)
    }));
    assert!(result.is_err(), "each bucket needs exactly one quorum");

    // A zero quorum would make a round trivially satisfiable.
    let mut d = test_config(&e, 3);
    let mut zeroes: Vec<u32> = Vec::new(&e);
    zeroes.push_back(0);
    zeroes.push_back(3);
    zeroes.push_back(5);
    d.quorums = zeroes;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_adaptive_quorum_config(&d)
    }));
    assert!(result.is_err(), "a zero quorum is not a quorum");

    // Boundaries must strictly ascend.
    let mut f = test_config(&e, 3);
    let mut descending: Vec<u32> = Vec::new(&e);
    descending.push_back(500);
    descending.push_back(50);
    f.boundaries = descending;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.set_adaptive_quorum_config(&f)
    }));
    assert!(result.is_err(), "boundaries must ascend");
}

/// The volatility estimate is scale-invariant: the same *relative* move reads
/// the same whether the asset trades at 1 or at 1 000 000.
#[test]
fn volatility_estimate_is_scale_invariant() {
    let e = Env::default();
    let (client, small, _contract) = setup(&e);
    let large = Address::generate(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    // Both assets see the identical 10 % up-move.
    for price in [100_000i128, 110_000, 100_000, 110_000] {
        client.observe_volatility(&small, &price);
    }
    for price in [100_000_000i128, 110_000_000, 100_000_000, 110_000_000] {
        client.observe_volatility(&large, &price);
    }

    let a = client.get_quorum_regime(&small);
    let b = client.get_quorum_regime(&large);
    assert_eq!(
        a.volatility_bps, b.volatility_bps,
        "a scale-invariant estimate gave {} bps and {} bps",
        a.volatility_bps, b.volatility_bps
    );
    assert_eq!(a.bucket, b.bucket);
    assert_eq!(a.quorum, b.quorum);
    // And the estimate really is a 10 %-scale number, not a rounding artefact.
    assert!(
        (900..=1_000).contains(&a.volatility_bps),
        "expected ~1000 bps, got {}",
        a.volatility_bps
    );
}

/// Bucket and quorum are observable to a consumer at any time.
#[test]
fn bucket_and_quorum_are_observable() {
    let e = Env::default();
    let (client, asset, _contract) = setup(&e);
    client.set_adaptive_quorum_config(&test_config(&e, 3));

    let stored = client.get_adaptive_quorum_config();
    assert_eq!(stored.boundaries.len(), 2);
    assert_eq!(stored.quorums.len(), 3);
    assert_eq!(stored.relax_after, 3);

    client.observe_volatility(&asset, &100_000i128);
    client.observe_volatility(&asset, &100_001i128);
    client.observe_volatility(&asset, &100_001i128);
    let regime = client.get_quorum_regime(&asset);
    assert_eq!(regime.bucket, 0);
    assert_eq!(regime.quorum, 1);
    assert_eq!(regime.samples, 2);
}
