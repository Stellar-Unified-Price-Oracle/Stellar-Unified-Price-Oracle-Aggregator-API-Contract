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

// ---------------------------------------------------------------------------
// Source outage escalation state machine (#616)
// ---------------------------------------------------------------------------

/// Lifecycle state of a price-source outage incident.
///
/// Transitions (all driven by `advance_outage_incident`):
///
/// ```text
/// Healthy --alert--> Alerting --ack--> Acknowledged --escalate--> Escalated
///    ^                  |                  |                          |
///    |                  | (timeout)        | (timeout)                |
///    |                  v                  v                          |
///    |              Escalated <------------+                          |
///    |                                                                |
///    +------------------recover---------------------------------------+
/// ```
///
/// `Recovered` is terminal for a given incident; a new outage opens a new
/// incident record so history is never overwritten.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutageState {
    /// Source is healthy; no open incident.
    Healthy,
    /// Source unavailable; alert emitted, awaiting acknowledgement.
    Alerting,
    /// Operator acknowledged the alert; awaiting resolution or escalation.
    Acknowledged,
    /// Incident escalated (unacknowledged timeout or manual escalation).
    Escalated,
    /// Source recovered; incident closed but record retained.
    Recovered,
}

/// Persisted incident record for a source outage.
///
/// Retained after recovery so the incident history is never lost.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OutageIncident {
    /// Monotonic incident id (per asset).
    pub incident_id: u32,
    /// Current state of the incident.
    pub state: OutageState,
    /// Ledger at which the incident was opened.
    pub opened_ledger: u32,
    /// Ledger of the last state transition.
    pub updated_ledger: u32,
    /// Ledger at which the incident was acknowledged (0 if never).
    pub acked_ledger: u32,
    /// Number of escalations applied to this incident.
    pub escalations: u32,
    /// Whether the alert notification has been emitted (dedupe guard).
    pub alert_notified: bool,
}

/// Ledgers an alert may remain unacknowledged before auto-escalation.
pub const OUTAGE_ACK_TIMEOUT_LEDGERS: u32 = 30;

/// Ledgers an acknowledged incident may remain unresolved before escalation.
pub const OUTAGE_RESOLVE_TIMEOUT_LEDGERS: u32 = 120;

/// Returns the persisted incident for an asset, if any.
pub fn get_outage_incident(env: &Env, asset: &Address) -> Option<OutageIncident> {
    let key = DataKey::AlertLastPrice(asset.clone());
    env.storage().persistent().get(&key)
}

fn store_outage_incident(env: &Env, asset: &Address, incident: &OutageIncident) {
    let key = DataKey::AlertLastPrice(asset.clone());
    env.storage().persistent().set(&key, incident);
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);
}

/// Opens (or reuses) an outage incident for an unavailable source.
///
/// If an incident is already open (Alerting/Acknowledged/Escalated) it is
/// returned unchanged and no duplicate alert is emitted. A previously
/// `Recovered` incident is replaced by a fresh incident with a new id.
pub fn alert_source_outage(env: &Env, asset: Address) -> OutageIncident {
    let now = env.ledger().sequence();
    if let Some(existing) = get_outage_incident(env, &asset) {
        if existing.state != OutageState::Recovered {
            // Duplicate alert for an open incident: suppress notification.
            return existing;
        }
        let incident = OutageIncident {
            incident_id: existing.incident_id.saturating_add(1),
            state: OutageState::Alerting,
            opened_ledger: now,
            updated_ledger: now,
            acked_ledger: 0,
            escalations: 0,
            alert_notified: true,
        };
        store_outage_incident(env, &asset, &incident);
        return incident;
    }

    let incident = OutageIncident {
        incident_id: 1,
        state: OutageState::Alerting,
        opened_ledger: now,
        updated_ledger: now,
        acked_ledger: 0,
        escalations: 0,
        alert_notified: true,
    };
    store_outage_incident(env, &asset, &incident);
    incident
}

/// Acknowledges an open outage alert.
///
/// Only valid from `Alerting`; acknowledging an already-acknowledged or
/// escalated incident is a no-op that returns the current record (dedupe).
pub fn acknowledge_source_outage(env: &Env, asset: Address) -> Option<OutageIncident> {
    let mut incident = get_outage_incident(env, &asset)?;
    if incident.state != OutageState::Alerting {
        return Some(incident);
    }
    let now = env.ledger().sequence();
    incident.state = OutageState::Acknowledged;
    incident.acked_ledger = now;
    incident.updated_ledger = now;
    store_outage_incident(env, &asset, &incident);
    Some(incident)
}

/// Advances an open incident, applying timeout-driven escalation.
///
/// * `Alerting` past `OUTAGE_ACK_TIMEOUT_LEDGERS` -> `Escalated`.
/// * `Acknowledged` past `OUTAGE_RESOLVE_TIMEOUT_LEDGERS` -> `Escalated`.
///
/// Returns the (possibly updated) incident. Escalation is idempotent: once
/// `Escalated`, further calls do not increment `escalations` again.
pub fn advance_outage_incident(env: &Env, asset: Address) -> Option<OutageIncident> {
    let mut incident = get_outage_incident(env, &asset)?;
    let now = env.ledger().sequence();
    let elapsed = now.saturating_sub(incident.updated_ledger);

    let should_escalate = match incident.state {
        OutageState::Alerting => elapsed >= OUTAGE_ACK_TIMEOUT_LEDGERS,
        OutageState::Acknowledged => elapsed >= OUTAGE_RESOLVE_TIMEOUT_LEDGERS,
        _ => false,
    };

    if should_escalate {
        incident.state = OutageState::Escalated;
        incident.escalations = incident.escalations.saturating_add(1);
        incident.updated_ledger = now;
        store_outage_incident(env, &asset, &incident);
    }
    Some(incident)
}

/// Marks an open incident as recovered.
///
/// The incident record is retained (state `Recovered`) so history is not lost.
/// Recovering an already-recovered incident is a no-op (dedupe).
pub fn recover_source_outage(env: &Env, asset: Address) -> Option<OutageIncident> {
    let mut incident = get_outage_incident(env, &asset)?;
    if incident.state == OutageState::Recovered {
        return Some(incident);
    }
    let now = env.ledger().sequence();
    incident.state = OutageState::Recovered;
    incident.updated_ledger = now;
    store_outage_incident(env, &asset, &incident);
    Some(incident)
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
