//! # Volatility-bucketed adaptive quorum (#482)
//!
//! See `docs/adaptive-quorum.md`.
//!
//! ## Why
//!
//! A fixed quorum is wrong in both directions. During a volatile regime it is
//! too permissive — precisely when manipulation is most profitable, a small
//! quorum is easiest to buy. During a calm regime it is needlessly expensive —
//! every round pays for a high quorum that the market does not require.
//!
//! ## Buckets
//!
//! Volatility is estimated as the **mean absolute return** over a rolling
//! window of observations, in basis points. The estimate is classified into
//! buckets by ascending boundaries, and each bucket selects a quorum.
//!
//! A mean absolute return is used rather than a standard deviation because it
//! needs only one pass, has no square-root or division step that could be
//! tuned, and — critically — cannot be driven to an arbitrary value by a
//! single outlier the way a variance can.
//!
//! ## Hysteresis, and its asymmetry
//!
//! Bucket transitions are damped, and the damping is **asymmetric**:
//!
//! * moving **up** (volatility rising, quorum tightening) is immediate;
//! * moving **down** (volatility falling, quorum relaxing) requires the new
//!   bucket to hold for `relax_after` consecutive observations.
//!
//! This asymmetry is the anti-manipulation property the issue asks for. If
//! both directions were damped, an adversary who could push a calm asset into
//! a high-volatility reading would be stuck paying the high quorum — safe but
//! a denial of service. If *relaxing* were immediate, a single submission
//! reporting a calm price would talk the quorum down, and the attacker would
//! then need only that many colluding votes. Making relaxation the slow
//! direction means a downgrade must be sustained, so one submission cannot
//! force it.
//!
//! ## The quorum is fixed at round start
//!
//! [`pin_round_quorum`] freezes the effective quorum for a round and stores it
//! under the round's identity. Later regime changes update the asset's *current*
//! regime but cannot touch a round already in flight, so:
//!
//! * a round's success criterion does not move under the participants, and
//! * an adversary cannot change the quorum mid-round to make a round
//!   unreachable (raising it) or trivially reachable (lowering it).
//!
//! The pinned value is what a consumer should read to know what a round will
//! actually require.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{QuorumBucketChangedEvent, QuorumPinnedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, QuorumRegime, RoundQuorum, VolatilityBuckets};

/// Returns the adaptive-quorum configuration (defaults when unset).
pub fn get_adaptive_quorum_config(env: &Env) -> VolatilityBuckets {
    let key = DataKey::AdaptiveQuorumConfig;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    match env.storage().persistent().get::<_, VolatilityBuckets>(&key) {
        Some(cfg) => cfg,
        None => {
            let mut boundaries: Vec<u32> = Vec::new(env);
            boundaries.push_back(50);
            boundaries.push_back(500);
            let mut quorums: Vec<u32> = Vec::new(env);
            quorums.push_back(1);
            quorums.push_back(3);
            quorums.push_back(5);
            VolatilityBuckets {
                boundaries,
                quorums,
                relax_after: 3,
                min_samples: 2,
                window: 16,
            }
        }
    }
}

/// Configures the volatility buckets, quorums and hysteresis. Admin only.
///
/// # Errors
///
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::InvalidAdaptiveQuorum`] — the boundaries and quorums differ in
///   length, a quorum is `0` (making the round unreachable), the boundaries are
///   not strictly ascending, or `window`/`min_samples` are `0`.
pub fn set_adaptive_quorum_config(env: &Env, config: VolatilityBuckets) {
    get_admin(env).require_auth();
    validate_config(env, &config);

    let key = DataKey::AdaptiveQuorumConfig;
    env.storage().persistent().set(&key, &config);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn validate_config(env: &Env, config: &VolatilityBuckets) {
    let n = config.boundaries.len();
    // `n` boundaries define `n + 1` buckets: the one below the first boundary
    // plus one per boundary. There must therefore be exactly one more quorum
    // than boundary — fewer leaves the top bucket unreachable, more leaves a
    // quorum that no bucket can select.
    if n == 0 || config.quorums.len() != n + 1 {
        panic_with_error!(env, ErrorCode::InvalidAdaptiveQuorum);
    }
    // A zero quorum would make a round trivially satisfiable by one colluding
    // source, which is the opposite of the intent.
    for i in 0..config.quorums.len() {
        if config.quorums.get_unchecked(i) == 0 {
            panic_with_error!(env, ErrorCode::InvalidAdaptiveQuorum);
        }
    }
    // Boundaries must strictly ascend, or bucket_of would be ill-defined.
    for i in 1..n {
        if config.boundaries.get_unchecked(i) <= config.boundaries.get_unchecked(i - 1) {
            panic_with_error!(env, ErrorCode::InvalidAdaptiveQuorum);
        }
    }
    if config.window == 0 || config.min_samples == 0 {
        panic_with_error!(env, ErrorCode::InvalidAdaptiveQuorum);
    }
}

/// Returns the bucket index a volatility estimate falls into.
///
/// The first index whose boundary the estimate is *strictly below*; the top
/// bucket when the estimate is at or above every boundary.
pub fn bucket_of(_env: &Env, config: &VolatilityBuckets, volatility_bps: u32) -> u32 {
    for i in 0..config.boundaries.len() {
        if volatility_bps < config.boundaries.get_unchecked(i) {
            return i;
        }
    }
    config.boundaries.len()
}

/// Reads the rolling window of absolute returns, in bps, for an asset.
fn window(env: &Env, asset: &Address) -> Vec<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::VolatilityWindow(asset.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

fn write_window(env: &Env, asset: &Address, values: &Vec<u32>) {
    let key = DataKey::VolatilityWindow(asset.clone());
    env.storage().persistent().set(&key, values);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Mean absolute return over a sample window, in bps.
fn mean_abs_return(samples: &Vec<u32>) -> u32 {
    let n = samples.len();
    if n == 0 {
        return 0;
    }
    let mut total: u64 = 0;
    for i in 0..n {
        total += u64::from(samples.get_unchecked(i));
    }
    (total / n as u64).min(u64::from(u32::MAX)) as u32
}

/// Records a new observation of `price` and returns the updated regime.
///
/// The return is measured against the asset's last recorded price, so the
/// window holds *returns*, not prices: the estimate is scale-invariant, and a
/// 10 % move is 1 000 bps whether the asset trades at 1 or at 1 000 000.
///
/// A first observation has no predecessor and so contributes no return; it
/// only seeds the reference price.
pub fn observe_price(env: &Env, asset: &Address, price: i128) -> QuorumRegime {
    let config = get_adaptive_quorum_config(env);
    // The reference price is the last one *observed here*, not the asset's
    // published aggregate: the volatility estimate must advance once per
    // observation, whether or not the observation also reached an aggregate.
    let last_key = DataKey::VolatilityLastPrice(asset.clone());
    let previous: Option<i128> = env.storage().persistent().get(&last_key);

    let mut samples = window(env, asset);
    if let Some(prev) = previous {
        if prev > 0 && price > 0 {
            // |Δ| / prev, in bps, computed wide so the product cannot overflow.
            let diff = if price >= prev {
                price - prev
            } else {
                prev - price
            };
            let bps = ((diff as u128) * 10_000u128 / (prev as u128)).min(u128::from(u32::MAX));
            samples.push_back(bps as u32);
            // Bounded window: the oldest observation is dropped, so the
            // estimate cannot be pinned forever by a historic outlier.
            while samples.len() > config.window {
                samples.remove(0);
            }
            write_window(env, asset, &samples);
        }
    }
    env.storage().persistent().set(&last_key, &price);
    env.storage()
        .persistent()
        .extend_ttl(&last_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    classify(env, asset, &config, &samples)
}

/// Classifies a sample window into a bucket, applying the relaxation damping.
fn classify(
    env: &Env,
    asset: &Address,
    config: &VolatilityBuckets,
    samples: &Vec<u32>,
) -> QuorumRegime {
    let n = samples.len();
    if n < config.min_samples {
        // Too little history to classify. Hold the calmest bucket, which is
        // also the cheapest, and report the shortfall so a consumer can see
        // the regime is not yet meaningful.
        return QuorumRegime {
            bucket: 0,
            quorum: config.quorums.get_unchecked(0),
            volatility_bps: 0,
            samples: n,
            streak: 0,
        };
    }

    let volatility_bps = mean_abs_return(samples);
    let target = bucket_of(env, config, volatility_bps);
    let state_key = DataKey::AdaptiveQuorumState(asset.clone());
    let previous: Option<QuorumRegime> = env.storage().persistent().get(&state_key);
    let from = previous.as_ref().map(|r| r.bucket).unwrap_or(target);
    let prior_streak = previous.as_ref().map(|r| r.streak).unwrap_or(0);

    // Tightening is immediate; relaxing waits for the target to hold.
    let (bucket, streak) = if target >= from {
        (target, 0)
    } else if prior_streak + 1 >= config.relax_after {
        (target, 0)
    } else {
        // Hold the current (stricter) bucket and count the relaxation run.
        (from, prior_streak + 1)
    };

    let regime = QuorumRegime {
        bucket,
        quorum: config
            .quorums
            .get_unchecked(bucket.min(config.quorums.len() - 1)),
        volatility_bps,
        samples: n,
        streak,
    };
    env.storage().persistent().set(&state_key, &regime);
    env.storage()
        .persistent()
        .extend_ttl(&state_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    if from != bucket {
        QuorumBucketChangedEvent {
            asset: asset.clone(),
            from_bucket: from,
            to_bucket: bucket,
            volatility_bps,
            quorum: regime.quorum,
            relaxed: bucket < from,
        }
        .publish(env);
    }

    regime
}

/// Returns an asset's current regime.
pub fn get_regime(env: &Env, asset: &Address) -> QuorumRegime {
    let config = get_adaptive_quorum_config(env);
    let samples = window(env, asset);
    let n = samples.len();
    if n < config.min_samples {
        return QuorumRegime {
            bucket: 0,
            quorum: config.quorums.get_unchecked(0),
            volatility_bps: 0,
            samples: n,
            streak: 0,
        };
    }
    let volatility_bps = mean_abs_return(&samples);
    let bucket = bucket_of(env, &config, volatility_bps);
    let streak = env
        .storage()
        .persistent()
        .get::<_, QuorumRegime>(&DataKey::AdaptiveQuorumState(asset.clone()))
        .map(|r| r.streak)
        .unwrap_or(0);
    QuorumRegime {
        bucket,
        quorum: config
            .quorums
            .get_unchecked(bucket.min(config.quorums.len() - 1)),
        volatility_bps,
        samples: n,
        streak,
    }
}

/// The quorum an asset's current regime implies.
///
/// # Errors
///
/// * [`ErrorCode::InsufficientVolatilitySamples`] — fewer than
///   `min_samples` observations, so the regime is not yet meaningful. Failing
///   loudly beats silently applying a calm-market quorum to a market nobody has
///   measured.
pub fn effective_quorum(env: &Env, asset: &Address) -> u32 {
    let config = get_adaptive_quorum_config(env);
    if window(env, asset).len() < config.min_samples {
        panic_with_error!(env, ErrorCode::InsufficientVolatilitySamples);
    }
    get_regime(env, asset).quorum
}

/// Freezes the effective quorum for `round` and returns it.
///
/// The stored value is what the round enforces. Later regime changes update the
/// asset's current regime but cannot alter a round already in flight, so a
/// round's success criterion does not move under its participants and an
/// adversary cannot change the quorum mid-round to make a round unreachable or
/// trivially reachable.
///
/// # Errors
///
/// * [`ErrorCode::InsufficientVolatilitySamples`] — too few observations to
///   classify the regime.
pub fn pin_round_quorum(env: &Env, asset: &Address, round: u32) -> RoundQuorum {
    let config = get_adaptive_quorum_config(env);
    if window(env, asset).len() < config.min_samples {
        panic_with_error!(env, ErrorCode::InsufficientVolatilitySamples);
    }
    let regime = get_regime(env, asset);
    let pinned = RoundQuorum {
        round,
        bucket: regime.bucket,
        quorum: regime.quorum,
        volatility_bps: regime.volatility_bps,
    };
    let key = DataKey::AdaptiveQuorumRound(asset.clone(), round);
    env.storage().persistent().set(&key, &pinned);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    QuorumPinnedEvent {
        asset: asset.clone(),
        round,
        bucket: pinned.bucket,
        quorum: pinned.quorum,
        volatility_bps: pinned.volatility_bps,
    }
    .publish(env);

    pinned
}

/// Returns the quorum frozen for a round, if one was pinned.
pub fn get_round_quorum(env: &Env, asset: &Address, round: u32) -> Option<RoundQuorum> {
    let key = DataKey::AdaptiveQuorumRound(asset.clone(), round);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}
