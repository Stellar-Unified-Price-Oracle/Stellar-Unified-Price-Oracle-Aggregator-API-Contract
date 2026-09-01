//! # Tiered Rate Limiting Per Consumer (#200)
//!
//! Implements three rate limit tiers: Free (10 req/ledger), Subscribed (100),
//! Enterprise (unlimited). Admin can assign/revoke enterprise status.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{RateLimitExceededEvent, RateLimitTierChangedEvent};
use crate::storage::{get_admin, LEDGER_BUMP};
use crate::types::{DataKey, ErrorCode};

/// Tier enumeration for rate limiting.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RateLimitTier {
    Free = 0,
    Subscribed = 1,
    Enterprise = 2,
}

impl RateLimitTier {
    /// Returns the request limit per ledger for this tier.
    pub fn request_limit(&self) -> u32 {
        match self {
            RateLimitTier::Free => 10,
            RateLimitTier::Subscribed => 100,
            RateLimitTier::Enterprise => u32::MAX, // unlimited
        }
    }
}

/// Checks if a consumer has exceeded their rate limit for the current ledger.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `consumer` - Consumer address to check.
///
/// # Returns
/// `true` if limit exceeded, `false` otherwise.
pub fn check_rate_limit(env: &Env, consumer: Address) -> bool {
    let tier = get_consumer_tier(env, &consumer);
    let limit = tier.request_limit();

    if limit == u32::MAX {
        return false; // Enterprise tier has no limit
    }

    let current_ledger = env.ledger().sequence();
    let count_key = DataKey::QueryCount(consumer.clone(), current_ledger);

    let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);

    if count >= limit {
        RateLimitExceededEvent {
            consumer: consumer.clone(),
            limit,
            current_count: count,
        }
        .publish(env);
        true
    } else {
        // Increment counter
        env.storage().persistent().set(&count_key, &(count + 1));
        env.storage()
            .persistent()
            .extend_ttl(&count_key, 300000, 3600000);
        false
    }
}

/// Returns the rate limit tier for a consumer.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `consumer` - Consumer address.
///
/// # Returns
/// The [`RateLimitTier`] assigned to this consumer (defaults to Free).
pub fn get_consumer_tier(env: &Env, consumer: &Address) -> RateLimitTier {
    let key = DataKey::ConsumerInfo(consumer.clone());
    env.storage()
        .persistent()
        .get::<_, crate::types::ConsumerInfo>(&key)
        .map(|info| match info.tier {
            crate::types::ConsumerTier::Free => RateLimitTier::Free,
            crate::types::ConsumerTier::Basic => RateLimitTier::Subscribed,
            crate::types::ConsumerTier::Premium => RateLimitTier::Enterprise,
        })
        .unwrap_or(RateLimitTier::Free)
}

/// Assigns enterprise (unlimited) tier to a consumer (admin-only).
///
/// # Arguments
/// * `env` - Execution environment.
/// * `consumer` - Consumer to grant enterprise status.
pub fn grant_enterprise_tier(env: &Env, consumer: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::ConsumerInfo(consumer.clone());
    let mut info = env
        .storage()
        .persistent()
        .get::<_, crate::types::ConsumerInfo>(&key)
        .unwrap_or(crate::types::ConsumerInfo {
            tier: crate::types::ConsumerTier::Free,
            subscription_expiry_ledger: 0,
            subscription_expiry_timestamp: 0,
        });

    info.tier = crate::types::ConsumerTier::Premium;
    env.storage().persistent().set(&key, &info);
    env.storage().persistent().extend_ttl(&key, 300000, 3600000);

    RateLimitTierChangedEvent {
        consumer,
        new_tier: 2, // Enterprise
    }
    .publish(env);
}

/// Revokes enterprise tier from a consumer, resetting to Free (admin-only).
///
/// # Arguments
/// * `env` - Execution environment.
/// * `consumer` - Consumer to revoke enterprise status from.
pub fn revoke_enterprise_tier(env: &Env, consumer: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = DataKey::ConsumerInfo(consumer.clone());
    let mut info = env
        .storage()
        .persistent()
        .get::<_, crate::types::ConsumerInfo>(&key)
        .unwrap_or(crate::types::ConsumerInfo {
            tier: crate::types::ConsumerTier::Free,
            subscription_expiry_ledger: 0,
            subscription_expiry_timestamp: 0,
        });

    info.tier = crate::types::ConsumerTier::Free;
    env.storage().persistent().set(&key, &info);
    env.storage().persistent().extend_ttl(&key, 300000, 3600000);

    RateLimitTierChangedEvent {
        consumer,
        new_tier: 0, // Free
    }
    .publish(env);
}
