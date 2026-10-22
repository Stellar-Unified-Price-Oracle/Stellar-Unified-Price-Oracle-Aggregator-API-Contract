//! # Source Performance Analytics (#204)
//!
//! Provides visibility into source performance metrics including accuracy, timeliness,
//! uptime, and reputation trajectory over configurable rolling windows.
//!
//! Also provides a reproducible monthly service-level evidence report (#625) built
//! from operational data: freshness, availability, correction rate, and incident time.

use soroban_sdk::{Address, Env, Vec};

use crate::events::SourceAnalyticsReportedEvent;
use crate::reputation;
use crate::storage::LEDGER_BUMP;
use crate::types::{DataKey, SourceAnalytics};

/// Computes and returns analytics for a source over the last `num_rounds` submissions.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source address to analyze.
/// * `num_rounds` - Number of recent submissions to consider (rolling window).
///
/// # Returns
/// A [`SourceAnalytics`] struct containing:
/// - `accuracy`: Percentage based on price deviation from median (0–100).
/// - `timeliness`: Percentage based on submission regularity (0–100).
/// - `uptime`: Percentage of time source was active vs inactive (0–100).
/// - `reputation`: Current reputation score (0–100).
/// - `reputation_trajectory`: Change trend (+/- direction).
pub fn get_source_analytics(env: &Env, source: Address, num_rounds: u32) -> SourceAnalytics {
    let reputation_score = reputation::get_reputation(env, source.clone());

    // Get submission count for this source
    let key = DataKey::SourceSubmissionCount(source.clone());
    let submission_count: u32 = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(0);

    // Get accuracy sum for averaging
    let accuracy_key = DataKey::SourceAccuracySum(source.clone());
    let accuracy_sum: i128 = env
        .storage()
        .persistent()
        .get(&accuracy_key)
        .unwrap_or(0);

    // Calculate average accuracy (0–100)
    let accuracy = if submission_count > 0 && num_rounds > 0 {
        let avg = accuracy_sum / (submission_count.min(num_rounds) as i128);
        (avg.clamp(0, 100)) as u32
    } else {
        50 // neutral if no data
    };

    // Calculate timeliness: assume regular submissions = higher timeliness
    // If submission_count > 0, assume timeliness is proportional to submission frequency
    let timeliness = if submission_count > 0 {
        ((submission_count as u32).min(100)) as u32
    } else {
        0
    };

    // Calculate uptime: assume active if submitted in recent rounds
    let uptime = if submission_count > 0 {
        ((submission_count.min(num_rounds) as u32 * 100) / num_rounds.max(1)) as u32
    } else {
        0
    };

    // Compute reputation trajectory: compare old and new reputation
    // For simplicity, we check if reputation is drifting toward 50 (decay) or away from 50 (growth)
    let neutral_reputation = 50i32;
    let reputation_int = reputation_score as i32;
    let reputation_trajectory = if reputation_int > neutral_reputation {
        // Positive reputation: check if growing or decaying
        if accuracy as i32 > neutral_reputation {
            1 // improving
        } else {
            -1 // decaying
        }
    } else if reputation_int < neutral_reputation {
        // Low reputation: check if recovering or degrading
        if accuracy as i32 > reputation_int {
            1 // recovering
        } else {
            -1 // worsening
        }
    } else {
        0 // stable at neutral
    };

    SourceAnalytics {
        accuracy,
        timeliness,
        uptime,
        reputation: reputation_score,
        reputation_trajectory,
    }
}

/// Updates accuracy metrics for a source after a price submission.
///
/// Called internally after aggregation to record the source's accuracy.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source address.
/// * `accuracy_score` - Accuracy score (0–100) for this submission.
pub fn record_submission_accuracy(env: &Env, source: &Address, accuracy_score: u32) {
    // Increment submission count
    let count_key = DataKey::SourceSubmissionCount(source.clone());
    let count: u32 = env
        .storage()
        .persistent()
        .get(&count_key)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&count_key, &(count + 1));

    // Add to accuracy sum
    let sum_key = DataKey::SourceAccuracySum(source.clone());
    let sum: i128 = env
        .storage()
        .persistent()
        .get(&sum_key)
        .unwrap_or(0);
    env.storage()
        .persistent()
        .set(&sum_key, &(sum + accuracy_score as i128));

    // Extend TTL
    env.storage()
        .persistent()
        .extend_ttl(&count_key, 300000, 3600000);
    env.storage()
        .persistent()
        .extend_ttl(&sum_key, 300000, 3600000);

    SourceAnalyticsReportedEvent {
        source: source.clone(),
        accuracy: accuracy_score,
    }
    .publish(env);
}

/// Raw operational inputs for a monthly service-level evidence report.
///
/// All fields are supplied by the operator from operational data so the report
/// is fully reproducible: the same `ServiceLevelInputs` always yields the same
/// [`ServiceLevelReport`].
#[derive(Clone)]
pub struct ServiceLevelInputs {
    /// Number of expected price updates in the reporting window.
    pub expected_updates: u32,
    /// Number of price updates actually observed in the window.
    pub observed_updates: u32,
    /// Number of observed updates that were within the freshness SLA.
    pub fresh_updates: u32,
    /// Number of rounds the oracle was expected to be available.
    pub expected_rounds: u32,
    /// Number of rounds the oracle was actually available.
    pub available_rounds: u32,
    /// Number of published prices that required a correction.
    pub corrected_prices: u32,
    /// Total number of published prices in the window.
    pub total_prices: u32,
    /// Total incident duration in seconds during the window.
    pub incident_seconds: u64,
    /// Total seconds in the reporting window.
    pub window_seconds: u64,
}

/// Deterministic monthly service-level evidence report (#625).
///
/// Percentages are expressed in basis points (0–10000) to avoid floating point
/// and keep the report byte-for-byte reproducible for a fixed dataset.
#[derive(Clone)]
pub struct ServiceLevelReport {
    /// Freshness: `fresh_updates / expected_updates` in basis points.
    pub freshness_bps: u32,
    /// Availability: `available_rounds / expected_rounds` in basis points.
    pub availability_bps: u32,
    /// Correction rate: `corrected_prices / total_prices` in basis points.
    pub correction_rate_bps: u32,
    /// Incident time: `incident_seconds / window_seconds` in basis points.
    pub incident_time_bps: u32,
    /// Human-readable metric definitions, in a stable order.
    pub definitions: Vec<&'static str>,
    /// Missing-data warnings, in a stable order. Empty when data is complete.
    pub warnings: Vec<&'static str>,
}

/// Computes a ratio in basis points, saturating at 10000 (100%).
fn ratio_bps(numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        return 0;
    }
    let scaled = (numerator as u64).saturating_mul(10_000) / denominator as u64;
    scaled.min(10_000) as u32
}

/// Generates a reproducible monthly service-level evidence report (#625).
///
/// The report is deterministic for a fixed [`ServiceLevelInputs`]: it performs
/// only integer arithmetic and emits metric definitions and missing-data
/// warnings in a fixed order. Missing or zero denominators yield a `0` metric
/// plus an explicit warning rather than a silent default.
pub fn generate_service_level_report(
    env: &Env,
    inputs: &ServiceLevelInputs,
) -> ServiceLevelReport {
    let mut warnings: Vec<&'static str> = Vec::new(env);

    if inputs.expected_updates == 0 {
        warnings.push_back("freshness: no expected updates in window");
    }
    if inputs.expected_rounds == 0 {
        warnings.push_back("availability: no expected rounds in window");
    }
    if inputs.total_prices == 0 {
        warnings.push_back("correction rate: no published prices in window");
    }
    if inputs.window_seconds == 0 {
        warnings.push_back("incident time: zero-length reporting window");
    }
    if inputs.observed_updates > inputs.expected_updates {
        warnings.push_back("freshness: observed updates exceed expected");
    }
    if inputs.fresh_updates > inputs.observed_updates {
        warnings.push_back("freshness: fresh updates exceed observed");
    }
    if inputs.available_rounds > inputs.expected_rounds {
        warnings.push_back("availability: available rounds exceed expected");
    }
    if inputs.corrected_prices > inputs.total_prices {
        warnings.push_back("correction rate: corrected prices exceed total");
    }
    if inputs.incident_seconds > inputs.window_seconds {
        warnings.push_back("incident time: incident exceeds window");
    }

    let freshness_bps = ratio_bps(inputs.fresh_updates, inputs.expected_updates);
    let availability_bps = ratio_bps(inputs.available_rounds, inputs.expected_rounds);
    let correction_rate_bps = ratio_bps(inputs.corrected_prices, inputs.total_prices);
    let incident_time_bps = if inputs.window_seconds == 0 {
        0
    } else {
        let scaled = inputs.incident_seconds.saturating_mul(10_000) / inputs.window_seconds;
        scaled.min(10_000) as u32
    };

    let mut definitions: Vec<&'static str> = Vec::new(env);
    definitions.push_back("freshness: fresh_updates / expected_updates (bps)");
    definitions.push_back("availability: available_rounds / expected_rounds (bps)");
    definitions.push_back("correction_rate: corrected_prices / total_prices (bps)");
    definitions.push_back("incident_time: incident_seconds / window_seconds (bps)");

    ServiceLevelReport {
        freshness_bps,
        availability_bps,
        correction_rate_bps,
        incident_time_bps,
        definitions,
        warnings,
    }
}
