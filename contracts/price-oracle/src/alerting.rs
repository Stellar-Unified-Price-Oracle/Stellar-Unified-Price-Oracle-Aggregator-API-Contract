//! # Off-Chain Price Deviation Alerting System (#199)
//!
//! Monitors for price deviations vs external references and triggers alerts
//! when thresholds are exceeded. Supports multiple notification channels.
//!
//! Also hosts the per-asset freshness SLO / error-budget evaluation (#614)
//! and the source-outage escalation state machine (#616).

use soroban_sdk::{Address, Env};

use crate::events::PriceDeviationAlertEvent;
use crate::storage::LEDGER_BUMP;
use crate::types::DataKey;

/// Production asset class used to select a freshness SLO.
///
/// Each class carries its own freshness target and error budget so that
/// high-value assets can be held to a tighter SLO than long-tail assets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetClass {
    /// Major, high-liquidity assets (e.g. XLM, USDC).
    Major,
    /// Mid-cap assets with moderate liquidity.
    MidCap,
    /// Long-tail / low-liquidity assets.
    LongTail,
}

/// Versioned freshness SLO configuration for a single asset class.
///
/// `config_version` is bumped whenever targets change so operators can audit
/// which SLO a given alert was evaluated against.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreshnessSlo {
    /// Configuration schema version.
    pub config_version: u32,
    /// Maximum acceptable age of a price, in ledgers.
    pub freshness_target_ledgers: u32,
    /// Error budget expressed in basis points of allowed staleness breaches
    /// over the evaluation window (10_000 bps = 100%).
    pub error_budget_bps: u32,
}

/// Current freshness SLO configuration version.
pub const FRESHNESS_SLO_CONFIG_VERSION: u32 = 1;

/// Returns the versioned freshness SLO for a production asset class.
///
/// This is the single in-repo source of truth for per-asset-class targets and
/// budgets; it is deterministic and unit-testable.
pub fn freshness_slo_for(class: AssetClass) -> FreshnessSlo {
    match class {
        AssetClass::Major => FreshnessSlo {
            config_version: FRESHNESS_SLO_CONFIG_VERSION,
            freshness_target_ledgers: 5,
            error_budget_bps: 100, // 1% of the window may breach
        },
        AssetClass::MidCap => FreshnessSlo {
            config_version: FRESHNESS_SLO_CONFIG_VERSION,
            freshness_target_ledgers: 15,
            error_budget_bps: 250, // 2.5%
        },
        AssetClass::LongTail => FreshnessSlo {
            config_version: FRESHNESS_SLO_CONFIG_VERSION,
            freshness_target_ledgers: 60,
            error_budget_bps: 500, // 5%
        },
    }
}

/// Result of evaluating an asset's freshness against its configured SLO.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FreshnessEvaluation {
    /// Age of the price in ledgers.
    pub age_ledgers: u32,
    /// Configured freshness target for the asset class.
    pub target_ledgers: u32,
    /// Whether the price breached the freshness target.
    pub breached: bool,
    /// Whether the error budget is exhausted (burn alert should fire).
    pub budget_exhausted: bool,
}

/// Evaluates a price age against the configured freshness SLO for an asset class.
///
/// A price is considered fresh while `age_ledgers <= freshness_target_ledgers`
/// (exactly at target is still within SLO). `breaches` is the number of
/// staleness breaches observed in the current evaluation window and
/// `window_samples` the total samples in that window; the budget is exhausted
/// once the breach ratio exceeds `error_budget_bps`.
///
/// When the budget is exhausted a budget-burn alert is emitted.
pub fn evaluate_freshness(
    env: &Env,
    asset: Address,
    class: AssetClass,
    age_ledgers: u32,
    breaches: u32,
    window_samples: u32,
) -> FreshnessEvaluation {
    let slo = freshness_slo_for(class);
    let breached = age_ledgers > slo.freshness_target_ledgers;

    let budget_exhausted = if window_samples == 0 {
        false
    } else {
        // breach_ratio_bps = breaches / window_samples * 10_000
        let breach_ratio_bps = (breaches as u64 * 10_000) / window_samples as u64;
        breach_ratio_bps > slo.error_budget_bps as u64
    };

    if budget_exhausted {
        record_budget_burn_alert(env, &asset, class, &slo, age_ledgers, breaches, window_samples);
    }

    FreshnessEvaluation {
        age_ledgers,
        target_ledgers: slo.freshness_target_ledgers,
        breached,
        budget_exhausted,
    }
}

/// Records a freshness budget-burn alert for monitoring/indexing.
fn record_budget_burn_alert(
    env: &Env,
    asset: &Address,
    class: AssetClass,
    slo: &FreshnessSlo,
    age_ledgers: u32,
    breaches: u32,
    window_samples: u32,
) {
    let key = DataKey::AlertLastPrice(asset.clone());
    env.storage()
        .persistent()
        .extend_ttl(&key, 300000, LEDGER_BUMP);

    crate::events::FreshnessBudgetBurnEvent {
        asset: asset.clone(),
        asset_class: class as u32,
        config_version: slo.config_version,
        target_ledgers: slo.freshness_target_ledgers,
        error_budget_bps: slo.error_budget_bps,
        age_ledgers,
        breaches,
        window_samples,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
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

    PriceDeviationAlertEvent {
        asset: asset.clone(),
        our_price,
        reference_price,
        deviation_bps,
        ledger: env.ledger().sequence(),
    }
    .publish(env);
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
