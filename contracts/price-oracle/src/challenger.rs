//! Price feed verification challenger module (#235)
//!
//! Implements an "oracle game" mechanism where external observers are economically incentivized
//! to challenge and correct price discrepancies. Correct challengers are rewarded.

use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, Vec};

use crate::events::{
    emit_admin_action, ChallengePricedEvent, ChallengeResolvedEvent, RewardsClaimedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{Challenge, DataKey, ErrorCode};

/// Maximum unresolved challenges a single challenger may have open at once (#461).
pub const MAX_OPEN_CHALLENGES_PER_CHALLENGER: u32 = 3;
/// Maximum unresolved challenges that may be open against one asset (#461).
pub const MAX_OPEN_CHALLENGES_PER_ASSET: u32 = 5;
/// Challenges resolved as invalid after which a challenger is barred (#461).
pub const MAX_CHALLENGER_STRIKES: u32 = 3;

fn read_u32(env: &Env, key: &DataKey) -> u32 {
    env.storage().persistent().get(key).unwrap_or(0)
}

fn write_u32(env: &Env, key: &DataKey, value: u32) {
    if value == 0 {
        env.storage().persistent().remove(key);
    } else {
        env.storage().persistent().set(key, &value);
    }
}

/// Challenge a price submission for an asset.
///
/// Any address can challenge an aggregate price by providing their expected price
/// and proof data. If the challenge is resolved as valid (via resolve_challenge),
/// the challenger receives a reward.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `asset` - The asset whose price is being challenged.
/// * `expected_price` - The challenger's claim of the correct price.
/// * `proof_data` - Arbitrary proof bytes supporting the challenge.
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — if the asset is not registered.
/// * [`ErrorCode::InvalidPrice`] — if `expected_price` is <= 0.
/// * [`ErrorCode::NotAuthorized`] — if the challenger reached [`MAX_CHALLENGER_STRIKES`].
/// * [`ErrorCode::RateLimitExceeded`] — if the challenger or asset open-challenge cap is hit.
pub fn challenge_price(
    env: &Env,
    challenger: Address,
    asset: Address,
    expected_price: i128,
    proof_data: Bytes,
) {
    challenger.require_auth();

    // Validate asset is registered
    crate::storage::check_registered_asset(env, &asset);

    if expected_price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    // Anti-griefing (#461): barred challengers and bounded open challenges.
    if read_u32(env, &DataKey::ChallengerStrikes(challenger.clone())) >= MAX_CHALLENGER_STRIKES {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }
    let challenger_key = DataKey::ChallengerOpenCount(challenger.clone());
    let challenger_open = read_u32(env, &challenger_key);
    let asset_key = DataKey::AssetOpenChallenges(asset.clone());
    let asset_open = read_u32(env, &asset_key);
    if challenger_open >= MAX_OPEN_CHALLENGES_PER_CHALLENGER
        || asset_open >= MAX_OPEN_CHALLENGES_PER_ASSET
    {
        panic_with_error!(env, ErrorCode::RateLimitExceeded);
    }
    write_u32(env, &challenger_key, challenger_open + 1);
    write_u32(env, &asset_key, asset_open + 1);

    let current_ledger = env.ledger().sequence();

    // Get next challenge ID
    let challenge_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::ChallengeCount)
        .unwrap_or(0);
    let challenge_id = challenge_count + 1;

    // Create challenge record
    let challenge = Challenge {
        id: challenge_id,
        asset: asset.clone(),
        challenger: challenger.clone(),
        expected_price,
        proof_data: proof_data.clone(),
        challenged_ledger: current_ledger,
        is_resolved: false,
        is_valid: false,
        reward_amount: 0,
    };

    // Store challenge
    env.storage()
        .persistent()
        .set(&DataKey::Challenge(challenge_id), &challenge);
    env.storage()
        .persistent()
        .set(&DataKey::ChallengeCount, &challenge_id);

    // Emit event
    ChallengePricedEvent {
        challenge_id,
        asset: asset.clone(),
        challenger: challenger.clone(),
        expected_price,
        challenged_ledger: current_ledger,
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("chall"), challenger.clone(), proof_data);
}

/// Resolve a challenge as valid or invalid.
///
/// Only the admin can resolve challenges. Valid challenges reward the challenger.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `challenge_id` - The ID of the challenge to resolve.
/// * `is_valid` - Whether the challenge is valid (true = reward, false = discard).
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — if the caller is not the admin.
/// * [`ErrorCode::OperationNotFound`] — if the challenge ID doesn't exist.
pub fn resolve_challenge(env: &Env, challenge_id: u32, is_valid: bool) {
    let admin = get_admin(env);
    admin.require_auth();

    let mut challenge: Challenge = env
        .storage()
        .persistent()
        .get(&DataKey::Challenge(challenge_id))
        .ok_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
        .unwrap();

    if challenge.is_resolved {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    challenge.is_resolved = true;
    challenge.is_valid = is_valid;

    let challenger_key = DataKey::ChallengerOpenCount(challenge.challenger.clone());
    write_u32(
        env,
        &challenger_key,
        read_u32(env, &challenger_key).saturating_sub(1),
    );
    let asset_key = DataKey::AssetOpenChallenges(challenge.asset.clone());
    write_u32(env, &asset_key, read_u32(env, &asset_key).saturating_sub(1));

    // Calculate reward if valid
    if is_valid {
        // Simple reward: 0.1% of the challenged price, in the asset's own scale.
        let base_reward = challenge.expected_price / 1000; // 0.1%
        challenge.reward_amount = if base_reward > 0 {
            base_reward
        } else {
            1 // Minimum 1 stroops
        };

        // Track unclaimed rewards for challenger
        let challenger_rewards: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ChallengerRewards(challenge.challenger.clone()))
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::ChallengerRewards(challenge.challenger.clone()),
            &(challenger_rewards + challenge.reward_amount),
        );
    } else {
        // Frivolous challenge (#461): record a strike and forfeit an amount equal
        // to the reward a valid challenge would have earned from unclaimed rewards.
        let strikes_key = DataKey::ChallengerStrikes(challenge.challenger.clone());
        write_u32(env, &strikes_key, read_u32(env, &strikes_key) + 1);
        let rewards_key = DataKey::ChallengerRewards(challenge.challenger.clone());
        let rewards: i128 = env.storage().persistent().get(&rewards_key).unwrap_or(0);
        let penalty = (challenge.expected_price / 1000).max(1);
        env.storage()
            .persistent()
            .set(&rewards_key, &(rewards - penalty).max(0));
    }

    // Update challenge
    env.storage()
        .persistent()
        .set(&DataKey::Challenge(challenge_id), &challenge);

    // Emit event
    ChallengeResolvedEvent {
        challenge_id,
        asset: challenge.asset,
        is_valid,
        reward_amount: challenge.reward_amount,
        resolved_by: admin.clone(),
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("resch"), admin, Bytes::new(env));
}

/// Claim accumulated challenge rewards.
///
/// Transfers all accumulated unclaimed rewards to the caller's account.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
///
/// # Returns
///
/// The amount of rewards claimed (in stroops).
pub fn claim_rewards(env: &Env, claimer: Address) -> i128 {
    claimer.require_auth();
    let rewards: i128 = env
        .storage()
        .persistent()
        .get(&DataKey::ChallengerRewards(claimer.clone()))
        .unwrap_or(0);

    if rewards == 0 {
        return 0;
    }

    // Clear rewards
    env.storage()
        .persistent()
        .remove(&DataKey::ChallengerRewards(claimer.clone()));

    // Emit event
    RewardsClaimedEvent {
        claimer: claimer.clone(),
        amount: rewards,
    }
    .publish(env);

    emit_admin_action(env, symbol_short!("claim"), claimer, Bytes::new(env));

    rewards
}

/// Get the history of challenges for an asset.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `asset` - The asset to query challenges for.
/// * `limit` - Maximum number of challenges to return (0 = no limit, but capped at 100).
///
/// # Returns
///
/// Ordered list of challenges (newest first, up to `limit`).
pub fn get_challenge_history(env: &Env, asset: Address, limit: u32) -> Vec<Challenge> {
    crate::storage::check_registered_asset(env, &asset);

    let effective_limit = if limit == 0 || limit > 100 {
        100
    } else {
        limit
    };

    let challenge_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::ChallengeCount)
        .unwrap_or(0);

    let mut results = Vec::new(env);
    let mut returned = 0u32;

    // Iterate from most recent backwards
    let mut i = challenge_count;
    while i > 0 && returned < effective_limit {
        if let Some(challenge) = env
            .storage()
            .persistent()
            .get::<_, Challenge>(&DataKey::Challenge(i))
        {
            if challenge.asset == asset {
                results.push_back(challenge);
                returned += 1;
            }
        }
        i -= 1;
    }

    results
}

/// Number of unresolved challenges against `asset` (#461).
///
/// Consumers must treat a price with open challenges as disputed.
pub fn get_open_challenge_count(env: &Env, asset: Address) -> u32 {
    read_u32(env, &DataKey::AssetOpenChallenges(asset))
}

/// Get accumulated unclaimed rewards for a challenger.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `challenger` - The address to query rewards for.
///
/// # Returns
///
/// Amount of unclaimed rewards in stroops.
pub fn get_challenger_rewards(env: &Env, challenger: Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::ChallengerRewards(challenger))
        .unwrap_or(0)
}
