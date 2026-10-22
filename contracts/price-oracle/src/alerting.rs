//! # Off-Chain Price Deviation Alerting System (#199)
//!
//! Monitors for price deviations vs external references and triggers alerts
//! when thresholds are exceeded. Supports multiple notification channels.
//!
//! Also hosts the per-asset freshness SLO / error-budget evaluation (#614).

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
