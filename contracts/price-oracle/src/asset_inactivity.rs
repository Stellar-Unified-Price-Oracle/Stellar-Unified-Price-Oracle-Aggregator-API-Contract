use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::DataKey;
use soroban_sdk::{symbol_short, Address, Env};

/// Default global inactivity timeout: 0 = disabled.
const DEFAULT_INACTIVITY_TIMEOUT: u32 = 0;

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
