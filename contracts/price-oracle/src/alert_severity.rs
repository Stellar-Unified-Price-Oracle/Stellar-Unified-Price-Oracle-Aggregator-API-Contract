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
//!|-------------|-----------------------------------------|-----------|
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
//! ## Price quality status vocabulary
//!
//! Consumers need a stable, machine-readable view of how trustworthy a price is.
//! [`PriceQualityStatus`] defines the canonical vocabulary:
//!
//! | Status        | Meaning                                                        |
//!|---------------|----------------------------------------------------------------|
//! | `Fresh`       | Price is within its freshness window and healthy.              |
//! | `Degraded`    | Price is usable but flagged (e.g. elevated deviation).          |
//! | `Stale`       | Price exists but is past its freshness window.                 |
//! | `Unavailable` | No price is available for the asset.                           |
//! | `Corrected`   | Price was recently corrected after a dispute/override.         |
//!
//! [`get_price_quality_status`] maps the existing freshness/health signals into
//! this vocabulary. Clients MUST handle every variant explicitly and treat any
//! unknown future discriminant as `Unavailable` (fail safe).

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{SeverityAlertEvent, SeverityThresholdsSetEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AlertChannel, AlertSeverity, DataKey, ErrorCode, SeverityThresholds};

/// Default global severity thresholds, in basis points of price movement.
pub const DEFAULT_WARNING_BPS: u32 = 300; // 3%
pub const DEFAULT_CRITICAL_BPS: u32 = 1_000; // 10%
pub const DEFAULT_EMERGENCY_BPS: u32 = 2_500; // 25%

// ─── Price quality status vocabulary ───────────────────────────────────────────

/// Stable machine-readable states describing the quality of a price for consumers.
///
/// Discriminants are part of the public ABI: do not renumber existing variants.
/// New variants may be appended; clients must treat unknown values as
/// [`PriceQualityStatus::Unavailable`].
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum PriceQualityStatus {
    /// Price is within its freshness window and healthy.
    Fresh = 0,
    /// Price is usable but flagged (e.g. elevated deviation).
    Degraded = 1,
    /// Price exists but is past its freshness window.
    Stale = 2,
    /// No price is available for the asset.
    Unavailable = 3,
    /// Price was recently corrected after a dispute/override.
    Corrected = 4,
}

impl PriceQualityStatus {
    /// Stable numeric discriminant for the status (part of the public ABI).
    pub fn as_u32(self) -> u32 {
        self as u32
    }

    /// Decodes a numeric discriminant into a status.
    ///
    /// Unknown future discriminants fail safe to [`PriceQualityStatus::Unavailable`].
    pub fn from_u32(value: u32) -> PriceQualityStatus {
        match value {
            0 => PriceQualityStatus::Fresh,
            1 => PriceQualityStatus::Degraded,
            2 => PriceQualityStatus::Stale,
            3 => PriceQualityStatus::Unavailable,
            4 => PriceQualityStatus::Corrected,
            _ => PriceQualityStatus::Unavailable,
        }
    }
}

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

// ─── Price quality status mapping ──────────────────────────────────────────────

/// Maps the existing freshness/health signals for `asset` into the stable
/// [`PriceQualityStatus`] vocabulary.
///
/// The mapping is conservative: any signal that cannot be positively confirmed
/// as healthy resolves to a lower-trust state, and a missing price resolves to
/// [`PriceQualityStatus::Unavailable`].
///
/// * No price recorded → `Unavailable`.
/// * Price flagged as corrected → `Corrected`.
/// * Price past its freshness window → `Stale`.
/// * Last alert severity at `Warning` or above → `Degraded`.
/// * Otherwise → `Fresh`.
pub fn get_price_quality_status(env: &Env, asset: Address) -> PriceQualityStatus {
    let price_key = DataKey::Price(asset.clone());
    let price: Option<crate::types::PriceData> = env.storage().persistent().get(&price_key);
    let price = match price {
        Some(p) => p,
        None => return PriceQualityStatus::Unavailable,
    };

    if price.corrected {
        return PriceQualityStatus::Corrected;
    }

    let now = env.ledger().timestamp();
    if now > price.timestamp.saturating_add(price.max_age) {
        return PriceQualityStatus::Stale;
    }

    match get_last_alert_severity(env, asset) {
        Some(AlertSeverity::Warning)
        | Some(AlertSeverity::Critical)
        | Some(AlertSeverity::Emergency) => PriceQualityStatus::Degraded,
        _ => PriceQualityStatus::Fresh,
    }
}
