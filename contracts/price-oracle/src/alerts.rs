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
//! ## Webhook delivery contract (#635)
//! Alert callbacks are the on-chain analogue of webhook deliveries. The delivery
//! contract is:
//!
//! - **Delivery identifier**: every logical alert event carries a stable,
//!   deterministic `delivery_id` derived from `(asset, old_price, new_price,
//!   ledger_sequence)`. The same logical event always yields the same id, so a
//!   redelivery is recognisable by consumers.
//! - **Retry schedule**: a failed callback is retried up to
//!   `MAX_DELIVERY_ATTEMPTS` times with exponential backoff measured in ledgers
//!   (`RETRY_BASE_LEDGERS * 2^(attempt-1)`). Retries are recorded per delivery id.
//! - **Duplicate semantics**: redelivering the same `delivery_id` is idempotent.
//!   Once a delivery is acknowledged (or exhausted), subsequent dispatches for
//!   that id are skipped, so consumers observe exactly one logical downstream
//!   event per id.
//! - **Acknowledgement**: a callback that returns without panicking is treated
//!   as acknowledged. A panic (including a simulated timeout) is a failure and
//!   schedules a redelivery per the retry schedule.
//!
//! ## Storage Layout
//! - `AlertSubscription(consumer, asset)` → `AlertSubscription` struct.
//! - `AlertSubscriptionList` → `Vec<(consumer_addr, asset_addr)>` pairs for enumeration.
//! - `MaxAlertSubscriptions` → global cap (u32).
//! - `AlertSubscriptionTtl` → default TTL in ledgers (u32).
//! - `AlertLastPrice(asset)` → last recorded price (i128) for deviation comparison.
//! - `AlertDelivery(delivery_id)` → `DeliveryRecord` tracking attempts/ack state.

use soroban_sdk::{panic_with_error, Address, Bytes, Env, IntoVal, Symbol, Vec};

use crate::events::{AlertSubscribedEvent, AlertSubscriptionExpiredEvent, AlertTriggeredEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AlertSubscription, DataKey, ErrorCode};

/// Default maximum number of alert subscriptions allowed globally.
pub const DEFAULT_MAX_SUBSCRIPTIONS: u32 = 1_000;
/// Default TTL for alert subscriptions in ledgers (~7 days at 5s/ledger ≈ 120_960).
pub const DEFAULT_SUBSCRIPTION_TTL: u32 = 120_960;
/// Basis-point precision (10_000 = 100%).
pub const BPS_PRECISION: u32 = 10_000;
/// Maximum number of delivery attempts (initial + retries) per delivery id.
pub const MAX_DELIVERY_ATTEMPTS: u32 = 5;
/// Base backoff between retries, in ledgers (doubled per attempt).
pub const RETRY_BASE_LEDGERS: u32 = 10;

// ─── Delivery Contract (#635) ────────────────────────────────────────────────

/// Computes the stable, deterministic delivery identifier for a logical alert
/// event. The same `(asset, old_price, new_price, ledger)` tuple always yields
/// the same id, so redeliveries are recognisable and can be deduplicated.
pub fn compute_delivery_id(
    env: &Env,
    asset: &Address,
    old_price: i128,
    new_price: i128,
    ledger: u32,
) -> Bytes {
    let mut buf = Bytes::new(env);
    buf.extend_from_array(&old_price.to_be_bytes());
    buf.extend_from_array(&new_price.to_be_bytes());
    buf.extend_from_array(&ledger.to_be_bytes());
    // Mix in the asset address bytes so distinct assets never collide.
    let asset_bytes = asset.to_string();
    buf.append(&Bytes::from_slice(env, asset_bytes.to_string().as_bytes()));
    buf
}

/// Returns the backoff (in ledgers) before the given retry attempt.
/// Attempt 1 is the initial delivery (no backoff); attempt `n >= 2` waits
/// `RETRY_BASE_LEDGERS * 2^(n-2)` ledgers.
pub fn retry_backoff_ledgers(attempt: u32) -> u32 {
    if attempt <= 1 {
        return 0;
    }
    let shift = attempt - 2;
    RETRY_BASE_LEDGERS.saturating_mul(1u32.checked_shl(shift).unwrap_or(u32::MAX))
}

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
                continue;
            }
        };

        // Prune expired subscriptions.
        if current_ledger > sub.created_ledger.saturating_add(sub.ttl_ledgers) {
            env.storage().persistent().remove(&sub_key);
            AlertSubscriptionExpiredEvent {
                consumer: sub.consumer.clone(),
                asset: sub.asset.clone(),
            }
            .publish(env);
            any_removed = true;
            continue;
        }

        new_list.push_back(entry);
    }

    if any_removed {
        env.storage().persistent().set(&list_key, &new_list);
        env.storage()
            .persistent()
            .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Reference to a subscription stored in the enumeration list.
#[derive(Clone)]
pub struct AlertSubscriptionRef {
    pub consumer: Address,
    pub asset: Address,
}

fn get_subscription_ttl(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AlertSubscriptionTtl)
        .unwrap_or(DEFAULT_SUBSCRIPTION_TTL)
}

fn get_max_subscriptions(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::MaxAlertSubscriptions)
        .unwrap_or(DEFAULT_MAX_SUBSCRIPTIONS)
}

fn remove_from_subscription_list(env: &Env, consumer: &Address, asset: &Address) {
    let list_key = DataKey::AlertSubscriptionList;
    let list: Vec<AlertSubscriptionRef> = match env.storage().persistent().get(&list_key) {
        Some(l) => l,
        None => return,
    };
    let mut new_list: Vec<AlertSubscriptionRef> = Vec::new(env);
    for i in 0..list.len() {
        let entry = list.get_unchecked(i);
        if entry.consumer == *consumer && entry.asset == *asset {
            continue;
        }
        new_list.push_back(entry);
    }
    env.storage().persistent().set(&list_key, &new_list);
    env.storage()
        .persistent()
        .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}
