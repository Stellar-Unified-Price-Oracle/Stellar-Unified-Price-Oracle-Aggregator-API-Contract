//! # Severity-Aware Alerting
//!
//! Extends the existing price-deviation alerting paths ([`crate::alerts`] on-chain
//! subscriptions and [`crate::alerting`] off-chain reference-price checks) with a
//! severity taxonomy so operators can triage anomalies instead of treating every
//! deviation identically.
//!
//! ## Severity taxonomy
//!
//! Movement is classified against configurable basis-point thresholds:
//!
//! | Severity    | Condition                              | Channel   |
//! |-------------|-----------------------------------------|-----------|
//! | `Info`      | `movement_bps < warning_bps`             | —         |
//! | `Warning`   | `warning_bps <= movement_bps < critical_bps`   | Dashboard |
//! | `Critical`  | `critical_bps <= movement_bps < emergency_bps` | Page      |
//! | `Emergency` | `movement_bps >= emergency_bps`          | Page      |
//!
//! Thresholds default globally and can be overridden per asset. Every classification
//! emits a [`SeverityAlertEvent`], which off-chain relayers subscribe to and forward
//! to the routed channel (a dashboard feed for `Warning`, a paging integration for
//! `Critical`/`Emergency`) — see `docs/monitoring/README.md`.
//!
//! `Info`-level movements are recorded (so `get_last_alert_severity` stays current)
//! but are not routed anywhere; they exist purely so operators can distinguish "no
//! alert has ever fired" from "the last alert was below the warning line".
//!
//! ## Alert deduplication & grouping
//!
//! A storm of equivalent alerts must not page an operator once per occurrence.
//! Alerts are grouped by the `(incident, asset, source)` triple: the first alert in
//! a group opens an active incident, and every subsequent equivalent alert is folded
//! into that incident, incrementing its event count while preserving each individual
//! occurrence (ledger + movement) for audit. See [`record_alert`], [`get_incident`],
//! and [`get_incident_occurrence`].

use soroban_sdk::{panic_with_error, Address, Env, Symbol, Vec};

use crate::events::{SeverityAlertEvent, SeverityThresholdsSetEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    AlertChannel, AlertIncident, AlertOccurrence, AlertSeverity, DataKey, ErrorCode,
    SeverityThresholds,
};

/// Default global severity thresholds, in basis points of price movement.
pub const DEFAULT_WARNING_BPS: u32 = 300; // 3%
pub const DEFAULT_CRITICAL_BPS: u32 = 1_000; // 10%
pub const DEFAULT_EMERGENCY_BPS: u32 = 2_500; // 25%

fn default_thresholds() -> SeverityThresholds {
    SeverityThresholds {
        warning_bps: DEFAULT_WARNING_BPS,
        critical_bps: DEFAULT_CRITICAL_BPS,
        emergency_bps: DEFAULT_EMERGENCY_BPS,
    }
}

fn validate_thresholds(env: &Env, thresholds: &SeverityThresholds) {
    if thresholds.warning_bps == 0
        || thresholds.warning_bps >= thresholds.critical_bps
        || thresholds.critical_bps >= thresholds.emergency_bps
    {
        panic_with_error!(env, ErrorCode::InvalidSeverityThresholds);
    }
}

// ─── Configuration ─────────────────────────────────────────────────────────────

/// Sets the global default severity thresholds (basis points). Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidSeverityThresholds`] — thresholds are not strictly
///   increasing, or `warning_bps == 0`.
pub fn set_severity_thresholds(env: &Env, warning_bps: u32, critical_bps: u32, emergency_bps: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let thresholds = SeverityThresholds {
        warning_bps,
        critical_bps,
        emergency_bps,
    };
    validate_thresholds(env, &thresholds);

    let key = DataKey::CfgSeverityThresholds;
    env.storage().persistent().set(&key, &thresholds);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SeverityThresholdsSetEvent {
        is_asset_override: false,
        warning_bps,
        critical_bps,
        emergency_bps,
    }
    .publish(env);
}

/// Returns the global default severity thresholds.
pub fn get_severity_thresholds(env: &Env) -> SeverityThresholds {
    env.storage()
        .persistent()
        .get(&DataKey::CfgSeverityThresholds)
        .unwrap_or_else(default_thresholds)
}

/// Sets a per-asset severity threshold override. Admin-only.
///
/// # Panics
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidSeverityThresholds`] — thresholds are not strictly
///   increasing, or `warning_bps == 0`.
pub fn set_asset_severity_thresholds(
    env: &Env,
    asset: Address,
    warning_bps: u32,
    critical_bps: u32,
    emergency_bps: u32,
) {
    let admin = get_admin(env);
    admin.require_auth();

    let thresholds = SeverityThresholds {
        warning_bps,
        critical_bps,
        emergency_bps,
    };
    validate_thresholds(env, &thresholds);

    let key = DataKey::AssetSeverityThresholds(asset);
    env.storage().persistent().set(&key, &thresholds);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SeverityThresholdsSetEvent {
        is_asset_override: true,
        warning_bps,
        critical_bps,
        emergency_bps,
    }
    .publish(env);
}

/// Returns the effective severity thresholds for `asset`: the per-asset override
/// if one is configured, otherwise the global default.
pub fn get_asset_severity_thresholds(env: &Env, asset: Address) -> SeverityThresholds {
    let key = DataKey::AssetSeverityThresholds(asset);
    if let Some(t) = env.storage().persistent().get(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        t
    } else {
        get_severity_thresholds(env)
    }
}

// ─── Classification & routing ───────────────────────────────────────────────────

/// Classifies a basis-point price movement against a set of thresholds.
pub fn classify(movement_bps: u32, thresholds: &SeverityThresholds) -> AlertSeverity {
    if movement_bps >= thresholds.emergency_bps {
        AlertSeverity::Emergency
    } else if movement_bps >= thresholds.critical_bps {
        AlertSeverity::Critical
    } else if movement_bps >= thresholds.warning_bps {
        AlertSeverity::Warning
    } else {
        AlertSeverity::Info
    }
}

/// Maps a severity level to the channel it should be routed to.
///
/// `Warning` goes to the dashboard; `Critical` and `Emergency` page an on-call
/// operator. `Info` is not routed anywhere by the caller (see `evaluate_and_route`).
pub fn route_for(severity: AlertSeverity) -> AlertChannel {
    match severity {
        AlertSeverity::Info | AlertSeverity::Warning => AlertChannel::Dashboard,
        AlertSeverity::Critical | AlertSeverity::Emergency => AlertChannel::Page,
    }
}

/// Classifies `movement_bps` for `asset`, records it, and emits a
/// [`SeverityAlertEvent`] when the severity is `Warning` or above.
///
/// Called internally from [`crate::alerts::dispatch_alerts`] and
/// [`crate::alerting::check_and_alert_deviation`] after each computes a movement
/// in basis points, so both the on-chain subscription path and the off-chain
/// reference-price path share one severity taxonomy and routing table.
///
/// Returns the classified `(severity, channel)` pair so callers can, e.g., only
/// invoke expensive callback dispatch for `Critical`/`Emergency` movements.
pub fn evaluate_and_route(
    env: &Env,
    asset: &Address,
    movement_bps: u32,
) -> (AlertSeverity, AlertChannel) {
    let thresholds = get_asset_severity_thresholds(env, asset.clone());
    let severity = classify(movement_bps, &thresholds);
    let channel = route_for(severity);

    let key = DataKey::LastAlertSeverity(asset.clone());
    env.storage().persistent().set(&key, &severity);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    if severity != AlertSeverity::Info {
        SeverityAlertEvent {
            asset: asset.clone(),
            severity: severity as u32,
            channel: channel as u32,
            movement_bps,
            ledger: env.ledger().sequence(),
        }
        .publish(env);
    }

    (severity, channel)
}

/// Returns the most recently classified severity for `asset`, or `None` if no
/// movement has ever been evaluated for it.
pub fn get_last_alert_severity(env: &Env, asset: Address) -> Option<AlertSeverity> {
    env.storage()
        .persistent()
        .get(&DataKey::LastAlertSeverity(asset))
}

// ─── Deduplication & grouping ───────────────────────────────────────────────────

/// Records an alert occurrence, grouping it into an active incident keyed by
/// `(incident, asset, source)`.
///
/// The first occurrence in a group opens a new active incident with an event
/// count of `1`. Every subsequent equivalent occurrence is folded into the same
/// incident: the count is incremented and the individual occurrence is appended
/// to the incident's audit trail (see [`get_incident_occurrence`]). This means a
/// storm of equivalent alerts yields exactly one active incident while retaining
/// an accurate event count and per-occurrence audit records.
///
/// Returns the incident id (the `incident` symbol) so callers can correlate.
pub fn record_alert(
    env: &Env,
    incident: Symbol,
    asset: Address,
    source: Symbol,
    severity: AlertSeverity,
    movement_bps: u32,
) -> Symbol {
    let key = DataKey::AlertIncident(incident.clone(), asset.clone(), source.clone());

    let mut record = match env.storage().persistent().get::<DataKey, AlertIncident>(&key) {
        Some(existing) => existing,
        None => AlertIncident {
            incident: incident.clone(),
            asset: asset.clone(),
            source: source.clone(),
            severity,
            event_count: 0,
            first_ledger: env.ledger().sequence(),
            last_ledger: env.ledger().sequence(),
            active: true,
        },
    };

    // Preserve the individual occurrence for audit before folding it in.
    let occurrence = AlertOccurrence {
        ledger: env.ledger().sequence(),
        severity: severity as u32,
        movement_bps,
    };
    let occ_key = DataKey::AlertOccurrence(
        incident.clone(),
        asset.clone(),
        source.clone(),
        record.event_count,
    );
    env.storage().persistent().set(&occ_key, &occurrence);
    env.storage()
        .persistent()
        .extend_ttl(&occ_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    record.event_count = record.event_count.saturating_add(1);
    record.last_ledger = env.ledger().sequence();
    // Escalate the incident's severity to the highest seen in the group.
    if (severity as u32) > (record.severity as u32) {
        record.severity = severity;
    }

    env.storage().persistent().set(&key, &record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    incident
}

/// Returns the grouped incident for `(incident, asset, source)`, or `None` if no
/// alert has ever been recorded for that group.
pub fn get_incident(
    env: &Env,
    incident: Symbol,
    asset: Address,
    source: Symbol,
) -> Option<AlertIncident> {
    let key = DataKey::AlertIncident(incident, asset, source);
    let record = env.storage().persistent().get::<DataKey, AlertIncident>(&key);
    if record.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    record
}

/// Returns the individual occurrence at `index` within a group's audit trail, or
/// `None` if it was never recorded.
pub fn get_incident_occurrence(
    env: &Env,
    incident: Symbol,
    asset: Address,
    source: Symbol,
    index: u32,
) -> Option<AlertOccurrence> {
    let key = DataKey::AlertOccurrence(incident, asset, source, index);
    env.storage().persistent().get(&key)
}

/// Returns the number of active incidents for `asset` across all sources.
///
/// Used by operators to confirm that a storm of equivalent alerts collapsed into
/// a single active incident rather than one per occurrence.
pub fn count_active_incidents(env: &Env, asset: Address) -> u32 {
    let key = DataKey::ActiveIncidentCount(asset);
    env.storage().persistent().get(&key).unwrap_or(0)
}

/// Marks the incident for `(incident, asset, source)` as resolved, decrementing
/// the active-incident counter for the asset. Idempotent: resolving an already
/// resolved (or unknown) incident is a no-op.
pub fn resolve_incident(env: &Env, incident: Symbol, asset: Address, source: Symbol) {
    let key = DataKey::AlertIncident(incident, asset.clone(), source);
    if let Some(mut record) = env.storage().persistent().get::<DataKey, AlertIncident>(&key) {
        if record.active {
            record.active = false;
            env.storage().persistent().set(&key, &record);
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

            let count_key = DataKey::ActiveIncidentCount(asset);
            let current: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
            env.storage()
                .persistent()
                .set(&count_key, &current.saturating_sub(1));
            env.storage()
                .persistent()
                .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
    }
}

/// Returns the audit trail of occurrences for a group as a `Vec`, in recording
/// order. Convenience wrapper over [`get_incident_occurrence`].
pub fn get_incident_occurrences(
    env: &Env,
    incident: Symbol,
    asset: Address,
    source: Symbol,
) -> Vec<AlertOccurrence> {
    let mut out = Vec::new(env);
    let count = match get_incident(env, incident.clone(), asset.clone(), source.clone()) {
        Some(record) => record.event_count,
        None => 0,
    };
    let mut i = 0;
    while i < count {
        if let Some(occ) = get_incident_occurrence(
            env,
            incident.clone(),
            asset.clone(),
            source.clone(),
            i,
        ) {
            out.push_back(occ);
        }
        i += 1;
    }
    out
}
