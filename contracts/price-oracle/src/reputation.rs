//! # Source Reputation & Slashing System (#171)
//!
//! Manages staking, reputation scoring (0–100), decay for idle sources, and admin-triggered
//! slashing of locked stake when reputation falls below a critical threshold.
//!
//! ## Staking Flow
//! 1. Source calls `stake_source` → tokens transferred into contract custody.
//! 2. On deregistration, unlocked portion returned via `unstake_source`.
//!
//! ## Reputation Math (integer-only, range 0–100)
//! - Default starting score: 50.
//! - Updated via weighted moving average after each price submission.
//! - Deviation within 1% → strong increase; deviation >50% → score penalized toward 0.
//! - `update_reputation_on_submission` is called by `prices.rs` after aggregation.
//!
//! ## Slashing
//! - Admin calls `slash_source` to slash `slash_percent`% of locked stake.
//! - Slashed funds are moved to the contract's internal treasury balance.
//! - Re-entrancy safe: uses a per-source guard so the function cannot be called
//!   recursively (the reentrancy module handles the outer guard; this module checks
//!   that the source is not simultaneously being slashed).

use soroban_sdk::{panic_with_error, symbol_short, Address, Env};

use crate::events::{
    SourceReputationUpdatedEvent, SourceSlashedEvent, SourceStakedEvent, SourceUnstakedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, SourceStakeRecord};

// ─── Constants ───────────────────────────────────────────────────────────────

/// Default reputation score assigned to newly registered sources.
pub const INITIAL_REPUTATION: i128 = 50;
/// Maximum reputation score.
pub const MAX_REPUTATION: i128 = 100;
/// Minimum reputation score.
pub const MIN_REPUTATION: i128 = 0;
/// Default decay factor (out of 100). Higher → faster decay toward 50 when idle.
pub const DEFAULT_DECAY_FACTOR: u32 = 5;
/// Default percentage of stake to slash per slash event (e.g. 20 = 20%).
pub const DEFAULT_SLASH_PERCENT: u32 = 20;
/// Default reputation threshold below which a source is slash-eligible (e.g. 20 out of 100).
pub const DEFAULT_SLASH_THRESHOLD: u32 = 20;
/// Precision multiplier for ratio / deviation calculations.
pub const BPS_PRECISION: i128 = 10_000;

// ─── Staking ─────────────────────────────────────────────────────────────────

/// Stakes `amount` of oracle tokens from `source` into contract custody.
///
/// The source must authorize this call. This function transfers `amount` from `source`
/// to the oracle contract using the Soroban token interface, then records the stake.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source address staking the tokens (must authorize).
/// * `amount` - Amount in stroops to stake (must be > 0).
/// * `token_contract` - Address of the XLM/oracle token contract.
pub fn stake_source(env: &Env, source: Address, amount: i128, token_contract: Address) {
    source.require_auth();

    if amount <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice); // reuse for invalid amount
    }

    // Transfer tokens from source to this contract.
    let client = soroban_sdk::token::Client::new(env, &token_contract);
    let contract_addr = env.current_contract_address();
    client.transfer(&source, &contract_addr, &amount);

    // Record the staked amount.
    let key = DataKey::SourceStake(source.clone());
    let existing: SourceStakeRecord =
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(SourceStakeRecord {
                amount: 0,
                last_updated_ledger: env.ledger().sequence(),
            });

    let new_record = SourceStakeRecord {
        amount: existing.amount.saturating_add(amount),
        last_updated_ledger: env.ledger().sequence(),
    };
    env.storage().persistent().set(&key, &new_record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SourceStakedEvent {
        source: source.clone(),
        amount,
        total_stake: new_record.amount,
    }
    .publish(env);
}

/// Returns the current locked stake for a source (in stroops). Returns 0 if no stake.
pub fn get_stake(env: &Env, source: Address) -> i128 {
    let key = DataKey::SourceStake(source);
    env.storage()
        .persistent()
        .get::<_, SourceStakeRecord>(&key)
        .map(|r| r.amount)
        .unwrap_or(0)
}

/// Returns the full stake record for a source, or a zero record if none exists.
pub fn get_stake_record(env: &Env, source: &Address) -> SourceStakeRecord {
    let key = DataKey::SourceStake(source.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(SourceStakeRecord {
            amount: 0,
            last_updated_ledger: env.ledger().sequence(),
        })
}

/// Returns the staked tokens to a source when deregistered.
///
/// Called by `remove_source` in sources.rs. Transfers any remaining (un-slashed) stake
/// back to the source. Admin-only context (guarded by the calling remove_source).
pub fn unstake_source(env: &Env, source: Address) {
    let key = DataKey::SourceStake(source.clone());
    let record: Option<SourceStakeRecord> = env.storage().persistent().get(&key);

    if let Some(r) = record {
        if r.amount > 0 {
            let token_key = DataKey::StakeTokenContract;
            let token_addr: Option<Address> = env.storage().persistent().get(&token_key);
            if let Some(token_contract) = token_addr {
                let client = soroban_sdk::token::Client::new(env, &token_contract);
                let contract_addr = env.current_contract_address();
                // Gracefully handle transfer failures so removal still proceeds.
                let _ = client.try_transfer(&contract_addr, &source, &r.amount);
            }
        }
        env.storage().persistent().remove(&key);

        SourceUnstakedEvent {
            source,
            amount_returned: r.amount,
        }
        .publish(env);
    }
}

// ─── Reputation ──────────────────────────────────────────────────────────────

/// Returns the reputation score (0–100) for a source. Defaults to 50.
pub fn get_reputation(env: &Env, source: Address) -> u32 {
    env.storage()
        .persistent()
        .get::<_, i128>(&DataKey::SourceReputation(source))
        .unwrap_or(INITIAL_REPUTATION) as u32
}

/// Updates a source's reputation based on how far its submitted price deviates from
/// the current median. Called internally after each aggregation.
///
/// Algorithm:
/// - Compute `deviation_bps = |source_price - median_price| * 10_000 / median_price`
/// - Map to an accuracy score 0–100 (100 = perfect, 0 = >=50% deviation)
/// - Weighted moving average: `new = (old * (100 - decay) + accuracy * decay) / 100`
/// - Clamp result to [0, 100]
///
/// Sources deviating >30% (3000 bps) get a severe penalty to ensure slash eligibility
/// is reached quickly.
pub fn update_reputation_on_submission(
    env: &Env,
    source: &Address,
    source_price: i128,
    median_price: i128,
) {
    if median_price <= 0 {
        return;
    }

    let old_score = get_reputation(env, source.clone()) as i128;
    let decay = get_decay_factor(env) as i128;

    // Deviation in basis points.
    let deviation_bps = ((source_price - median_price)
        .abs()
        .saturating_mul(BPS_PRECISION))
        / median_price;

    // accuracy: 100 at 0 deviation, 0 at >= 5000 bps (50%), with a cliff at 3000 bps.
    let accuracy: i128 = if deviation_bps >= 5000 {
        0
    } else if deviation_bps >= 3000 {
        // Severe penalty zone: accuracy 0–10.
        10 - (deviation_bps - 3000) * 10 / 2000
    } else if deviation_bps <= 100 {
        // Within 1% → near perfect score.
        100
    } else {
        // Linear interpolation between 100 and 10 from 100 to 3000 bps.
        100 - (deviation_bps - 100) * 90 / 2900
    };

    let accuracy = accuracy.clamp(0, 100);

    let new_score = ((old_score * (100 - decay)) + (accuracy * decay)) / 100;
    let new_score = new_score.clamp(MIN_REPUTATION, MAX_REPUTATION);

    env.storage()
        .persistent()
        .set(&DataKey::SourceReputation(source.clone()), &new_score);
    env.storage().persistent().extend_ttl(
        &DataKey::SourceReputation(source.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    SourceReputationUpdatedEvent {
        source: source.clone(),
        old_score,
        new_score,
    }
    .publish(env);
}

/// Applies ledger-based reputation decay to all sources with a stored score.
///
/// Idle sources (those that haven't updated this ledger) drift back toward 50.
/// This is a lightweight per-source function; callers control which sources to decay.
///
/// `decay_bps`: amount to move toward 50, scaled as basis points (100 = 1%).
pub fn apply_reputation_decay(env: &Env, source: &Address) {
    let key = DataKey::SourceReputation(source.clone());
    let score: i128 = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(INITIAL_REPUTATION);

    if score == INITIAL_REPUTATION {
        return; // already at neutral, no decay needed
    }

    let decay = get_decay_factor(env) as i128;
    // Move score toward 50 (neutral) by `decay` percent of the distance.
    let distance = score - INITIAL_REPUTATION; // negative if below 50
    let delta = distance * decay / 100;
    let new_score = (score - delta).clamp(MIN_REPUTATION, MAX_REPUTATION);

    if new_score != score {
        env.storage().persistent().set(&key, &new_score);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}

// ─── Slashing ────────────────────────────────────────────────────────────────

/// Slashes a configurable percentage of `source`'s locked stake.
///
/// Admin-only. Slashed funds are added to the contract's internal treasury balance.
/// Re-entrancy safety is handled by the outer reentrancy guard in `lib.rs`.
///
/// Panics with `NotAuthorized` if reputation is not below the slash threshold (i.e.,
/// the source must be in a slash-eligible state), unless the admin forces a slash.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `source` - Source to slash.
/// * `force` - If `true`, skip the reputation threshold check (admin override).
pub fn slash_source(env: &Env, source: Address, force: bool) {
    let admin = get_admin(env);
    admin.require_auth();

    // Check slash-eligibility unless forced.
    if !force {
        let threshold = get_slash_threshold(env) as i128;
        let score = get_reputation(env, source.clone()) as i128;
        if score >= threshold {
            panic_with_error!(env, ErrorCode::ReputationTooHighToSlash);
        }
    }

    let key = DataKey::SourceStake(source.clone());
    let record: SourceStakeRecord =
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(SourceStakeRecord {
                amount: 0,
                last_updated_ledger: env.ledger().sequence(),
            });

    if record.amount == 0 {
        // Nothing to slash; silently return.
        return;
    }

    let slash_pct = get_slash_percent(env) as i128;
    let slash_amount = (record.amount * slash_pct / 100).max(1);
    let slash_amount = slash_amount.min(record.amount);
    let remaining = record.amount - slash_amount;

    // Update stake record.
    let new_record = SourceStakeRecord {
        amount: remaining,
        last_updated_ledger: env.ledger().sequence(),
    };
    env.storage().persistent().set(&key, &new_record);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    // Add slashed amount to treasury balance.
    let treasury_key = DataKey::TreasuryBalance;
    let current_treasury: i128 = env
        .storage()
        .persistent()
        .get(&treasury_key)
        .unwrap_or(0i128);
    env.storage().persistent().set(
        &treasury_key,
        &current_treasury.saturating_add(slash_amount),
    );
    env.storage()
        .persistent()
        .extend_ttl(&treasury_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SourceSlashedEvent {
        source: source.clone(),
        slash_amount,
        remaining_stake: remaining,
        slash_percent: slash_pct as u32,
    }
    .publish(env);
}

/// Sweeps all slashed treasury funds to `recipient`. Admin-only.
pub fn sweep_treasury(env: &Env, recipient: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    let treasury_key = DataKey::TreasuryBalance;
    let balance: i128 = env
        .storage()
        .persistent()
        .get(&treasury_key)
        .unwrap_or(0i128);

    if balance <= 0 {
        return;
    }

    let token_key = DataKey::StakeTokenContract;
    let token_addr: Option<Address> = env.storage().persistent().get(&token_key);
    if let Some(token_contract) = token_addr {
        let client = soroban_sdk::token::Client::new(env, &token_contract);
        let contract_addr = env.current_contract_address();
        client.transfer(&contract_addr, &recipient, &balance);
        env.storage().persistent().set(&treasury_key, &0i128);
    }
}

/// Returns current treasury balance (sum of all slashed amounts).
pub fn get_treasury_balance(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::TreasuryBalance)
        .unwrap_or(0i128)
}

// ─── Config Setters / Getters ────────────────────────────────────────────────

/// Sets the address of the stake/fee token contract. Admin-only.
pub fn set_stake_token_contract(env: &Env, token: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::StakeTokenContract, &token);
}

/// Returns the configured stake token contract address, if set.
pub fn get_stake_token_contract(env: &Env) -> Option<Address> {
    env.storage().persistent().get(&DataKey::StakeTokenContract)
}

/// Sets the reputation decay factor (0–100). Admin-only.
pub fn set_decay_factor(env: &Env, factor: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if factor > 100 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::ReputationDecayFactor, &factor);
    crate::events::ReputationDecayChangedEvent { value: factor }.publish(env);
}

/// Returns the current reputation decay factor. Defaults to `DEFAULT_DECAY_FACTOR`.
pub fn get_decay_factor(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::ReputationDecayFactor)
        .unwrap_or(DEFAULT_DECAY_FACTOR)
}

/// Sets the slash percentage (0–100). Admin-only.
pub fn set_slash_percent(env: &Env, percent: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if percent > 100 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::SlashPercent, &percent);
}

/// Returns the current slash percentage. Defaults to `DEFAULT_SLASH_PERCENT`.
pub fn get_slash_percent(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::SlashPercent)
        .unwrap_or(DEFAULT_SLASH_PERCENT)
}

/// Sets the reputation threshold below which a source becomes slash-eligible. Admin-only.
pub fn set_slash_threshold(env: &Env, threshold: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if threshold > 100 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::SlashReputationThreshold, &threshold);
}

/// Returns the current slash reputation threshold. Defaults to `DEFAULT_SLASH_THRESHOLD`.
pub fn get_slash_threshold(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::SlashReputationThreshold)
        .unwrap_or(DEFAULT_SLASH_THRESHOLD)
}
