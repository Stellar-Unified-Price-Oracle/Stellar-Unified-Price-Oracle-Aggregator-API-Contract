//! Audit log for oracle operations.
//!
//! Records individual operational events (including alerts) for audit while
//! grouping repeated alerts into a single active incident per
//! (incident, asset, source) tuple. Individual occurrences are always retained
//! so operators can reconstruct the full history of an alert storm.
//!
//! Also provides a deterministic monthly service-level evidence report
//! (freshness, availability, correction rate, incident time) computed purely
//! from the recorded operational data.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

/// A single recorded audit event.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditEvent {
    pub kind: Symbol,
    pub asset: Symbol,
    pub source: Symbol,
    pub timestamp: u64,
    pub detail: u32,
}

/// A grouped incident aggregating equivalent alerts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Incident {
    pub incident: Symbol,
    pub asset: Symbol,
    pub source: Symbol,
    pub first_seen: u64,
    pub last_seen: u64,
    /// Number of individual occurrences folded into this incident.
    pub event_count: u32,
    pub active: bool,
}

/// Deterministic monthly service-level evidence report.
///
/// All fields are derived purely from recorded operational data for the
/// inclusive window `[window_start, window_end]`, so the report is
/// reproducible for a fixed dataset.
///
/// Metric definitions:
/// - `freshness_events` / `stale_events`: count of `FRESH` / `STALE` events.
/// - `freshness_ratio_bps`: `freshness_events / (freshness_events + stale_events)`
///   in basis points (0..=10_000); 0 when there is no freshness data.
/// - `availability_events` / `unavailable_events`: count of `AVAIL` / `DOWN` events.
/// - `availability_ratio_bps`: `availability_events / (availability_events +
///   unavailable_events)` in basis points; 0 when there is no availability data.
/// - `correction_events`: count of `CORRECT` events.
/// - `correction_rate_bps`: `correction_events / total_events` in basis points;
///   0 when there are no events.
/// - `incident_count`: number of grouped incidents overlapping the window.
/// - `incident_time`: summed duration (seconds) of incidents overlapping the
///   window, clipped to the window bounds.
/// - `missing_data`: bitmask of warnings for absent data sources.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceLevelReport {
    pub window_start: u64,
    pub window_end: u64,
    pub total_events: u32,
    pub freshness_events: u32,
    pub stale_events: u32,
    pub freshness_ratio_bps: u32,
    pub availability_events: u32,
    pub unavailable_events: u32,
    pub availability_ratio_bps: u32,
    pub correction_events: u32,
    pub correction_rate_bps: u32,
    pub incident_count: u32,
    pub incident_time: u64,
    pub missing_data: u32,
}

/// Missing-data warning bit: no freshness events in the window.
pub const MISSING_FRESHNESS: u32 = 1 << 0;
/// Missing-data warning bit: no availability events in the window.
pub const MISSING_AVAILABILITY: u32 = 1 << 1;
/// Missing-data warning bit: no correction events in the window.
pub const MISSING_CORRECTION: u32 = 1 << 2;
/// Missing-data warning bit: no incidents in the window.
pub const MISSING_INCIDENTS: u32 = 1 << 3;

const EVENTS_KEY: Symbol = symbol_short!("AUD_EVT");
const INCIDENT_KEY: Symbol = symbol_short!("AUD_INC");
const INCIDENT_INDEX_KEY: Symbol = symbol_short!("AUD_IDX");

fn incident_key(incident: &Symbol, asset: &Symbol, source: &Symbol) -> (Symbol, Symbol, Symbol) {
    (incident.clone(), asset.clone(), source.clone())
}

fn ratio_bps(numerator: u32, denominator: u32) -> u32 {
    if denominator == 0 {
        return 0;
    }
    ((numerator as u64) * 10_000 / (denominator as u64)) as u32
}

/// Append an individual audit event, preserving every occurrence.
pub fn record_event(env: &Env, event: AuditEvent) {
    let mut events: Vec<AuditEvent> = env
        .storage()
        .persistent()
        .get(&EVENTS_KEY)
        .unwrap_or_else(|| Vec::new(env));
    events.push_back(event);
    env.storage().persistent().set(&EVENTS_KEY, &events);
}

/// Return all recorded audit events.
pub fn events(env: &Env) -> Vec<AuditEvent> {
    env.storage()
        .persistent()
        .get(&EVENTS_KEY)
        .unwrap_or_else(|| Vec::new(env))
}

/// Record an alert, grouping equivalent alerts into one active incident.
///
/// A storm of equivalent alerts (same incident, asset, and source) creates
/// exactly one active incident while incrementing its event count. Each
/// occurrence is still appended to the audit log.
pub fn record_alert(
    env: &Env,
    incident: Symbol,
    asset: Symbol,
    source: Symbol,
    timestamp: u64,
) -> Incident {
    record_event(
        env,
        AuditEvent {
            kind: symbol_short!("ALERT"),
            asset: asset.clone(),
            source: source.clone(),
            timestamp,
            detail: 0,
        },
    );

    let key = incident_key(&incident, &asset, &source);
    let mut grouped: Incident = match env.storage().persistent().get(&(INCIDENT_KEY, key.clone())) {
        Some(existing) => existing,
        None => {
            let mut index: Vec<(Symbol, Symbol, Symbol)> = env
                .storage()
                .persistent()
                .get(&INCIDENT_INDEX_KEY)
                .unwrap_or_else(|| Vec::new(env));
            index.push_back(key.clone());
            env.storage().persistent().set(&INCIDENT_INDEX_KEY, &index);
            Incident {
                incident: incident.clone(),
                asset: asset.clone(),
                source: source.clone(),
                first_seen: timestamp,
                last_seen: timestamp,
                event_count: 0,
                active: true,
            }
        }
    };

    grouped.event_count = grouped.event_count.saturating_add(1);
    grouped.last_seen = timestamp;
    grouped.active = true;

    env.storage()
        .persistent()
        .set(&(INCIDENT_KEY, key), &grouped);
    grouped
}

/// Fetch the grouped incident for a given (incident, asset, source) tuple.
pub fn get_incident(
    env: &Env,
    incident: Symbol,
    asset: Symbol,
    source: Symbol,
) -> Option<Incident> {
    let key = incident_key(&incident, &asset, &source);
    env.storage().persistent().get(&(INCIDENT_KEY, key))
}

/// Return all grouped incidents.
pub fn incidents(env: &Env) -> Vec<Incident> {
    let index: Vec<(Symbol, Symbol, Symbol)> = env
        .storage()
        .persistent()
        .get(&INCIDENT_INDEX_KEY)
        .unwrap_or_else(|| Vec::new(env));
    let mut out: Vec<Incident> = Vec::new(env);
    for key in index.iter() {
        if let Some(inc) = env.storage().persistent().get(&(INCIDENT_KEY, key)) {
            out.push_back(inc);
        }
    }
    out
}

/// Mark a grouped incident as resolved.
pub fn resolve_incident(
    env: &Env,
    incident: Symbol,
    asset: Symbol,
    source: Symbol,
) -> Option<Incident> {
    let key = incident_key(&incident, &asset, &source);
    let mut grouped: Incident = env.storage().persistent().get(&(INCIDENT_KEY, key.clone()))?;
    grouped.active = false;
    env.storage()
        .persistent()
        .set(&(INCIDENT_KEY, key), &grouped);
    Some(grouped)
}

/// Convenience helper used by callers that only have an address source.
pub fn record_alert_from(
    env: &Env,
    incident: Symbol,
    asset: Symbol,
    source: Address,
    timestamp: u64,
) -> Incident {
    let _ = source;
    record_alert(env, incident, asset, symbol_short!("ADDR"), timestamp)
}

/// Build a deterministic monthly service-level evidence report.
///
/// Only events whose `timestamp` falls within the inclusive window
/// `[window_start, window_end]` are considered. Incidents contribute their
/// overlap with the window, clipped to the window bounds. The result is fully
/// determined by the stored data and the window, so it is reproducible for a
/// fixed dataset. Missing data sources are flagged via `missing_data`.
pub fn service_level_report(
    env: &Env,
    window_start: u64,
    window_end: u64,
) -> ServiceLevelReport {
    let fresh = symbol_short!("FRESH");
    let stale = symbol_short!("STALE");
    let avail = symbol_short!("AVAIL");
    let down = symbol_short!("DOWN");
    let correct = symbol_short!("CORRECT");

    let mut total_events: u32 = 0;
    let mut freshness_events: u32 = 0;
    let mut stale_events: u32 = 0;
    let mut availability_events: u32 = 0;
    let mut unavailable_events: u32 = 0;
    let mut correction_events: u32 = 0;

    for event in events(env).iter() {
        if event.timestamp < window_start || event.timestamp > window_end {
            continue;
        }
        total_events = total_events.saturating_add(1);
        if event.kind == fresh {
            freshness_events = freshness_events.saturating_add(1);
        } else if event.kind == stale {
            stale_events = stale_events.saturating_add(1);
        } else if event.kind == avail {
            availability_events = availability_events.saturating_add(1);
        } else if event.kind == down {
            unavailable_events = unavailable_events.saturating_add(1);
        } else if event.kind == correct {
            correction_events = correction_events.saturating_add(1);
        }
    }

    let mut incident_count: u32 = 0;
    let mut incident_time: u64 = 0;
    for inc in incidents(env).iter() {
        if inc.last_seen < window_start || inc.first_seen > window_end {
            continue;
        }
        incident_count = incident_count.saturating_add(1);
        let start = if inc.first_seen < window_start {
            window_start
        } else {
            inc.first_seen
        };
        let end = if inc.last_seen > window_end {
            window_end
        } else {
            inc.last_seen
        };
        if end > start {
            incident_time = incident_time.saturating_add(end - start);
        }
    }

    let freshness_total = freshness_events.saturating_add(stale_events);
    let availability_total = availability_events.saturating_add(unavailable_events);

    let mut missing_data: u32 = 0;
    if freshness_total == 0 {
        missing_data |= MISSING_FRESHNESS;
    }
    if availability_total == 0 {
        missing_data |= MISSING_AVAILABILITY;
    }
    if correction_events == 0 {
        missing_data |= MISSING_CORRECTION;
    }
    if incident_count == 0 {
        missing_data |= MISSING_INCIDENTS;
    }

    ServiceLevelReport {
        window_start,
        window_end,
        total_events,
        freshness_events,
        stale_events,
        freshness_ratio_bps: ratio_bps(freshness_events, freshness_total),
        availability_events,
        unavailable_events,
        availability_ratio_bps: ratio_bps(availability_events, availability_total),
        correction_events,
        correction_rate_bps: ratio_bps(correction_events, total_events),
        incident_count,
        incident_time,
        missing_data,
    }
}
