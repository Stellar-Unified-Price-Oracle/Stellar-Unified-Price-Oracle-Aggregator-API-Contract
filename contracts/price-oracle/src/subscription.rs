use soroban_sdk::{panic_with_error, Address, Env, Symbol, Vec};

use crate::events::{
    SubscriptionCreatedEvent, SubscriptionRenewedEvent,
};
use crate::storage::{
    get_plan_amount, is_subscribed, read_subscription_expiry, read_subscription_plans,
    write_subscription_expiry, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{DataKey, ErrorCode, SubscriptionPlans};

pub const DEFAULT_SUBSCRIPTION_DURATION: u64 = 86400; // 1 day in seconds

pub fn subscribe(env: &Env, consumer: Address, duration: u32) {
    consumer.require_auth();

    let plan_amount = get_plan_amount(env, duration).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::InvalidDuration)
    });

    let ledger_timestamp = env.ledger().timestamp();
    let new_expiry = ledger_timestamp.saturating_add(duration as u64);

    write_subscription_expiry(env, &consumer, new_expiry);

    env.events().publish(
        (Symbol::new(env, "sub_created"), consumer.clone()),
        SubscriptionCreatedEvent {
            consumer: consumer.clone(),
            duration: duration as u64,
        },
    );
}

pub fn renew_subscription(env: &Env, consumer: Address) {
    consumer.require_auth();

    let current_expiry = read_subscription_expiry(env, &consumer).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::NoData)
    });

    let ledger_timestamp = env.ledger().timestamp();

    if current_expiry < ledger_timestamp {
        panic_with_error!(env, ErrorCode::SubscriptionExpired);
    }

    let remaining_duration = current_expiry.saturating_sub(ledger_timestamp);
    let new_expiry = current_expiry.saturating_add(remaining_duration);

    write_subscription_expiry(env, &consumer, new_expiry);

    env.events().publish(
        (Symbol::new(env, "sub_renewed"), consumer.clone()),
        SubscriptionRenewedEvent {
            consumer: consumer.clone(),
        },
    );
}

pub fn get_subscription_expiry(env: &Env, consumer: Address) -> u64 {
    let key = DataKey::SubscriptionExpiry(consumer.clone());
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    read_subscription_expiry(env, &consumer).unwrap_or(0)
}

pub fn get_subscription_plans(env: &Env) -> SubscriptionPlans {
    let key = DataKey::SubscriptionPlans;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    read_subscription_plans(env)
}
