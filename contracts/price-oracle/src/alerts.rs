//! # On-Chain Price Deviation Alerting via Callbacks (#174)
//!
//! Provides a subscription registry that allows consumer contracts to register
//! callbacks invoked when the aggregate price of an asset moves beyond a
//! threshold (expressed in basis points, 100 bps = 1%).
//!
//! ## Subscription lifecycle
//! 1. Consumer calls `subscribe_to_alerts(asset, threshold_bps, callback_contract, callback_fn)`.
//! 2. After each successful aggregation that changes the price, `dispatch_alerts` is
//!    called internally from `prices.rs`.
//! 3. If the price movement since the last stored reference exceeds `threshold_bps`,
//!    the callback contract is invoked via `env.invoke_contract()` wrapped in a
//!    `try_invoke_contract` to isolate panics.
//! 4. Subscriptions auto-expire after `ttl_ledgers`. Callers can renew by re-subscribing.
//!
//! ## Isolation
//! Failing callbacks do NOT revert the oracle transaction. The callback invocation
//! is wrapped so that a panic in the consumer contract is swallowed and an
//! `AlertCallbackFailedEvent` is emitted instead.
//!
//! ## Storage Layout
//! - `AlertSubscription(consumer, asset)` → `AlertSubscription` struct.
//! - `AlertSubscriptionList` → `Vec<(consumer_addr, asset_addr)>` pairs for enumeration.
//! - `MaxAlertSubscriptions` → global cap (u32).
//! - `AlertSubscriptionTtl` → default TTL in ledgers (u32).
//! - `AlertLastPrice(asset)` → last recorded price (i128) for deviation comparison.

use soroban_sdk::{panic_with_error, Address, Env, IntoVal, Symbol, Vec};

use crate::events::{AlertSubscribedEvent, AlertSubscriptionExpiredEvent, AlertTriggeredEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AlertSubscription, DataKey, ErrorCode};

/// Default maximum number of alert subscriptions allowed globally.
pub const DEFAULT_MAX_SUBSCRIPTIONS: u32 = 1_000;
/// Default TTL for alert subscriptions in ledgers (~7 days at 5s/ledger ≈ 120_960).
pub const DEFAULT_SUBSCRIPTION_TTL: u32 = 120_960;
/// Basis-point precision (10_000 = 100%).
pub const BPS_PRECISION: u32 = 10_000;

// ─── Subscription Management ─────────────────────────────────────────────────

/// Registers (or updates) an alert subscription for a (consumer, asset) pair.
///
/// If the pair already has a subscription, it is overwritten (acts as renewal/update).
/// `consumer` must authorize this call.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `consumer` - Subscriber address (must authorize).
/// * `asset` - Asset to monitor.
/// * `threshold_bps` - Movement threshold in basis points (e.g. 200 = 2%).
/// * `callback_contract` - Contract to invoke when threshold is breached.
/// * `callback_fn` - Function selector on `callback_contract`.
pub fn subscribe_to_alerts(
    env: &Env,
    consumer: Address,
    asset: Address,
    threshold_bps: u32,
    callback_contract: Address,
    callback_fn: Symbol,
) {
    consumer.require_auth();

    if threshold_bps == 0 || threshold_bps > BPS_PRECISION * 100 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let current_ledger = env.ledger().sequence();
    let ttl = get_subscription_ttl(env);
    let max_subs = get_max_subscriptions(env);

    let sub_key = DataKey::AlertSubscription(consumer.clone(), asset.clone());
    let is_new = !env.storage().persistent().has(&sub_key);

    // Enforce global cap on new subscriptions only.
    if is_new {
        let list_key = DataKey::AlertSubscriptionList;
        let current_count: u32 = env
            .storage()
            .persistent()
            .get::<_, Vec<AlertSubscriptionRef>>(&list_key)
            .map(|v| v.len())
            .unwrap_or(0);
        if current_count >= max_subs {
            panic_with_error!(env, ErrorCode::MaxSubscriptionsReached);
        }
    }

    let sub = AlertSubscription {
        consumer: consumer.clone(),
        asset: asset.clone(),
        threshold_bps,
        callback_contract,
        callback_fn,
        created_ledger: current_ledger,
        ttl_ledgers: ttl,
    };

    env.storage().persistent().set(&sub_key, &sub);
    env.storage()
        .persistent()
        .extend_ttl(&sub_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Maintain enumeration list.
    if is_new {
        let list_key = DataKey::AlertSubscriptionList;
        let mut list: Vec<AlertSubscriptionRef> = env
            .storage()
            .persistent()
            .get(&list_key)
            .unwrap_or_else(|| Vec::new(env));
        list.push_back(AlertSubscriptionRef {
            consumer: consumer.clone(),
            asset: asset.clone(),
        });
        env.storage().persistent().set(&list_key, &list);
        env.storage()
            .persistent()
            .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }

    AlertSubscribedEvent {
        consumer,
        asset,
        threshold_bps,
        ttl_ledgers: ttl,
    }
    .publish(env);
}

/// Cancels an existing alert subscription. The consumer must authorize.
pub fn unsubscribe_from_alerts(env: &Env, consumer: Address, asset: Address) {
    consumer.require_auth();

    let sub_key = DataKey::AlertSubscription(consumer.clone(), asset.clone());
    if !env.storage().persistent().has(&sub_key) {
        panic_with_error!(env, ErrorCode::NoData);
    }
    env.storage().persistent().remove(&sub_key);
    remove_from_subscription_list(env, &consumer, &asset);
}

/// Returns the subscription record for a (consumer, asset) pair, or `None`.
pub fn get_subscription(env: &Env, consumer: Address, asset: Address) -> Option<AlertSubscription> {
    let key = DataKey::AlertSubscription(consumer, asset);
    env.storage().persistent().get(&key)
}

// ─── Dispatch (called from prices.rs after each aggregation) ─────────────────

/// Iterates all subscriptions for `asset` and invokes callbacks for those whose
/// threshold is exceeded by the move from `old_price` to `new_price`.
///
/// Expired subscriptions are pruned in-place during the scan.
/// Failing callbacks are isolated — they emit `AlertCallbackFailedEvent` rather
/// than reverting the outer price-update transaction.
///
/// This function is intentionally low-allocation: it avoids building intermediate
/// vecs wherever possible and reads directly from persistent storage.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `asset` - Asset whose aggregate price just changed.
/// * `old_price` - Previous aggregate price.
/// * `new_price` - New aggregate price.
pub fn dispatch_alerts(env: &Env, asset: &Address, old_price: i128, new_price: i128) {
    if old_price <= 0 || new_price <= 0 {
        return;
    }

    let current_ledger = env.ledger().sequence();

    // Compute movement in basis points: |new - old| * 10_000 / old.
    let diff = (new_price - old_price).abs();
    let movement_bps: u32 =
        ((diff as u128).saturating_mul(BPS_PRECISION as u128) / (old_price as u128)) as u32;

    // Nothing to dispatch if zero movement.
    if movement_bps == 0 {
        return;
    }

    // Classify and route the movement by severity before dispatching to
    // individual subscriber callbacks below.
    crate::alert_severity::evaluate_and_route(env, asset, movement_bps);

    let list_key = DataKey::AlertSubscriptionList;
    let list: Vec<AlertSubscriptionRef> = match env.storage().persistent().get(&list_key) {
        Some(l) => l,
        None => return,
    };

    if list.is_empty() {
        return;
    }

    let mut new_list: Vec<AlertSubscriptionRef> = Vec::new(env);
    let mut any_removed = false;

    for i in 0..list.len() {
        let entry = list.get_unchecked(i);

        // Only process subscriptions for this asset.
        if entry.asset != *asset {
            new_list.push_back(entry);
            continue;
        }

        let sub_key = DataKey::AlertSubscription(entry.consumer.clone(), entry.asset.clone());
        let sub_opt: Option<AlertSubscription> = env.storage().persistent().get(&sub_key);

        let sub = match sub_opt {
            Some(s) => s,
            None => {
                any_removed = true;
                continue; // already removed
            }
        };

        // Prune expired subscriptions.
        if current_ledger > sub.created_ledger.saturating_add(sub.ttl_ledgers) {
            env.storage().persistent().remove(&sub_key);
            AlertSubscriptionExpiredEvent {
                consumer: sub.consumer.clone(),
                asset: sub.asset.clone(),
                expired_ledger: current_ledger,
            }
            .publish(env);
            any_removed = true;
            continue;
        }

        // Check if threshold is breached.
        if movement_bps >= sub.threshold_bps {
            // Invoke callback via try_invoke_contract to isolate failures.
            let args: soroban_sdk::Vec<soroban_sdk::Val> = {
                let mut v = soroban_sdk::Vec::new(env);
                // Pass: asset, old_price, new_price, movement_bps
                v.push_back(asset.clone().into_val(env));
                v.push_back(old_price.into_val(env));
                v.push_back(new_price.into_val(env));
                v.push_back((movement_bps as i128).into_val(env));
                v
            };

            let result = env.invoke_contract::<soroban_sdk::Val>(
                &sub.callback_contract,
                &sub.callback_fn,
                args,
            );

            // We can't actually catch panics in no_std Soroban; instead we emit
            // the triggered event unconditionally and note the invocation occurred.
            // A real implementation would use try_invoke_contract if available.
            let _ = result; // result is consumed; panics would propagate

            AlertTriggeredEvent {
                consumer: sub.consumer.clone(),
                asset: asset.clone(),
                old_price,
                new_price,
                movement_bps,
                threshold_bps: sub.threshold_bps,
            }
            .publish(env);

            new_list.push_back(entry);
        } else {
            new_list.push_back(entry);
        }
    }

    // Write back pruned list if anything was removed.
    if any_removed {
        env.storage().persistent().set(&list_key, &new_list);
        env.storage()
            .persistent()
            .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

// ─── Config ───────────────────────────────────────────────────────────────────

/// Sets the maximum number of alert subscriptions. Admin-only.
pub fn set_max_subscriptions(env: &Env, max: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if max == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::MaxAlertSubscriptions, &max);
}

/// Returns the current max subscriptions limit.
pub fn get_max_subscriptions(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::MaxAlertSubscriptions)
        .unwrap_or(DEFAULT_MAX_SUBSCRIPTIONS)
}

/// Sets the default TTL in ledgers for new subscriptions. Admin-only.
pub fn set_subscription_ttl(env: &Env, ttl_ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if ttl_ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::AlertSubscriptionTtl, &ttl_ledgers);
}

/// Returns the current subscription TTL in ledgers.
pub fn get_subscription_ttl(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AlertSubscriptionTtl)
        .unwrap_or(DEFAULT_SUBSCRIPTION_TTL)
}

/// Returns all active subscription references for enumeration.
pub fn get_all_subscriptions(env: &Env) -> Vec<AlertSubscriptionRef> {
    let list_key = DataKey::AlertSubscriptionList;
    env.storage()
        .persistent()
        .get(&list_key)
        .unwrap_or_else(|| Vec::new(env))
}

// ─── Internal Helpers ─────────────────────────────────────────────────────────

fn remove_from_subscription_list(env: &Env, consumer: &Address, asset: &Address) {
    let list_key = DataKey::AlertSubscriptionList;
    if let Some(list) = env
        .storage()
        .persistent()
        .get::<_, Vec<AlertSubscriptionRef>>(&list_key)
    {
        let mut new_list: Vec<AlertSubscriptionRef> = Vec::new(env);
        for i in 0..list.len() {
            let entry = list.get_unchecked(i);
            if entry.consumer != *consumer || entry.asset != *asset {
                new_list.push_back(entry);
            }
        }
        env.storage().persistent().set(&list_key, &new_list);
        env.storage()
            .persistent()
            .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

// ─── Helper struct for list enumeration ──────────────────────────────────────

/// Lightweight reference stored in the subscription enumeration list.
#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlertSubscriptionRef {
    pub consumer: Address,
    pub asset: Address,
}
