use soroban_sdk::{panic_with_error, token, Address, Env};

use crate::events::{
    SubscriptionCancelledEvent, SubscriptionCreatedEvent, SubscriptionFeesDistributedEvent,
    SubscriptionPaymentRefundedEvent, SubscriptionRenewedEvent, SubscriptionTokenSetEvent,
};
use crate::storage::{
    get_admin, get_plan_amount, read_subscription_expiry, read_subscription_plans,
    write_subscription_expiry, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{
    DataKey, ErrorCode, SubscriptionPayment, SubscriptionPlans, TokenSubscriptionRecord,
};
use crate::whitelisting::get_xlm_token_contract;

// ---------------------------------------------------------------------------
// SAC token helpers
// ---------------------------------------------------------------------------

/// Read the configured SAC token contract address, if any.
pub fn read_subscription_token(env: &Env) -> Option<Address> {
    env.storage().instance().get(&DataKey::SubscriptionToken)
}

/// Write the SAC token contract address to instance storage.
fn write_subscription_token(env: &Env, token: &Address) {
    env.storage()
        .instance()
        .set(&DataKey::SubscriptionToken, token);
}

/// Read the token-backed subscription record for `consumer`, if any.
fn read_token_record(env: &Env, consumer: &Address) -> Option<TokenSubscriptionRecord> {
    let key = DataKey::SubscriptionTokenDeposit(consumer.clone());
    let rec: Option<TokenSubscriptionRecord> = env.storage().persistent().get(&key);
    if rec.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    rec
}

/// Write (or overwrite) the token-backed subscription record for `consumer`.
fn write_token_record(env: &Env, consumer: &Address, record: &TokenSubscriptionRecord) {
    let key = DataKey::SubscriptionTokenDeposit(consumer.clone());
    env.storage().persistent().set(&key, record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Remove the token-backed subscription record for `consumer`.
fn remove_token_record(env: &Env, consumer: &Address) {
    let key = DataKey::SubscriptionTokenDeposit(consumer.clone());
    if env.storage().persistent().has(&key) {
        env.storage().persistent().remove(&key);
    }
}

// ---------------------------------------------------------------------------
// Admin: configure the subscription token
// ---------------------------------------------------------------------------

/// Sets the SAC token contract used for subscription payments.  Admin-only.
///
/// Pass `None` (by calling `remove_subscription_token`) or a valid token address.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the current admin.
pub fn set_subscription_token(env: &Env, token_contract: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    write_subscription_token(env, &token_contract);

    SubscriptionTokenSetEvent {
        admin,
        token: token_contract,
    }
    .publish(env);
}

/// Returns the currently configured SAC token contract, or `None`.
pub fn get_subscription_token(env: &Env) -> Option<Address> {
    read_subscription_token(env)
}

// ---------------------------------------------------------------------------
// Consumer: subscribe / renew / cancel
// ---------------------------------------------------------------------------

/// Creates a new subscription for `consumer` for the given `duration` plan.
///
/// If an XLM token contract is configured, transfers `plan_amount` tokens from `consumer`
/// to this contract as payment.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`]   — `consumer` did not authorize the call.
/// * [`ErrorCode::InvalidDuration`] — `duration` does not match any registered plan.
pub fn subscribe(env: &Env, consumer: Address, duration: u32) {
    consumer.require_auth();

    let plan_amount = get_plan_amount(env, duration)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::InvalidDuration));

    let ledger_timestamp = env.ledger().timestamp();
    let new_expiry = ledger_timestamp.saturating_add(duration as u64);

    // If XLM token is configured, collect payment.
    if let Some(token_contract) = get_xlm_token_contract(env) {
        if plan_amount > 0 {
            let client = token::Client::new(env, &token_contract);
            let contract_addr = env.current_contract_address();
            client.transfer(&consumer, &contract_addr, &plan_amount);

            let payment = SubscriptionPayment {
                consumer: consumer.clone(),
                amount: plan_amount,
                timestamp: ledger_timestamp,
                status: 1,
            };
            env.storage()
                .persistent()
                .set(&DataKey::SubscriptionPayment(consumer.clone()), &payment);
            env.storage().persistent().extend_ttl(
                &DataKey::SubscriptionPayment(consumer.clone()),
                LEDGER_THRESHOLD,
                LEDGER_BUMP,
            );

            SubscriptionPaymentReceivedEvent {
                consumer: consumer.clone(),
                amount: plan_amount,
                ledger: env.ledger().sequence(),
            }
            .publish(env);
        }
    }

    write_subscription_expiry(env, &consumer, new_expiry);

    SubscriptionCreatedEvent {
        consumer: consumer.clone(),
        duration: duration as u64,
    }
    .publish(env);
}

/// Renews an existing active subscription.
///
/// The subscription must not have expired.  Expiry is extended by the remaining time.
/// No additional token transfer is required for a renewal (the existing deposit covers it).
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`]     — `consumer` did not authorize the call.
/// * [`ErrorCode::NoData`]            — no subscription exists.
/// * [`ErrorCode::SubscriptionExpired`] — the current subscription has expired.
pub fn renew_subscription(env: &Env, consumer: Address) {
    consumer.require_auth();

    let current_expiry = read_subscription_expiry(env, &consumer)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));

    let ledger_timestamp = env.ledger().timestamp();

    if current_expiry < ledger_timestamp {
        panic_with_error!(env, ErrorCode::SubscriptionExpired);
    }

    let remaining_duration = current_expiry.saturating_sub(ledger_timestamp);
    let new_expiry = current_expiry.saturating_add(remaining_duration);

    write_subscription_expiry(env, &consumer, new_expiry);

    SubscriptionRenewedEvent {
        consumer: consumer.clone(),
    }
    .publish(env);
}

/// Cancels `consumer`'s subscription and, if a SAC token is configured, returns a
/// pro-rated refund for the unused portion.
///
/// Pro-rata calculation:
/// ```text
/// elapsed    = now - start_timestamp
/// total_dur  = expiry - start_timestamp
/// refund     = deposited_amount * (total_dur - elapsed) / total_dur
/// ```
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`]  — `consumer` did not authorize the call.
/// * [`ErrorCode::NoActiveSubscription`] — no active subscription found.
pub fn cancel_subscription(env: &Env, consumer: Address) {
    consumer.require_auth();

    let expiry = read_subscription_expiry(env, &consumer)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoActiveSubscription));

    let ledger_timestamp = env.ledger().timestamp();
    let mut refund_amount: i128 = 0;

    // Process token refund if a token is configured and a deposit record exists.
    if let Some(token_contract) = read_subscription_token(env) {
        if let Some(record) = read_token_record(env, &consumer) {
            let total_dur = record
                .expiry_timestamp
                .saturating_sub(record.start_timestamp);
            if total_dur > 0 && ledger_timestamp < expiry {
                let elapsed = ledger_timestamp.saturating_sub(record.start_timestamp);
                let remaining = total_dur.saturating_sub(elapsed);
                // Integer division: refund = deposited * remaining / total_dur
                refund_amount = (record.deposited_amount * remaining as i128) / total_dur as i128;
            }

            if refund_amount > 0 {
                let client = token::Client::new(env, &token_contract);
                let contract_addr = env.current_contract_address();
                client.transfer(&contract_addr, &consumer, &refund_amount);
            }

            remove_token_record(env, &consumer);
        }
    }

    // Remove the subscription expiry record.
    let expiry_key = DataKey::SubscriptionExpiry(consumer.clone());
    if env.storage().persistent().has(&expiry_key) {
        env.storage().persistent().remove(&expiry_key);
    }

    SubscriptionCancelledEvent {
        consumer: consumer.clone(),
        refund_amount,
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Read-only helpers (unchanged from original)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// #294: Native token fee distribution and refunds
// ---------------------------------------------------------------------------

/// Distributes collected subscription fees to treasury and sources/relayers.
///
/// Admin-only.
pub fn distribute_subscription_fees(env: &Env, consumer: &Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let payment_key = DataKey::SubscriptionPayment(consumer.clone());
    let payment: SubscriptionPayment = env
        .storage()
        .persistent()
        .get(&payment_key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));

    if payment.status != 1 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let total = payment.amount;
    let treasury_share = total / 4;
    let sources_share = total - treasury_share;

    if treasury_share > 0 {
        if let Some(token_contract) = get_xlm_token_contract(env) {
            let client = token::Client::new(env, &token_contract);
            let contract_addr = env.current_contract_address();
            let treasury = get_admin(env);
            client.transfer(&contract_addr, &treasury, &treasury_share);
        }
    }

    let mut updated = payment;
    updated.status = 2;
    env.storage().persistent().set(&payment_key, &updated);

    SubscriptionFeesDistributedEvent {
        consumer: consumer.clone(),
        amount: total,
        treasury_share,
        sources_share,
    }
    .publish(env);
}

/// Refunds a consumer's subscription payment.
///
/// Admin-only.
pub fn refund_subscription_payment(env: &Env, consumer: &Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let payment_key = DataKey::SubscriptionPayment(consumer.clone());
    let payment: SubscriptionPayment = env
        .storage()
        .persistent()
        .get(&payment_key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));

    if payment.status != 1 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    if let Some(token_contract) = get_xlm_token_contract(env) {
        let client = token::Client::new(env, &token_contract);
        let contract_addr = env.current_contract_address();
        client.transfer(&contract_addr, consumer, &payment.amount);
    }

    let mut updated = payment;
    updated.status = 2;
    env.storage().persistent().set(&payment_key, &updated);

    SubscriptionPaymentRefundedEvent {
        consumer: consumer.clone(),
        amount: payment.amount,
        reason: String::from_slice(env, "Subscription failed"),
    }
    .publish(env);
}
