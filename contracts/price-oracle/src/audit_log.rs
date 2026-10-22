//! Audit log for oracle operations.
//!
//! Records individual operational events (including alerts) for audit while
//! grouping repeated alerts into a single active incident per
//! (incident, asset, source) tuple. Individual occurrences are always retained
//! so operators can reconstruct the full history of an alert storm.

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

const EVENTS_KEY: Symbol = symbol_short!("AUD_EVT");
const INCIDENT_KEY: Symbol = symbol_short!("AUD_INC");
const INCIDENT_INDEX_KEY: Symbol = symbol_short!("AUD_IDX");

fn incident_key(incident: &Symbol, asset: &Symbol, source: &Symbol) -> (Symbol, Symbol, Symbol) {
    (incident.clone(), asset.clone(), source.clone())
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
