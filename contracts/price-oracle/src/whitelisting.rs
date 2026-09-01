//! # Tiered Consumer Access & XLM Treasury Management (#173)
//!
//! Implements a three-tier consumer access model (Free / Basic / Premium) with
//! per-ledger query quotas, XLM subscription fee collection via the Soroban token
//! interface, and an admin sweep endpoint.
//!
//! ## Tiers
//! | Tier    | Queries/ledger | Freshness     | Monthly fee (XLM) |
//! |---------|----------------|---------------|-------------------|
//! | Free    | 10             | ≤ 1 hour stale | 0                |
//! | Basic   | 100            | ≤ 30 seconds  | 10 XLM           |
//! | Premium | unlimited      | real-time     | 100 XLM          |
//!
//! ## Storage Layout
//! - `ConsumerInfo(addr)` → `ConsumerInfo` struct (tier, quotas, expiry).
//! - `TierPricing(tier_discriminant)` → price in stroops.
//! - `TierQueryCount(addr, ledger)` → temporary u32 counter.
//! - `WhitelistTreasury` → treasury `Address` for fee sweeps.
//! - `XlmTokenContract` → address of the XLM token contract.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::events::{ConsumerRegisteredEvent, ConsumerTierChangedEvent, TierFeePaidEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{ConsumerInfo, ConsumerTier, DataKey, ErrorCode};

// ─── Tier constants ───────────────────────────────────────────────────────────

/// Free tier: max 10 queries per ledger.
pub const FREE_QUOTA: u32 = 10;
/// Basic tier: max 100 queries per ledger.
pub const BASIC_QUOTA: u32 = 100;
/// Premium tier: no quota limit (represented as u32::MAX).
pub const PREMIUM_QUOTA: u32 = u32::MAX;

/// Free tier staleness limit in seconds (1 hour).
pub const FREE_STALENESS_SECS: u64 = 3600;
/// Basic tier staleness limit in seconds (30 seconds).
pub const BASIC_STALENESS_SECS: u64 = 30;
/// Premium tier: no staleness limit (0 = real-time, no additional filter).
pub const PREMIUM_STALENESS_SECS: u64 = 0;

/// Default Basic tier price: 10 XLM = 100_000_000 stroops.
pub const DEFAULT_BASIC_PRICE_STROOPS: i128 = 100_000_000;
/// Default Premium tier price: 100 XLM = 1_000_000_000 stroops.
pub const DEFAULT_PREMIUM_PRICE_STROOPS: i128 = 1_000_000_000;

// ─── Consumer Registration ────────────────────────────────────────────────────

/// Registers or upgrades a consumer to the given tier.
///
/// For `Basic` and `Premium` tiers the consumer must pay the configured subscription
/// fee in XLM (or the configured XLM-equivalent token). The subscription is valid for
/// ~30 days (expressed as ledgers: 30 * 24 * 3600 / 5 ≈ 518_400 ledgers at 5 s/ledger,
/// but we store an expiry *timestamp* for precision).
///
/// `Free` tier registration is free; no token transfer occurs.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `consumer` - Consumer address (must authorize for paid tiers).
/// * `tier` - Desired tier.
pub fn register_consumer(env: &Env, consumer: Address, tier: ConsumerTier) {
    consumer.require_auth();

    let current_ledger = env.ledger().sequence();
    let current_ts = env.ledger().timestamp();

    // Duration: ~30 days in seconds.
    const THIRTY_DAYS_SECS: u64 = 30 * 24 * 3600;
    // Duration in ledgers (≈ 5 s/ledger).
    const THIRTY_DAYS_LEDGERS: u32 = 518_400;

    let (expiry_ts, expiry_ledger) = match tier {
        ConsumerTier::Free => (0u64, 0u32), // Free = permanent, no expiry tracking
        ConsumerTier::Basic | ConsumerTier::Premium => (
            current_ts.saturating_add(THIRTY_DAYS_SECS),
            current_ledger.saturating_add(THIRTY_DAYS_LEDGERS),
        ),
    };

    // Collect fee for paid tiers.
    let fee = match tier {
        ConsumerTier::Free => 0i128,
        ConsumerTier::Basic | ConsumerTier::Premium => {
            let tier_key = tier_discriminant(&tier);
            get_tier_price_internal(env, tier_key)
        }
    };

    if fee > 0 {
        let token_addr = get_xlm_token_contract(env)
            .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NotAuthorized));
        let client = soroban_sdk::token::Client::new(env, &token_addr);
        let contract_addr = env.current_contract_address();
        client.transfer(&consumer, &contract_addr, &fee);

        TierFeePaidEvent {
            consumer: consumer.clone(),
            tier: tier_discriminant(&tier),
            amount: fee,
        }
        .publish(env);

        distribute_subscription_fee(env, fee);
    }

    // Determine whether existing record exists (for tier-change event).
    let info_key = DataKey::ConsumerInfo(consumer.clone());
    let old_tier_disc: Option<u32> = env
        .storage()
        .persistent()
        .get::<_, ConsumerInfo>(&info_key)
        .map(|c| tier_discriminant(&c.tier));

    let info = ConsumerInfo {
        tier: tier.clone(),
        subscription_expiry_ledger: expiry_ledger,
        subscription_expiry_ts: expiry_ts,
        queries_this_ledger: 0,
        quota_reset_ledger: current_ledger,
    };
    env.storage().persistent().set(&info_key, &info);
    env.storage()
        .persistent()
        .extend_ttl(&info_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    if let Some(old_disc) = old_tier_disc {
        ConsumerTierChangedEvent {
            consumer: consumer.clone(),
            old_tier: old_disc,
            new_tier: tier_discriminant(&tier),
        }
        .publish(env);
    } else {
        ConsumerRegisteredEvent {
            consumer: consumer.clone(),
            tier: tier_discriminant(&tier),
            subscription_expiry_ts: expiry_ts,
        }
        .publish(env);
    }
}

/// Returns the `ConsumerInfo` for a given consumer, or `None` if not registered.
pub fn query_consumer(env: &Env, consumer: Address) -> Option<ConsumerInfo> {
    let key = DataKey::ConsumerInfo(consumer);
    env.storage().persistent().get(&key)
}

// ─── Quota Enforcement ────────────────────────────────────────────────────────

/// Checks and enforces the per-ledger query quota for a consumer.
///
/// Returns the effective staleness limit in seconds for the caller's tier:
/// - Free → `FREE_STALENESS_SECS`
/// - Basic → `BASIC_STALENESS_SECS`
/// - Premium → `PREMIUM_STALENESS_SECS` (0 = no limit)
///
/// Panics with `RateLimitExceeded` if the consumer has exhausted their ledger quota.
/// Panics with `SubscriptionExpired` if a paid subscription has lapsed.
///
/// Unregistered callers are treated as Free tier.
pub fn check_and_record_query(env: &Env, consumer: &Address) -> u64 {
    let current_ledger = env.ledger().sequence();
    let current_ts = env.ledger().timestamp();

    let info_key = DataKey::ConsumerInfo(consumer.clone());
    let info_opt: Option<ConsumerInfo> = env.storage().persistent().get(&info_key);

    let (tier, sub_expiry_ledger, sub_expiry_ts, mut queries, quota_reset_ledger) = match info_opt {
        Some(ref i) => (
            i.tier.clone(),
            i.subscription_expiry_ledger,
            i.subscription_expiry_ts,
            i.queries_this_ledger,
            i.quota_reset_ledger,
        ),
        None => (ConsumerTier::Free, 0u32, 0u64, 0u32, current_ledger),
    };

    // Validate subscription expiry for paid tiers.
    match tier {
        ConsumerTier::Basic | ConsumerTier::Premium => {
            if sub_expiry_ts > 0 && current_ts > sub_expiry_ts {
                panic_with_error!(env, ErrorCode::SubscriptionExpired);
            }
        }
        ConsumerTier::Free => {}
    }

    // Reset counter if we're in a new ledger.
    if quota_reset_ledger != current_ledger {
        queries = 0;
    }

    // Check quota.
    let quota = tier_quota(&tier);
    if quota != PREMIUM_QUOTA && queries >= quota {
        panic_with_error!(env, ErrorCode::RateLimitExceeded);
    }

    // Increment and write back.
    let new_queries = if quota == PREMIUM_QUOTA {
        queries // don't overflow premium counter
    } else {
        queries + 1
    };

    let updated = ConsumerInfo {
        tier: tier.clone(),
        subscription_expiry_ledger: sub_expiry_ledger,
        subscription_expiry_ts: sub_expiry_ts,
        queries_this_ledger: new_queries,
        quota_reset_ledger: current_ledger,
    };
    env.storage().persistent().set(&info_key, &updated);
    env.storage()
        .persistent()
        .extend_ttl(&info_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    tier_staleness(&tier)
}

// ─── Tier Pricing ─────────────────────────────────────────────────────────────

/// Sets the subscription fee for a tier. Admin-only.
///
/// `tier_disc` is the `u32` discriminant of `ConsumerTier` (0=Free, 1=Basic, 2=Premium).
pub fn set_tier_pricing(env: &Env, tier: ConsumerTier, price: i128) {
    let admin = get_admin(env);
    admin.require_auth();

    if price < 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let disc = tier_discriminant(&tier);
    env.storage()
        .persistent()
        .set(&DataKey::TierPricing(disc), &price);
    env.storage().persistent().extend_ttl(
        &DataKey::TierPricing(disc),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

/// Returns the subscription fee in stroops for a given tier.
pub fn get_tier_price(env: &Env, tier: ConsumerTier) -> i128 {
    get_tier_price_internal(env, tier_discriminant(&tier))
}

fn get_tier_price_internal(env: &Env, disc: u32) -> i128 {
    let stored: Option<i128> = env.storage().persistent().get(&DataKey::TierPricing(disc));

    stored.unwrap_or(match disc {
        0 => 0,
        1 => DEFAULT_BASIC_PRICE_STROOPS,
        2 => DEFAULT_PREMIUM_PRICE_STROOPS,
        _ => 0,
    })
}

// ─── XLM Token Contract Config ────────────────────────────────────────────────

/// Sets the XLM token contract address used for fee collection. Admin-only.
pub fn set_xlm_token_contract(env: &Env, token: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::XlmTokenContract, &token);
    env.storage().persistent().extend_ttl(
        &DataKey::XlmTokenContract,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

/// Returns the configured XLM token contract address.
pub fn get_xlm_token_contract(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&DataKey::XlmTokenContract)
}

// ─── Treasury Sweep ───────────────────────────────────────────────────────────

/// Sets the treasury address for XLM fee sweeps. Admin-only.
pub fn set_whitelist_treasury(env: &Env, treasury: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::WhitelistTreasury, &treasury);
    env.storage().persistent().extend_ttl(
        &DataKey::WhitelistTreasury,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );
}

/// Returns the configured treasury address, or `None` if not set.
pub fn get_whitelist_treasury(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&DataKey::WhitelistTreasury)
}

/// Sweeps all collected subscription fees held by the contract to the treasury address.
///
/// Admin-only. Transfers the entire token balance held by this contract (as fees) to
/// the configured treasury address.
///
/// `amount` - Exact amount to sweep in stroops. Use `0` to sweep the full balance.
pub fn sweep_fees(env: &Env, amount: i128) {
    let admin = get_admin(env);
    admin.require_auth();

    let treasury = get_whitelist_treasury(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NotAuthorized));
    let token_addr = get_xlm_token_contract(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NotAuthorized));

    let client = soroban_sdk::token::Client::new(env, &token_addr);
    let contract_addr = env.current_contract_address();

    let sweep_amount = if amount > 0 {
        amount
    } else {
        // Query the contract's current balance.
        client.balance(&contract_addr)
    };

    if sweep_amount > 0 {
        client.transfer(&contract_addr, &treasury, &sweep_amount);
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Returns the numeric discriminant for a `ConsumerTier`.
pub fn tier_discriminant(tier: &ConsumerTier) -> u32 {
    match tier {
        ConsumerTier::Free => 0,
        ConsumerTier::Basic => 1,
        ConsumerTier::Premium => 2,
    }
}

/// Returns the queries-per-ledger quota for a tier.
pub fn tier_quota(tier: &ConsumerTier) -> u32 {
    match tier {
        ConsumerTier::Free => FREE_QUOTA,
        ConsumerTier::Basic => BASIC_QUOTA,
        ConsumerTier::Premium => PREMIUM_QUOTA,
    }
}

/// Returns the staleness limit (seconds) for a tier. `0` = no limit.
pub fn tier_staleness(tier: &ConsumerTier) -> u64 {
    match tier {
        ConsumerTier::Free => FREE_STALENESS_SECS,
        ConsumerTier::Basic => BASIC_STALENESS_SECS,
        ConsumerTier::Premium => PREMIUM_STALENESS_SECS,
    }
}

pub fn distribute_subscription_fee(env: &Env, fee: i128) {
    if fee <= 0 {
        return;
    }

    let oracle_sources: crate::types::OracleSources = crate::storage::read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();
    if total_sources == 0 {
        return;
    }

    // Get total submission count across all sources
    let total_sub_key = DataKey::TotalSubmissionCount;
    let total_submissions: u32 = env.storage().persistent().get(&total_sub_key).unwrap_or(0);

    if total_submissions > 0 {
        let mut distributed_fee = 0i128;
        for i in 0..total_sources {
            let source = oracle_sources.sources.get_unchecked(i);
            let src_sub_key = DataKey::SourceSubmissionCount(source.clone());
            let source_submissions: u32 = env.storage().persistent().get(&src_sub_key).unwrap_or(0);

            if source_submissions > 0 {
                let share = (fee * source_submissions as i128) / total_submissions as i128;
                if share > 0 {
                    let balance_key = DataKey::SourceFeeBalance(source.clone());
                    let current_balance: i128 =
                        env.storage().persistent().get(&balance_key).unwrap_or(0);
                    env.storage()
                        .persistent()
                        .set(&balance_key, &(current_balance + share));
                    env.storage().persistent().extend_ttl(
                        &balance_key,
                        LEDGER_THRESHOLD,
                        LEDGER_BUMP,
                    );

                    distributed_fee += share;

                    crate::events::SourceFeeCreditedEvent {
                        source: source.clone(),
                        amount: share,
                    }
                    .publish(env);
                }
            }
        }

        let remainder = fee - distributed_fee;
        if remainder > 0 {
            let source = oracle_sources.sources.get_unchecked(0);
            let balance_key = DataKey::SourceFeeBalance(source.clone());
            let current_balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);
            env.storage()
                .persistent()
                .set(&balance_key, &(current_balance + remainder));
            env.storage()
                .persistent()
                .extend_ttl(&balance_key, LEDGER_THRESHOLD, LEDGER_BUMP);

            crate::events::SourceFeeCreditedEvent {
                source: source.clone(),
                amount: remainder,
            }
            .publish(env);
        }
    } else {
        let share = fee / total_sources as i128;
        if share > 0 {
            let mut distributed_fee = 0i128;
            for i in 0..total_sources {
                let source = oracle_sources.sources.get_unchecked(i);
                let balance_key = DataKey::SourceFeeBalance(source.clone());
                let current_balance: i128 =
                    env.storage().persistent().get(&balance_key).unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&balance_key, &(current_balance + share));
                env.storage()
                    .persistent()
                    .extend_ttl(&balance_key, LEDGER_THRESHOLD, LEDGER_BUMP);

                distributed_fee += share;

                crate::events::SourceFeeCreditedEvent {
                    source: source.clone(),
                    amount: share,
                }
                .publish(env);
            }
            let remainder = fee - distributed_fee;
            if remainder > 0 {
                let source = oracle_sources.sources.get_unchecked(0);
                let balance_key = DataKey::SourceFeeBalance(source.clone());
                let current_balance: i128 =
                    env.storage().persistent().get(&balance_key).unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&balance_key, &(current_balance + remainder));
                env.storage()
                    .persistent()
                    .extend_ttl(&balance_key, LEDGER_THRESHOLD, LEDGER_BUMP);

                crate::events::SourceFeeCreditedEvent {
                    source: source.clone(),
                    amount: remainder,
                }
                .publish(env);
            }
        }
    }
}

pub fn withdraw_fees(env: &Env, source: Address) {
    source.require_auth();
    crate::storage::check_source(env, &source);

    let balance_key = DataKey::SourceFeeBalance(source.clone());
    let balance: i128 = env.storage().persistent().get(&balance_key).unwrap_or(0);

    if balance <= 0 {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let token_addr = get_xlm_token_contract(env)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NotAuthorized));
    let client = soroban_sdk::token::Client::new(env, &token_addr);
    let contract_addr = env.current_contract_address();

    client.transfer(&contract_addr, &source, &balance);

    env.storage().persistent().set(&balance_key, &0i128);

    crate::events::SourceFeesWithdrawnEvent {
        source,
        amount: balance,
    }
    .publish(env);
}

pub fn get_source_fee_balance(env: &Env, source: Address) -> i128 {
    crate::storage::check_source(env, &source);
    let key = DataKey::SourceFeeBalance(source);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(0i128)
}
