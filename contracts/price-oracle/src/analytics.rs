//! # Source Performance Analytics (#204, #547)
//!
//! Provides visibility into source performance metrics including accuracy, timeliness,
//! uptime, and reputation trajectory over configurable rolling windows.
//!
//! Also exposes a consumer-safe explanation view (#706) describing which eligible
//! submissions contributed to an aggregate and why others were excluded.

use soroban_sdk::{contracttype, Address, Env, Vec};

use crate::events::SourceAnalyticsReportedEvent;
use crate::reputation;
use crate::storage::LEDGER_BUMP;
use crate::types::{DataKey, SourceAnalytics};

/// Reason a submission was excluded from an aggregate.
///
/// These are consumer-safe: they never reveal private source credentials.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
#[allow(dead_code)]
pub enum ExclusionReason {
    /// Submission was included in the aggregate.
    Included = 0,
    /// Source is not eligible (e.g. not registered / not active).
    Ineligible = 1,
    /// Submission was stale relative to the aggregation round.
    Stale = 2,
    /// Submission was outside the accepted deviation band.
    OutOfBand = 3,
    /// Source had no recorded submission for the round.
    Missing = 4,
}

/// Per-source explanation of an inclusion/exclusion decision.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceContribution {
    /// Source address (public on-chain identity, not a credential).
    pub source: Address,
    /// Whether this source contributed to the aggregate.
    pub included: bool,
    /// Machine-readable reason for the decision.
    pub reason: ExclusionReason,
}

/// Consumer-safe explanation of an aggregate's contributing sources.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AggregateExplanation {
    /// Number of sources that contributed to the aggregate.
    pub included_count: u32,
    /// Number of sources that were excluded.
    pub excluded_count: u32,
    /// Per-source decisions, in deterministic input order.
    pub contributions: Vec<SourceContribution>,
}

/// Builds a consumer-safe explanation of which eligible submissions contributed
/// to an aggregate and why others were excluded.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `sources` - Candidate sources, in deterministic order.
/// * `eligible` - Parallel flags: `true` if the source was eligible for the round.
/// * `included` - Parallel flags: `true` if the source contributed to the aggregate.
///
/// # Returns
/// An [`AggregateExplanation`] with one [`SourceContribution`] per candidate.
/// The decision is deterministic: given the same inputs the same explanation is
/// produced, and no private source credentials are ever exposed.
pub fn explain_aggregate(
    env: &Env,
    sources: Vec<Address>,
    eligible: Vec<bool>,
    included: Vec<bool>,
) -> AggregateExplanation {
    let mut contributions: Vec<SourceContribution> = Vec::new(env);
    let mut included_count: u32 = 0;
    let mut excluded_count: u32 = 0;

    let len = sources.len();
    let mut i: u32 = 0;
    while i < len {
        let source = sources.get(i).unwrap();
        let is_eligible = eligible.get(i).unwrap_or(false);
        let is_included = included.get(i).unwrap_or(false);

        let (did_include, reason) = if !is_eligible {
            (false, ExclusionReason::Ineligible)
        } else if is_included {
            (true, ExclusionReason::Included)
        } else {
            (false, ExclusionReason::OutOfBand)
        };

        if did_include {
            included_count += 1;
        } else {
            excluded_count += 1;
        }

        contributions.push_back(SourceContribution {
            source,
            included: did_include,
            reason,
        });

        i += 1;
    }

    AggregateExplanation {
        included_count,
        excluded_count,
        contributions,
    }
}

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

/// Records a single retained submission outcome for a source so the conformance
/// scorecard can be reproduced from retained events (#643).
///
/// Each call appends one observation to the source's retained event log. The
/// `ledger` is the ledger sequence at which the submission was observed and is
/// used to scope the scorecard window.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source address that submitted.
/// * `ledger` - Ledger sequence of the submission.
/// * `schema_ok` - Whether the payload passed schema validation.
/// * `timestamp_ok` - Whether the timestamp was within tolerance.
/// * `accepted` - Whether the oracle accepted the submission.
pub fn record_submission_outcome(
    env: &Env,
    source: &Address,
    ledger: u32,
    schema_ok: bool,
    timestamp_ok: bool,
    accepted: bool,
) {
    let key = DataKey::SourceSubmissionOutcome(source.clone(), ledger);
    env.storage().persistent().set(
        &key,
        &(schema_ok, timestamp_ok, accepted),
    );
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, 3600000);
}

/// Builds a per-source submission conformance scorecard over an explicit window
/// `[window_start_ledger, window_end_ledger]` (inclusive) from retained events (#643).
///
/// The result is reproducible: given the same retained events and window, the
/// scorecard is deterministic. `has_data` is `false` when no retained submissions
/// fall in the window, distinguishing missing data from zero failures.
pub fn get_source_conformance_scorecard(
    env: &Env,
    source: Address,
    window_start_ledger: u32,
    window_end_ledger: u32,
) -> SourceConformanceScorecard {
    let mut scorecard = SourceConformanceScorecard {
        window_start_ledger,
        window_end_ledger,
        has_data: false,
        total_submissions: 0,
        accepted: 0,
        rejected: 0,
        schema_valid: 0,
        schema_invalid: 0,
        timestamp_valid: 0,
        timestamp_invalid: 0,
        longest_accept_streak: 0,
        continuity_breaks: 0,
    };

    if window_start_ledger > window_end_ledger {
        return scorecard;
    }

    let mut current_streak: u32 = 0;
    let mut ledger = window_start_ledger;
    while ledger <= window_end_ledger {
        let key = DataKey::SourceSubmissionOutcome(source.clone(), ledger);
        if let Some((schema_ok, timestamp_ok, accepted)) = env
            .storage()
            .persistent()
            .get::<DataKey, (bool, bool, bool)>(&key)
        {
            scorecard.has_data = true;
            scorecard.total_submissions += 1;

            if schema_ok {
                scorecard.schema_valid += 1;
            } else {
                scorecard.schema_invalid += 1;
            }

            if timestamp_ok {
                scorecard.timestamp_valid += 1;
            } else {
                scorecard.timestamp_invalid += 1;
            }

            if accepted {
                scorecard.accepted += 1;
                current_streak += 1;
                if current_streak > scorecard.longest_accept_streak {
                    scorecard.longest_accept_streak = current_streak;
                }
            } else {
                scorecard.rejected += 1;
                if current_streak > 0 {
                    scorecard.continuity_breaks += 1;
                }
                current_streak = 0;
            }
        }

        // Guard against overflow at the top of the u32 range.
        if ledger == u32::MAX {
            break;
        }
        ledger += 1;
    }

    scorecard
}
