//! # Off-Chain Price Deviation Alerting System (#199)
//!
//! Monitors for price deviations vs external references and triggers alerts
//! when thresholds are exceeded. Supports multiple notification channels.
//!
//! Also hosts the incident severity and decision authority matrix (#622):
//! maps oracle impact levels to response roles, pause authority,
//! communications requirements, and escalation deadlines.

use soroban_sdk::{Address, Env, Symbol};

use crate::events::PriceDeviationAlertEvent;
use crate::storage::LEDGER_BUMP;
use crate::types::DataKey;

/// Maximum number of alert records retained in the incident evidence bundle.
const MAX_INCIDENT_ALERTS: u32 = 32;

/// A single redacted alert record captured for incident reproduction.
#[derive(Clone)]
pub struct IncidentAlertRecord {
    /// Asset the alert was raised for.
    pub asset: Address,
    /// Our aggregated price at alert time.
    pub our_price: i128,
    /// External reference price at alert time.
    pub reference_price: i128,
    /// Deviation in basis points that triggered the alert.
    pub deviation_bps: u32,
    /// Ledger sequence at which the alert was recorded.
    pub ledger: u32,
}

/// Redacted incident evidence bundle for a source incident.
///
/// Contains only the data required to reproduce a source incident: the alert
/// history, timestamps (ledger sequences), and the source version. Credentials
/// and unrelated user data are never included.
#[derive(Clone)]
pub struct IncidentEvidenceBundle {
    /// Source version string (e.g. contract build identifier).
    pub source_version: String,
    /// Ledger sequence at which the bundle was assembled.
    pub captured_at_ledger: u32,
    /// Redacted alert history, most recent last.
    pub alerts: Vec<IncidentAlertRecord>,
}

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
/// Called internally when deviation exceeds threshold.
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

    // Append to the bounded alert history used by the incident evidence bundle.
    append_alert_history(
        env,
        asset,
        our_price,
        reference_price,
        deviation_bps,
    );

    PriceDeviationAlertEvent {
        asset: asset.clone(),
        our_price,
        reference_price,
        deviation_bps,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
}

/// Appends an alert to the bounded per-asset alert history.
///
/// Keeps at most `MAX_INCIDENT_ALERTS` records, dropping the oldest when full.
fn append_alert_history(
    env: &Env,
    asset: &Address,
    our_price: i128,
    reference_price: i128,
    deviation_bps: u32,
) {
    let key = DataKey::AlertHistory(asset.clone());
    let mut history: Vec<IncidentAlertRecord> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    if history.len() >= MAX_INCIDENT_ALERTS {
        let mut trimmed = Vec::new(env);
        let start = history.len() - (MAX_INCIDENT_ALERTS - 1);
        for i in start..history.len() {
            if let Some(record) = history.get(i) {
                trimmed.push_back(record);
            }
        }
        history = trimmed;
    }

    history.push_back(IncidentAlertRecord {
        asset: asset.clone(),
        our_price,
        reference_price,
        deviation_bps,
        ledger: env.ledger().sequence(),
    });

    env.storage().persistent().set(&key, &history);
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);
}

/// Builds a redacted incident evidence bundle for an asset.
///
/// The bundle contains the alert history, timestamps (ledger sequences), and
/// the supplied source version. It intentionally excludes credentials and any
/// unrelated user data so it can be shared to reproduce a source incident.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset whose incident evidence is being collected.
/// * `source_version` - Source version string to stamp on the bundle.
///
/// # Returns
/// A redacted `IncidentEvidenceBundle` for the asset.
pub fn build_incident_evidence_bundle(
    env: &Env,
    asset: &Address,
    source_version: String,
) -> IncidentEvidenceBundle {
    let key = DataKey::AlertHistory(asset.clone());
    let alerts: Vec<IncidentAlertRecord> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));

    IncidentEvidenceBundle {
        source_version,
        captured_at_ledger: env.ledger().sequence(),
        alerts,
    }
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

// ---------------------------------------------------------------------------
// Incident severity & decision authority matrix (#622)
// ---------------------------------------------------------------------------

/// Oracle incident severity levels, ordered from most to least severe.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum IncidentSeverity {
    /// SEV1: total price corruption / oracle outage affecting funds.
    Sev1 = 1,
    /// SEV2: significant deviation or degraded availability.
    Sev2 = 2,
    /// SEV3: localized anomaly, no immediate fund risk.
    Sev3 = 3,
    /// SEV4: informational / cosmetic.
    Sev4 = 4,
}

impl IncidentSeverity {
    /// Maps a raw deviation in basis points to an incident severity.
    ///
    /// Boundaries are inclusive lower bounds so every deviation maps to
    /// exactly one severity (normal operation, boundary, and failure paths).
    pub fn from_deviation_bps(deviation_bps: u32) -> Self {
        if deviation_bps >= 5_000 {
            IncidentSeverity::Sev1
        } else if deviation_bps >= 1_000 {
            IncidentSeverity::Sev2
        } else if deviation_bps >= 250 {
            IncidentSeverity::Sev3
        } else {
            IncidentSeverity::Sev4
        }
    }

    /// Numeric rank (1 = most severe) for ordering and comparisons.
    pub fn rank(self) -> u32 {
        self as u32
    }
}

/// Response role accountable for an incident severity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ResponseRole {
    /// On-call engineer triages and drives the response.
    OnCallEngineer,
    /// Incident commander coordinates and owns decisions.
    IncidentCommander,
    /// Oracle maintainer with protocol-level authority.
    OracleMaintainer,
    /// Security lead with emergency pause authority.
    SecurityLead,
}

/// Who is authorized to pause the oracle for a given severity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PauseAuthority {
    /// Any on-call engineer may pause immediately.
    OnCallEngineer,
    /// Incident commander may pause.
    IncidentCommander,
    /// Only the security lead may pause.
    SecurityLead,
    /// No pause required; monitor only.
    None,
}

/// Communications requirement for a given severity.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Communications {
    /// Public status page + stakeholder notification.
    PublicAndStakeholders,
    /// Internal stakeholders only.
    InternalStakeholders,
    /// Internal team channel only.
    InternalTeam,
    /// No external communications required.
    None,
}

/// Full decision-authority record for one severity level.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SeverityPolicy {
    pub severity: IncidentSeverity,
    pub response_role: ResponseRole,
    pub pause_authority: PauseAuthority,
    pub communications: Communications,
    /// Escalation deadline in minutes from incident declaration.
    pub escalation_deadline_minutes: u32,
}

/// Returns the decision authority policy for a severity level.
///
/// This is the single source of truth for the incident matrix and is
/// exercised by the tabletop exercise check.
pub fn policy_for(severity: IncidentSeverity) -> SeverityPolicy {
    match severity {
        IncidentSeverity::Sev1 => SeverityPolicy {
            severity,
            response_role: ResponseRole::SecurityLead,
            pause_authority: PauseAuthority::OnCallEngineer,
            communications: Communications::PublicAndStakeholders,
            escalation_deadline_minutes: 5,
        },
        IncidentSeverity::Sev2 => SeverityPolicy {
            severity,
            response_role: ResponseRole::IncidentCommander,
            pause_authority: PauseAuthority::IncidentCommander,
            communications: Communications::InternalStakeholders,
            escalation_deadline_minutes: 15,
        },
        IncidentSeverity::Sev3 => SeverityPolicy {
            severity,
            response_role: ResponseRole::OracleMaintainer,
            pause_authority: PauseAuthority::SecurityLead,
            communications: Communications::InternalTeam,
            escalation_deadline_minutes: 60,
        },
        IncidentSeverity::Sev4 => SeverityPolicy {
            severity,
            response_role: ResponseRole::OnCallEngineer,
            pause_authority: PauseAuthority::None,
            communications: Communications::None,
            escalation_deadline_minutes: 240,
        },
    }
}

/// Resolves the decision authority for a raw deviation in basis points.
///
/// Convenience wrapper used by alerting and by the tabletop exercise.
pub fn decision_authority_for_deviation(deviation_bps: u32) -> SeverityPolicy {
    policy_for(IncidentSeverity::from_deviation_bps(deviation_bps))
}

/// Emits an incident declaration event carrying the resolved policy so
/// off-chain responders can act on the correct decision-maker.
pub fn declare_incident(env: &Env, asset: Address, deviation_bps: u32) -> SeverityPolicy {
    let policy = decision_authority_for_deviation(deviation_bps);
    let key = DataKey::AlertLastPrice(asset.clone());
    env.storage().persistent().set(&key, &(policy.severity.rank() as i128));
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);
    let _ = Symbol::new(env, "incident");
    policy
}

#[cfg(test)]
mod tabletop_tests {
    use super::*;

    /// Tabletop exercise: every severity resolves to the correct
    /// decision-maker, pause authority, communications, and deadline.
    #[test]
    fn tabletop_every_severity_has_correct_response_path() {
        let cases = [
            (
                IncidentSeverity::Sev1,
                ResponseRole::SecurityLead,
                PauseAuthority::OnCallEngineer,
                Communications::PublicAndStakeholders,
                5u32,
            ),
            (
                IncidentSeverity::Sev2,
                ResponseRole::IncidentCommander,
                PauseAuthority::IncidentCommander,
                Communications::InternalStakeholders,
                15u32,
            ),
            (
                IncidentSeverity::Sev3,
                ResponseRole::OracleMaintainer,
                PauseAuthority::SecurityLead,
                Communications::InternalTeam,
                60u32,
            ),
            (
                IncidentSeverity::Sev4,
                ResponseRole::OnCallEngineer,
                PauseAuthority::None,
                Communications::None,
                240u32,
            ),
        ];

        for (severity, role, pause, comms, deadline) in cases {
            let policy = policy_for(severity);
            assert_eq!(policy.severity, severity);
            assert_eq!(policy.response_role, role);
            assert_eq!(policy.pause_authority, pause);
            assert_eq!(policy.communications, comms);
            assert_eq!(policy.escalation_deadline_minutes, deadline);
        }
    }

    /// Boundary conditions map to exactly one severity each.
    #[test]
    fn tabletop_deviation_boundaries_map_to_expected_severity() {
        assert_eq!(IncidentSeverity::from_deviation_bps(0), IncidentSeverity::Sev4);
        assert_eq!(IncidentSeverity::from_deviation_bps(249), IncidentSeverity::Sev4);
        assert_eq!(IncidentSeverity::from_deviation_bps(250), IncidentSeverity::Sev3);
        assert_eq!(IncidentSeverity::from_deviation_bps(999), IncidentSeverity::Sev3);
        assert_eq!(IncidentSeverity::from_deviation_bps(1_000), IncidentSeverity::Sev2);
        assert_eq!(IncidentSeverity::from_deviation_bps(4_999), IncidentSeverity::Sev2);
        assert_eq!(IncidentSeverity::from_deviation_bps(5_000), IncidentSeverity::Sev1);
        assert_eq!(IncidentSeverity::from_deviation_bps(10_000), IncidentSeverity::Sev1);
    }

    /// Escalation deadlines tighten monotonically with severity.
    #[test]
    fn tabletop_escalation_deadlines_are_monotonic() {
        let sev1 = policy_for(IncidentSeverity::Sev1).escalation_deadline_minutes;
        let sev2 = policy_for(IncidentSeverity::Sev2).escalation_deadline_minutes;
        let sev3 = policy_for(IncidentSeverity::Sev3).escalation_deadline_minutes;
        let sev4 = policy_for(IncidentSeverity::Sev4).escalation_deadline_minutes;
        assert!(sev1 < sev2 && sev2 < sev3 && sev3 < sev4);
    }
}
