use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, DeviationReport, ErrorCode};
use soroban_sdk::{panic_with_error, Address, Env};

/// Sets per-source deviation tolerance in basis points (admin only).
/// Pass `tolerance_bps = 0` to clear the per-source override (falls back to global).
pub fn set_source_deviation_tolerance(env: &Env, source: Address, tolerance_bps: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    // Validate source is registered
    let is_source: bool = env
        .storage()
        .persistent()
        .get(&DataKey::SrcActive(source.clone()))
        .unwrap_or(false);
    if !is_source {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    // Max 10000 bps = 100%
    if tolerance_bps > 10_000 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DataKey::SrcDeviationTolerance(source.clone());
    if tolerance_bps == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &tolerance_bps);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

/// Returns per-source deviation tolerance in bps, or None if not set (use global).
pub fn get_source_deviation_tolerance(env: &Env, source: &Address) -> Option<u32> {
    let key = DataKey::SrcDeviationTolerance(source.clone());
    let val: Option<u32> = env.storage().persistent().get(&key);
    if val.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    val
}

/// Returns the effective deviation tolerance for a source:
/// - per-source value if set
/// - otherwise the global CfgMaxDeviation
pub fn effective_deviation_tolerance(env: &Env, source: &Address) -> u32 {
    if let Some(per_source) = get_source_deviation_tolerance(env, source) {
        return per_source;
    }
    // Fall back to global
    env.storage()
        .persistent()
        .get(&DataKey::CfgMaxDeviation)
        .unwrap_or(500) // default 5%
}

/// Returns a deviation report for a source over the given number of recent rounds.
pub fn get_source_deviation_report(
    env: &Env,
    _source: Address,
    _asset: Address,
    num_rounds: u32,
) -> DeviationReport {
    // Compute deviation statistics from recent price history
    // For now, return a zeroed report — full implementation requires
    // iterating over historical submissions which is expensive.
    DeviationReport {
        avg_deviation_bps: 0,
        max_deviation_bps: 0,
        outlier_count: 0,
        trend: 0,
        num_rounds,
    }
}
