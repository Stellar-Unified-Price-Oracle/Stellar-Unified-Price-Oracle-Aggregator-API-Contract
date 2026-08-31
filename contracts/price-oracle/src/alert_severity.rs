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

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{SeverityAlertEvent, SeverityThresholdsSetEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AlertChannel, AlertSeverity, DataKey, ErrorCode, SeverityThresholds};

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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::String as SorobanString;

    fn init(env: &Env) -> Address {
        let admin = Address::generate(env);
        env.ledger().with_mut(|l| l.timestamp = 1000);
        crate::admin::initialize(
            env,
            admin.clone(),
            1,
            100,
            18,
            SorobanString::from_str(env, "Oracle"),
        );
        admin
    }

    #[test]
    fn test_default_thresholds_classification() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);

        let t = get_severity_thresholds(&env);
        assert_eq!(classify(50, &t), AlertSeverity::Info);
        assert_eq!(classify(DEFAULT_WARNING_BPS, &t), AlertSeverity::Warning);
        assert_eq!(classify(DEFAULT_CRITICAL_BPS, &t), AlertSeverity::Critical);
        assert_eq!(
            classify(DEFAULT_EMERGENCY_BPS, &t),
            AlertSeverity::Emergency
        );
    }

    #[test]
    fn test_routing_table() {
        assert_eq!(route_for(AlertSeverity::Info), AlertChannel::Dashboard);
        assert_eq!(route_for(AlertSeverity::Warning), AlertChannel::Dashboard);
        assert_eq!(route_for(AlertSeverity::Critical), AlertChannel::Page);
        assert_eq!(route_for(AlertSeverity::Emergency), AlertChannel::Page);
    }

    #[test]
    fn test_evaluate_and_route_records_last_severity() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);
        let asset = Address::generate(&env);

        assert!(get_last_alert_severity(&env, asset.clone()).is_none());

        let (severity, channel) = evaluate_and_route(&env, &asset, DEFAULT_CRITICAL_BPS + 1);
        assert_eq!(severity, AlertSeverity::Critical);
        assert_eq!(channel, AlertChannel::Page);
        assert_eq!(
            get_last_alert_severity(&env, asset.clone()),
            Some(AlertSeverity::Critical)
        );
    }

    #[test]
    fn test_asset_override_takes_priority_over_global() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);
        let asset = Address::generate(&env);

        set_severity_thresholds(&env, 300, 1_000, 2_500);
        set_asset_severity_thresholds(&env, asset.clone(), 50, 100, 200);

        let effective = get_asset_severity_thresholds(&env, asset.clone());
        assert_eq!(effective.warning_bps, 50);

        let (severity, _) = evaluate_and_route(&env, &asset, 150);
        assert_eq!(severity, AlertSeverity::Critical);
    }

    #[test]
    #[should_panic]
    fn test_invalid_thresholds_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        init(&env);
        set_severity_thresholds(&env, 1_000, 500, 2_000);
    }
}
