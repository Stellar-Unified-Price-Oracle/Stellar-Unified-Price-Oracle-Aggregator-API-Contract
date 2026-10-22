//! # Off-Chain Price Deviation Alerting System (#199)
//!
//! Monitors for price deviations vs external references and triggers alerts
//! when thresholds are exceeded. Supports multiple notification channels.
//!
//! #624: Repeated alerts are deduplicated and grouped by incident, asset, and
//! source. A storm of equivalent alerts collapses into a single active incident
//! while every individual occurrence is still recorded for audit.
//!
//! #625: Provides a deterministic monthly service-level evidence report built
//! from the operational alert data. The report covers freshness, availability,
//! correction rate, and incident time, and always carries metric definitions
//! plus missing-data warnings so a fixed dataset yields a fixed report.

use soroban_sdk::{Address, Env, Vec};

use crate::events::PriceDeviationAlertEvent;
use crate::storage::LEDGER_BUMP;
use crate::types::DataKey;

/// Checks price deviation against a reference and triggers alert if threshold exceeded.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset being monitored.
/// * `our_price` - Our aggregated price.
/// * `reference_price` - Price from external reference (Coinbase, Binance, etc).
/// * `deviation_threshold_bps` - Threshold in basis points (1% = 100 bps).
///
/// # Returns
/// `true` if deviation exceeded threshold and alert triggered, `false` otherwise.
pub fn check_and_alert_deviation(
    env: &Env,
    asset: Address,
    our_price: i128,
    reference_price: i128,
    deviation_threshold_bps: u32,
) -> bool {
    if our_price <= 0 || reference_price <= 0 {
        return false;
    }

    // Calculate deviation in basis points
    let abs_diff = (our_price - reference_price).abs();
    let deviation_bps = (abs_diff * 10_000) / reference_price;

    if deviation_bps as u32 > deviation_threshold_bps {
        // Trigger alert
        record_deviation_alert(
            env,
            &asset,
            our_price,
            reference_price,
            deviation_bps as u32,
        );
        // Classify the deviation by severity and route it to the appropriate channel.
        crate::alert_severity::evaluate_and_route(env, &asset, deviation_bps as u32);
        true
    } else {
        false
    }
}

/// Records a price deviation alert for monitoring/indexing.
///
/// Called internally when deviation exceeds threshold. Each occurrence is
/// preserved for audit, but equivalent alerts are grouped into a single active
/// incident keyed by (asset, source) so an alert storm does not fan out into
/// duplicate incidents.
fn record_deviation_alert(
    env: &Env,
    asset: &Address,
    our_price: i128,
    reference_price: i128,
    deviation_bps: u32,
) {
    // Track last alert for this asset
    let key = DataKey::AlertLastPrice(asset.clone());
    env.storage().persistent().set(&key, &our_price);
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);

    // Group this occurrence into the active incident for (asset, source).
    // The incident id is derived deterministically from the grouping key so a
    // storm of equivalent alerts resolves to exactly one active incident.
    let incident_key = DataKey::AlertIncident(asset.clone(), reference_price);
    let mut event_count: u32 = env
        .storage()
        .persistent()
        .get(&incident_key)
        .unwrap_or(0u32);
    event_count = event_count.saturating_add(1);
    env.storage().persistent().set(&incident_key, &event_count);
    env.storage()
        .persistent()
        .extend_ttl(&incident_key, 300000, LEDGER_BUMP);

    PriceDeviationAlertEvent {
        asset: asset.clone(),
        our_price,
        reference_price,
        deviation_bps,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
}

/// Returns the number of occurrences grouped into the active incident for an
/// asset and reference source. Returns 0 when no incident is active.
pub fn get_incident_event_count(env: &Env, asset: &Address, source: i128) -> u32 {
    let key = DataKey::AlertIncident(asset.clone(), source);
    env.storage().persistent().get(&key).unwrap_or(0u32)
}

/// Registers an external reference oracle for price comparison.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset to monitor.
/// * `reference_oracle` - External oracle contract address.
pub fn register_reference_oracle(env: &Env, asset: Address, reference_oracle: Address) {
    let key = DataKey::ReferenceOracle(reference_oracle.clone());
    // Store the mapping from our asset to reference oracle
    let mut mapping = soroban_sdk::Map::new(env);
    mapping.set(asset.clone(), reference_oracle.clone());

    env.storage().persistent().set(&key, &mapping);
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);
}

/// Returns the deviation check result for an asset.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset to check.
///
/// # Returns
/// Last recorded price if deviation alert was triggered, 0 otherwise.
pub fn get_last_alert_price(env: &Env, asset: &Address) -> i128 {
    let key = DataKey::AlertLastPrice(asset.clone());
    env.storage().persistent().get(&key).unwrap_or(0i128)
}

/// Inputs for a monthly service-level evidence report.
///
/// All fields are plain operational counters so the report is a pure function
/// of the supplied dataset: identical inputs always produce an identical
/// report, independent of ledger state or wall-clock time.
#[derive(Clone)]
pub struct ServiceLevelInputs {
    /// Number of price updates expected in the reporting window.
    pub expected_updates: u32,
    /// Number of price updates actually observed in the window.
    pub observed_updates: u32,
    /// Number of freshness checks performed.
    pub freshness_checks: u32,
    /// Number of freshness checks that passed within the staleness bound.
    pub freshness_passes: u32,
    /// Number of availability probes performed.
    pub availability_probes: u32,
    /// Number of availability probes that succeeded.
    pub availability_successes: u32,
    /// Number of published prices that were later corrected.
    pub published_prices: u32,
    /// Number of corrections applied to published prices.
    pub corrections: u32,
    /// Total incident duration in ledgers across the window.
    pub incident_ledgers: u32,
    /// Number of distinct incidents in the window.
    pub incident_count: u32,
}

/// Deterministic monthly service-level evidence report.
///
/// Rates are expressed in basis points (1% = 100 bps) and computed with
/// integer arithmetic only, so the report is reproducible for a fixed dataset.
/// When a denominator is zero the corresponding rate is reported as 0 and a
/// missing-data warning is emitted instead of silently dividing by zero.
#[derive(Clone)]
pub struct ServiceLevelReport {
    /// Freshness rate in bps: freshness_passes / freshness_checks.
    pub freshness_bps: u32,
    /// Availability rate in bps: availability_successes / availability_probes.
    pub availability_bps: u32,
    /// Correction rate in bps: corrections / published_prices.
    pub correction_rate_bps: u32,
    /// Mean incident time in ledgers: incident_ledgers / incident_count.
    pub mean_incident_ledgers: u32,
    /// Metric definitions, in a fixed order, for the report header.
    pub definitions: Vec<u32>,
    /// Missing-data warnings, in a fixed order, for the report footer.
    pub warnings: Vec<u32>,
}

/// Metric definition codes, in the fixed order they appear in a report.
pub const DEF_FRESHNESS: u32 = 1;
pub const DEF_AVAILABILITY: u32 = 2;
pub const DEF_CORRECTION_RATE: u32 = 3;
pub const DEF_INCIDENT_TIME: u32 = 4;

/// Missing-data warning codes, in the fixed order they appear in a report.
pub const WARN_NO_FRESHNESS_DATA: u32 = 1;
pub const WARN_NO_AVAILABILITY_DATA: u32 = 2;
pub const WARN_NO_CORRECTION_DATA: u32 = 3;
pub const WARN_NO_INCIDENT_DATA: u32 = 4;
pub const WARN_UPDATE_GAP: u32 = 5;

/// Computes a ratio in basis points, returning 0 when the denominator is zero.
fn ratio_bps(numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        return 0;
    }
    ((numerator as u64 * 10_000) / denominator as u64) as u32
}

/// Generates a reproducible monthly service-level evidence report.
///
/// The report is a pure function of `inputs`: for a fixed dataset the metric
/// values, definitions, and warnings are byte-for-byte identical on every call.
/// Metric definitions are always included; missing-data warnings are appended
/// whenever a metric has no underlying observations.
pub fn generate_service_level_report(env: &Env, inputs: &ServiceLevelInputs) -> ServiceLevelReport {
    let freshness_bps = ratio_bps(inputs.freshness_passes, inputs.freshness_checks);
    let availability_bps = ratio_bps(inputs.availability_successes, inputs.availability_probes);
    let correction_rate_bps = ratio_bps(inputs.corrections, inputs.published_prices);
    let mean_incident_ledgers = if inputs.incident_count == 0 {
        0
    } else {
        inputs.incident_ledgers / inputs.incident_count
    };

    // Definitions are always present and emitted in a fixed order.
    let mut definitions = Vec::new(env);
    definitions.push_back(DEF_FRESHNESS);
    definitions.push_back(DEF_AVAILABILITY);
    definitions.push_back(DEF_CORRECTION_RATE);
    definitions.push_back(DEF_INCIDENT_TIME);

    // Warnings are emitted in a fixed order so the report stays deterministic.
    let mut warnings = Vec::new(env);
    if inputs.freshness_checks == 0 {
        warnings.push_back(WARN_NO_FRESHNESS_DATA);
    }
    if inputs.availability_probes == 0 {
        warnings.push_back(WARN_NO_AVAILABILITY_DATA);
    }
    if inputs.published_prices == 0 {
        warnings.push_back(WARN_NO_CORRECTION_DATA);
    }
    if inputs.incident_count == 0 {
        warnings.push_back(WARN_NO_INCIDENT_DATA);
    }
    if inputs.observed_updates < inputs.expected_updates {
        warnings.push_back(WARN_UPDATE_GAP);
    }

    ServiceLevelReport {
        freshness_bps,
        availability_bps,
        correction_rate_bps,
        mean_incident_ledgers,
        definitions,
        warnings,
    }
}
