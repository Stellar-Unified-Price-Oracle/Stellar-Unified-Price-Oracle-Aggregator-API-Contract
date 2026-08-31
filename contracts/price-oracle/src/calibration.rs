//! # Source Accuracy Calibration Framework
//!
//! Measures each source's historical accuracy against an admin-submitted
//! reference benchmark price (e.g. a trusted external index), maintains a
//! rolling per-(asset, source) accuracy score, and exposes a bounded
//! calibration weight that downstream aggregation or off-chain analytics can
//! consume.
//!
//! ## Design notes
//!
//! - **Reference benchmarks** ([`CalibrationBenchmark`]) are asset-scoped and
//!   admin-submitted, mirroring `cross_chain_verify::submit_cross_chain_price`'s
//!   trusted-input convention elsewhere in this contract.
//! - **Rolling accuracy** uses the same deviation -> accuracy curve as
//!   `reputation::update_reputation_on_submission` (0 at >=50% deviation, 100
//!   within 1%), smoothed with a configurable EMA, so a source's calibration
//!   score and its reputation score are comparable to each other.
//! - **Feeding into aggregation weighting**: rather than mutating the core
//!   median aggregation path in `prices.rs` (which every consumer already
//!   depends on), calibration exposes [`get_calibration_weight`] as a
//!   read-only signal — the "or analytics" half of the issue's acceptance
//!   criteria. A future weighted-aggregation mode can multiply submissions by
//!   this weight without calibration itself needing to touch the hot path.
//! - **"Runs automatically... reports per-source accuracy"**: `scripts/calibration_runner.py`
//!   is the off-chain half — it fetches a reference price, reads each
//!   source's last submission via the existing `get_source_price` query, and
//!   calls `calibration_record_sample` on a schedule, printing a per-source
//!   accuracy report every run.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{CalibrationBenchmarkSetEvent, CalibrationScoreUpdatedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{CalibrationBenchmark, CalibrationConfig, CalibrationScore, DataKey, ErrorCode};

const DEFAULT_ENABLED: bool = true;
const DEFAULT_SMOOTHING_BPS: u32 = 2_000;
const DEFAULT_MIN_SAMPLES: u32 = 5;
const DEFAULT_MAX_WEIGHT_BPS: u32 = 2_000;
const NEUTRAL_ACCURACY: u32 = 50;

/// Sets the global calibration configuration. Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidCalibrationConfig`] — `smoothing_bps` or `max_weight_bps` exceeds 10000.
pub fn set_calibration_config(env: &Env, config: CalibrationConfig) {
    let admin = get_admin(env);
    admin.require_auth();

    if config.smoothing_bps > 10_000 || config.max_weight_bps > 10_000 {
        panic_with_error!(env, ErrorCode::InvalidCalibrationConfig);
    }

    env.storage()
        .persistent()
        .set(&DataKey::CalibrationConfig, &config);
    env.storage().persistent().extend_ttl(
        &DataKey::CalibrationConfig,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

/// Returns the current calibration configuration, defaulting to a
/// conservative preset (enabled, 20% EMA smoothing, 5-sample minimum, 20%
/// max weight contribution) if never set.
pub fn get_calibration_config(env: &Env) -> CalibrationConfig {
    env.storage()
        .persistent()
        .get(&DataKey::CalibrationConfig)
        .unwrap_or(CalibrationConfig {
            enabled: DEFAULT_ENABLED,
            smoothing_bps: DEFAULT_SMOOTHING_BPS,
            min_samples_for_weighting: DEFAULT_MIN_SAMPLES,
            max_weight_bps: DEFAULT_MAX_WEIGHT_BPS,
        })
}

/// Sets the reference benchmark price for an asset. Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::InvalidPrice`] — `reference_price` is non-positive.
pub fn set_calibration_benchmark(
    env: &Env,
    asset: Address,
    reference_price: i128,
    decimals: u32,
    timestamp: u64,
) {
    let admin = get_admin(env);
    admin.require_auth();

    crate::storage::check_registered_asset(env, &asset);
    if reference_price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let benchmark = CalibrationBenchmark {
        asset: asset.clone(),
        reference_price,
        decimals,
        timestamp,
    };
    let key = DataKey::CalibrationBenchmark(asset);
    env.storage().persistent().set(&key, &benchmark);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    CalibrationBenchmarkSetEvent {
        asset: benchmark.asset.clone(),
        reference_price,
        timestamp,
    }
    .publish(env);
}

/// Returns the reference benchmark for an asset, or `None`.
pub fn get_calibration_benchmark(env: &Env, asset: Address) -> Option<CalibrationBenchmark> {
    env.storage()
        .persistent()
        .get(&DataKey::CalibrationBenchmark(asset))
}

/// Maps a deviation (basis points) to an accuracy score (0-100), using the
/// same curve as `reputation::update_reputation_on_submission` so calibration
/// and reputation scores stay comparable.
fn accuracy_from_deviation_bps(deviation_bps: i128) -> i128 {
    let accuracy: i128 = if deviation_bps >= 5000 {
        0
    } else if deviation_bps >= 3000 {
        10 - (deviation_bps - 3000) * 10 / 2000
    } else if deviation_bps <= 100 {
        100
    } else {
        100 - (deviation_bps - 100) * 90 / 2900
    };
    accuracy.clamp(0, 100)
}

/// Records one accuracy sample for `source` on `asset` against the stored
/// reference benchmark, updating its rolling (EMA) calibration score.
///
/// Admin-gated: intended to be called by a trusted off-chain calibration
/// process (see `scripts/calibration_runner.py`) running on a schedule.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::CalibrationBenchmarkNotFound`] — no benchmark set for `asset`.
/// * [`ErrorCode::InvalidPrice`] — `source_price` is non-positive.
pub fn record_calibration_sample(env: &Env, asset: Address, source: Address, source_price: i128) {
    let admin = get_admin(env);
    admin.require_auth();

    let benchmark = get_calibration_benchmark(env, asset.clone())
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::CalibrationBenchmarkNotFound));

    if source_price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let deviation_bps = (source_price - benchmark.reference_price)
        .abs()
        .saturating_mul(10_000)
        / benchmark.reference_price;
    let accuracy = accuracy_from_deviation_bps(deviation_bps);

    let config = get_calibration_config(env);
    let smoothing = config.smoothing_bps as i128;

    let key = DataKey::CalibrationScore(asset.clone(), source.clone());
    let existing: CalibrationScore =
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(CalibrationScore {
                asset: asset.clone(),
                source: source.clone(),
                sample_count: 0,
                rolling_accuracy: NEUTRAL_ACCURACY,
                last_updated: 0,
            });

    let new_rolling = ((existing.rolling_accuracy as i128) * (10_000 - smoothing)
        + accuracy * smoothing)
        / 10_000;
    let new_rolling = new_rolling.clamp(0, 100) as u32;

    let updated = CalibrationScore {
        asset: asset.clone(),
        source: source.clone(),
        sample_count: existing.sample_count.saturating_add(1),
        rolling_accuracy: new_rolling,
        last_updated: env.ledger().timestamp(),
    };

    env.storage().persistent().set(&key, &updated);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    CalibrationScoreUpdatedEvent {
        asset,
        source,
        rolling_accuracy: new_rolling,
        sample_count: updated.sample_count,
    }
    .publish(env);
}

/// Returns the rolling calibration score for (asset, source), defaulting to a
/// neutral, zero-sample score if none has been recorded yet.
pub fn get_calibration_score(env: &Env, asset: Address, source: Address) -> CalibrationScore {
    let key = DataKey::CalibrationScore(asset.clone(), source.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(CalibrationScore {
            asset,
            source,
            sample_count: 0,
            rolling_accuracy: NEUTRAL_ACCURACY,
            last_updated: 0,
        })
}

/// Returns the calibration-derived aggregation weight (basis points, out of
/// 10000) for (asset, source). Returns `0` when calibration is disabled, the
/// source has not yet accumulated `min_samples_for_weighting` samples, or no
/// score has been recorded — i.e. it never silently assumes a source is
/// trustworthy.
pub fn get_calibration_weight(env: &Env, asset: Address, source: Address) -> u32 {
    let config = get_calibration_config(env);
    if !config.enabled {
        return 0;
    }

    let score = get_calibration_score(env, asset, source);
    if score.sample_count < config.min_samples_for_weighting {
        return 0;
    }

    (score.rolling_accuracy.saturating_mul(config.max_weight_bps) / 100).min(config.max_weight_bps)
}

/// Returns calibration scores for every currently registered source against
/// `asset` — the on-chain half of "reports per-source accuracy".
pub fn calibration_report(env: &Env, asset: Address) -> Vec<CalibrationScore> {
    let oracle_sources = crate::storage::read_oracle_sources(env);
    let mut report = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let source = oracle_sources.sources.get_unchecked(i);
        report.push_back(get_calibration_score(env, asset.clone(), source));
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Env, String as SorobanString};

    fn setup(env: &Env) -> (Address, Address, Address) {
        let admin = Address::generate(env);
        env.ledger().with_mut(|l| l.timestamp = 1_000);
        crate::admin::initialize(
            env,
            admin.clone(),
            1,
            100,
            8,
            SorobanString::from_slice(env, "Oracle"),
        );
        let asset = Address::generate(env);
        crate::assets::register_asset(env, asset.clone());
        let source = Address::generate(env);
        (admin, asset, source)
    }

    #[test]
    fn test_accurate_source_converges_to_high_score() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, asset, source) = setup(&env);

        set_calibration_benchmark(&env, asset.clone(), 100_000_000, 8, 1_000);

        // Repeated near-exact submissions should push rolling_accuracy toward 100.
        for _ in 0..10 {
            record_calibration_sample(&env, asset.clone(), source.clone(), 100_050_000);
        }

        let score = get_calibration_score(&env, asset, source);
        assert!(
            score.rolling_accuracy >= 95,
            "expected high accuracy, got {}",
            score.rolling_accuracy
        );
        assert_eq!(score.sample_count, 10);
    }

    #[test]
    fn test_inaccurate_source_converges_to_low_score_and_zero_weight() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, asset, source) = setup(&env);

        set_calibration_benchmark(&env, asset.clone(), 100_000_000, 8, 1_000);

        // Consistently 60% off the benchmark.
        for _ in 0..10 {
            record_calibration_sample(&env, asset.clone(), source.clone(), 160_000_000);
        }

        let score = get_calibration_score(&env, asset.clone(), source.clone());
        assert!(
            score.rolling_accuracy <= 5,
            "expected low accuracy, got {}",
            score.rolling_accuracy
        );

        let weight = get_calibration_weight(&env, asset, source);
        assert_eq!(
            weight, 0,
            "an inaccurate source must not receive aggregation weight"
        );
    }

    #[test]
    fn test_weight_zero_below_min_samples() {
        let env = Env::default();
        env.mock_all_auths();
        let (_admin, asset, source) = setup(&env);

        set_calibration_benchmark(&env, asset.clone(), 100_000_000, 8, 1_000);
        record_calibration_sample(&env, asset.clone(), source.clone(), 100_000_000);

        // Default min_samples_for_weighting is 5; only 1 sample recorded so far.
        let weight = get_calibration_weight(&env, asset, source);
        assert_eq!(weight, 0);
    }

    #[test]
    fn test_calibration_report_covers_all_sources() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, asset, _source) = setup(&env);
        let _ = admin;

        crate::sources::add_source(
            &env,
            Address::generate(&env),
            SorobanString::from_slice(&env, "A"),
        );
        crate::sources::add_source(
            &env,
            Address::generate(&env),
            SorobanString::from_slice(&env, "B"),
        );

        let report = calibration_report(&env, asset);
        assert_eq!(report.len(), 2);
    }
}
