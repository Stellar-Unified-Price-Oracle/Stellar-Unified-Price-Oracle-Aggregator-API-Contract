//! Pull-based Price Update Subscription Registry (#305)
//!
//! ## Why pull-based, not push-based?
//!
//! Soroban does **not** support asynchronous cross-contract callbacks.  A
//! contract cannot initiate an outbound invocation to an arbitrary consumer
//! address at an unpredictable future time (e.g. "when the price changes").
//! Any such design would require the contract to store an unbounded list of
//! consumers and iterate over them synchronously inside `submit_price`, which:
//!
//! 1. Inflates per-submission CPU / memory costs linearly with the number of
//!    subscribers.
//! 2. Allows a malicious subscriber to craft a callback that aborts the entire
//!    `submit_price` transaction, effectively censoring all other sources.
//! 3. Has no deterministic upper bound on execution, making reliable fee
//!    estimation impossible.
//!
//! The correct pattern on Soroban is **pull-based**: consumers register interest
//! on-chain, and an **off-chain relayer** reads [`get_subscribed_consumers`] after
//! each price event and dispatches individual cross-contract calls to each
//! registered consumer.  This decouples the oracle's execution cost from the
//! number of subscribers and eliminates the griefing vector.
//!
//! ## Storage layout
//!
//! Each subscription is stored under [`DataKey::PriceUpdateSubscription`]
//! keyed by `(consumer, asset)` as a boolean presence flag.  A parallel index
//! [`DataKey::AssetSubscriberList`] holds the ordered `Vec<Address>` of
//! consumers that have subscribed to a given asset.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{PriceUpdateSubscribedEvent, PriceUpdateUnsubscribedEvent};
use crate::storage::{check_registered_asset, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn subscriber_list_key(asset: &Address) -> DataKey {
    DataKey::AssetSubscriberList(asset.clone())
}

fn subscription_key(consumer: &Address, asset: &Address) -> DataKey {
    DataKey::PriceUpdateSubscription(consumer.clone(), asset.clone())
}

fn read_subscriber_list(env: &Env, asset: &Address) -> Vec<Address> {
    let key = subscriber_list_key(asset);
    let list: Option<Vec<Address>> = env.storage().persistent().get(&key);
    match list {
        Some(v) => {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            v
        }
        None => Vec::new(env),
    }
}

fn write_subscriber_list(env: &Env, asset: &Address, list: &Vec<Address>) {
    let key = subscriber_list_key(asset);
    env.storage().persistent().set(&key, list);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn has_subscription(env: &Env, consumer: &Address, asset: &Address) -> bool {
    let key = subscription_key(consumer, asset);
    let flag: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if flag {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    flag
}

// ---------------------------------------------------------------------------
// Public entry points
// ---------------------------------------------------------------------------

/// Registers `consumer` as interested in price updates for `asset`.
///
/// `consumer` must authorize this call (they are registering their own
/// interest).  The `asset` must already be registered with the oracle.
///
/// If `consumer` has already subscribed to `asset` this function is a no-op
/// (idempotent).
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
pub fn subscribe_price_updates(env: &Env, consumer: Address, asset: Address) {
    consumer.require_auth();
    check_registered_asset(env, &asset);

    let sub_key = subscription_key(&consumer, &asset);

    // Idempotent: do nothing if already subscribed.
    let already: bool = env.storage().persistent().get(&sub_key).unwrap_or(false);
    if already {
        return;
    }

    // Mark the subscription.
    env.storage().persistent().set(&sub_key, &true);
    env.storage()
        .persistent()
        .extend_ttl(&sub_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Append to the asset's subscriber list.
    let mut list = read_subscriber_list(env, &asset);
    list.push_back(consumer.clone());
    write_subscriber_list(env, &asset, &list);

    PriceUpdateSubscribedEvent { consumer, asset }.publish(env);
}

/// Removes `consumer`'s interest registration for `asset`.
///
/// `consumer` must authorize this call.  If no subscription exists this
/// function is a no-op (idempotent).
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
pub fn unsubscribe_price_updates(env: &Env, consumer: Address, asset: Address) {
    consumer.require_auth();
    check_registered_asset(env, &asset);

    let sub_key = subscription_key(&consumer, &asset);

    // Check if subscribed.
    let subscribed: bool = env.storage().persistent().get(&sub_key).unwrap_or(false);
    if !subscribed {
        return;
    }

    // Remove the subscription flag.
    env.storage().persistent().remove(&sub_key);

    // Remove from the asset's subscriber list.
    let list = read_subscriber_list(env, &asset);
    let mut new_list: Vec<Address> = Vec::new(env);
    for i in 0..list.len() {
        let addr = list.get_unchecked(i);
        if addr != consumer {
            new_list.push_back(addr);
        }
    }
    write_subscriber_list(env, &asset, &new_list);

    PriceUpdateUnsubscribedEvent { consumer, asset }.publish(env);
}

/// Returns the list of all consumers currently subscribed to `asset`.
///
/// Off-chain relayers read this list after each price-update event and
/// dispatch individual pull-requests to each subscribed consumer.
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — if `asset` is not registered.
pub fn get_subscribed_consumers(env: &Env, asset: Address) -> Vec<Address> {
    check_registered_asset(env, &asset);
    read_subscriber_list(env, &asset)
}

/// Returns `true` when `consumer` is currently subscribed to updates for `asset`.
pub fn is_subscribed_to_asset(env: &Env, consumer: &Address, asset: &Address) -> bool {
    has_subscription(env, consumer, asset)
}
