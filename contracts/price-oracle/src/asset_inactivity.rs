use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::DataKey;
use soroban_sdk::{symbol_short, Address, Env, Vec};

/// Default global inactivity timeout: 0 = disabled.
const DEFAULT_INACTIVITY_TIMEOUT: u32 = 0;

/// Default per-source clock-skew threshold in seconds: 0 = disabled.
const DEFAULT_SKEW_THRESHOLD: u64 = 0;

// --- Configuration ---

/// Sets the global default inactivity timeout in ledgers (admin only).
/// 0 disables auto-deregistration globally.
pub fn set_inactivity_timeout(env: &Env, timeout_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::CfgInactivityTimeout, &timeout_ledgers);
    env.storage().persistent().extend_ttl(
        &DataKey::CfgInactivityTimeout,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

pub fn get_inactivity_timeout(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgInactivityTimeout)
        .unwrap_or(DEFAULT_INACTIVITY_TIMEOUT)
}

/// Sets per-asset inactivity timeout override in ledgers (admin only, 0 = use global).
pub fn set_asset_inactivity_timeout(env: &Env, asset: Address, timeout_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);
    let key = DataKey::AssetInactivityTimeout(asset.clone());
    if timeout_ledgers == 0 {
        env.storage().persistent().remove(&key);
    } else {
        env.storage().persistent().set(&key, &timeout_ledgers);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

pub fn get_asset_inactivity_timeout(env: &Env, asset: &Address) -> u32 {
    let key = DataKey::AssetInactivityTimeout(asset.clone());
    let per_asset: Option<u32> = env.storage().persistent().get(&key);
    match per_asset {
        Some(t) if t > 0 => {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            t
        }
        _ => get_inactivity_timeout(env),
    }
}

// --- Activity tracking ---

/// Called from submit_price to record that an asset was active at this ledger.
pub fn record_asset_activity(env: &Env, asset: &Address) {
    let ledger = env.ledger().sequence();
    let key = DataKey::AssetLastActivity(asset.clone());
    env.storage().persistent().set(&key, &ledger);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn get_asset_last_activity(env: &Env, asset: &Address) -> Option<u32> {
    let key = DataKey::AssetLastActivity(asset.clone());
    let val: Option<u32> = env.storage().persistent().get(&key);
    if val.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    val
}

// --- Inactivity check and deregistration ---

/// Returns true if the asset is inactive (past its timeout threshold).
pub fn is_asset_inactive(env: &Env, asset: &Address) -> bool {
    let timeout = get_asset_inactivity_timeout(env, asset);
    if timeout == 0 {
        return false; // disabled
    }
    let last = match get_asset_last_activity(env, asset) {
        Some(l) => l,
        None => return false, // never had a submission — not tracked yet
    };
    let current = env.ledger().sequence();
    current.saturating_sub(last) >= timeout
}

/// Admin can trigger deregistration of an inactive asset.
/// Emits a warning event if within grace period (20% of timeout remaining),
/// and actually deregisters if past the full timeout.
pub fn check_and_deregister_if_inactive(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::storage::check_registered_asset(env, &asset);

    let timeout = get_asset_inactivity_timeout(env, &asset);
    if timeout == 0 {
        // Auto-deregistration disabled for this asset
        return;
    }

    let last = match get_asset_last_activity(env, &asset) {
        Some(l) => l,
        None => return, // no activity recorded yet
    };

    let current = env.ledger().sequence();
    let idle_ledgers = current.saturating_sub(last);

    // Grace period = 20% of the timeout window
    let grace_start = timeout.saturating_mul(4) / 5;

    if idle_ledgers >= timeout {
        // Fully inactive — auto-deregister
        emit_asset_auto_deregistered(env, &asset, idle_ledgers);
        crate::assets::unregister_asset(env, asset);
    } else if idle_ledgers >= grace_start {
        // Within grace period — emit warning only
        emit_asset_inactivity_warning(env, &asset, idle_ledgers, timeout);
    }
}

// --- Per-source submission conformance scorecard ---

/// Per-source conformance scorecard over an explicit ledger window.
///
/// All metrics are computed from retained per-source events so the result is
/// reproducible: given the same retained events and the same `[from_ledger,
/// to_ledger]` window, the scorecard is deterministic.
///
/// `has_data` distinguishes "no retained events in window" (missing data) from
/// "events present but zero failures" (a clean window). When `has_data` is
/// false, all counts are zero and the validity ratios are reported as 0 —
/// callers must check `has_data` before interpreting the ratios.
#[derive(Clone)]
pub struct SourceScorecard {
    /// Inclusive lower bound of the reporting window (ledger sequence).
    pub from_ledger: u32,
    /// Inclusive upper bound of the reporting window (ledger sequence).
    pub to_ledger: u32,
    /// False when no retained events fall inside the window (missing data).
    pub has_data: bool,
    /// Total submissions observed in the window.
    pub total_submissions: u32,
    /// Submissions accepted in the window.
    pub accepted: u32,
    /// Submissions rejected in the window.
    pub rejected: u32,
    /// Submissions whose payload passed schema validation.
    pub schema_valid: u32,
    /// Submissions whose payload failed schema validation.
    pub schema_invalid: u32,
    /// Submissions whose timestamp passed validity checks.
    pub timestamp_valid: u32,
    /// Submissions whose timestamp failed validity checks.
    pub timestamp_invalid: u32,
    /// Largest gap (in ledgers) between consecutive accepted submissions.
    pub max_gap_ledgers: u32,
    /// Number of gaps that exceeded the configured continuity threshold.
    pub continuity_breaks: u32,
}

/// Computes a per-source conformance scorecard from retained events.
///
/// `events` is the retained per-source submission log (each entry is
/// `(ledger, accepted, schema_ok, timestamp_ok)`), and `continuity_threshold`
/// is the maximum allowed gap in ledgers between consecutive accepted
/// submissions before it counts as a continuity break. Only events whose
/// ledger falls within `[from_ledger, to_ledger]` are considered.
pub fn compute_source_scorecard(
    from_ledger: u32,
    to_ledger: u32,
    events: &Vec<(u32, bool, bool, bool)>,
    continuity_threshold: u32,
) -> SourceScorecard {
    let mut card = SourceScorecard {
        from_ledger,
        to_ledger,
        has_data: false,
        total_submissions: 0,
        accepted: 0,
        rejected: 0,
        schema_valid: 0,
        schema_invalid: 0,
        timestamp_valid: 0,
        timestamp_invalid: 0,
        max_gap_ledgers: 0,
        continuity_breaks: 0,
    };

    let mut last_accepted: Option<u32> = None;
    let len = events.len();
    let mut i = 0;
    while i < len {
        let (ledger, accepted, schema_ok, timestamp_ok) = events.get(i).unwrap();
        i += 1;
        if ledger < from_ledger || ledger > to_ledger {
            continue;
        }
        card.has_data = true;
        card.total_submissions += 1;
        if accepted {
            card.accepted += 1;
        } else {
            card.rejected += 1;
        }
        if schema_ok {
            card.schema_valid += 1;
        } else {
            card.schema_invalid += 1;
        }
        if timestamp_ok {
            card.timestamp_valid += 1;
        } else {
            card.timestamp_invalid += 1;
        }
        if accepted {
            if let Some(prev) = last_accepted {
                let gap = ledger.saturating_sub(prev);
                if gap > card.max_gap_ledgers {
                    card.max_gap_ledgers = gap;
                }
                if continuity_threshold > 0 && gap > continuity_threshold {
                    card.continuity_breaks += 1;
                }
            }
            last_accepted = Some(ledger);
        }
    }

    card
}

// --- Events ---

fn emit_asset_inactivity_warning(env: &Env, asset: &Address, idle_ledgers: u32, timeout: u32) {
    env.events().publish(
        (symbol_short!("inact_wrn"), asset.clone()),
        (idle_ledgers, timeout),
    );
}

fn emit_asset_auto_deregistered(env: &Env, asset: &Address, idle_ledgers: u32) {
    env.events()
        .publish((symbol_short!("auto_dreg"), asset.clone()), (idle_ledgers,));
}

fn emit_source_skew_alert(
    env: &Env,
    source: &Address,
    ledger_skew: u64,
    submission_skew: u64,
    threshold: u64,
) {
    env.events().publish(
        (symbol_short!("skew_alrt"), source.clone()),
        (ledger_skew, submission_skew, threshold),
    );
}
