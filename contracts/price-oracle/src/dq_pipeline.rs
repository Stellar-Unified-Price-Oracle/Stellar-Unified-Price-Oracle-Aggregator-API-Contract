//! #398 — Pre-aggregation data-quality (DQ) pipeline
//!
//! Every submission that reaches the aggregation loop for an asset with a
//! [`DqConfig`] is screened, in order, by four checks. The first failing check
//! rejects the input, excludes it from the median and emits a
//! [`DqInputRejectedEvent`] carrying the reason, the offending value, the
//! threshold applied and the reference it was compared against — enough to
//! reconstruct every rejection from events alone.
//!
//! | Reason | Check | Reference |
//! |---|---|---|
//! | 1 `STALE` | `now - ledger_timestamp > max_staleness_secs` | submission ledger time |
//! | 2 `BOUNDS` | `price < min_price` or `price > max_price` | the bound breached |
//! | 3 `STEP` | `|price - last| / last > max_step_bps` | last published aggregate |
//! | 4 `DRIFT` | `|price - anchor| / anchor > max_drift_bps` | drift anchor |
//!
//! The step and drift checks compare against *published history*, not against
//! the other submissions in the round, so a majority of colluding sources
//! cannot redefine "normal": every accepted value lies within `max_step_bps` of
//! the last aggregate, hence so does their median. That is the documented
//! bound on how far a round can move the aggregate, and `max_drift_bps` bounds
//! the cumulative movement across one `drift_window_secs`, which is what
//! catches staged drift made of individually-acceptable increments.
//!
//! What each check does **not** stop is written down in
//! `docs/data-quality-pipeline.md` ("Limitations").

use soroban_sdk::{contractevent, contracttype, panic_with_error, Address, Env};

use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregatePrice, DataKey, ErrorCode, PriceEntry};

pub const REASON_STALE: u32 = 1;
pub const REASON_BOUNDS: u32 = 2;
pub const REASON_STEP: u32 = 3;
pub const REASON_DRIFT: u32 = 4;

const BPS: i128 = 10_000;

/// Per-asset DQ thresholds. A zero value disables that individual check.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DqConfig {
    /// Maximum age of a submission, in seconds, measured from its ledger time.
    pub max_staleness_secs: u64,
    /// Lowest accepted price (inclusive).
    pub min_price: i128,
    /// Highest accepted price (inclusive). `0` means unbounded.
    pub max_price: i128,
    /// Maximum per-input distance from the last published aggregate, in bps.
    pub max_step_bps: u32,
    /// Maximum per-input distance from the drift anchor, in bps.
    pub max_drift_bps: u32,
    /// Lifetime of a drift anchor, in seconds. The anchor is re-based on the
    /// last published aggregate once it expires.
    pub drift_window_secs: u64,
}

/// Reference point the drift check measures cumulative movement against.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DqAnchor {
    pub price: i128,
    pub set_at: u64,
}

#[contracttype]
#[derive(Clone)]
enum DqKey {
    Config(Address),
    Anchor(Address),
}

/// Emitted once per rejected input.
#[contractevent]
#[derive(Clone)]
pub struct DqInputRejectedEvent {
    #[topic]
    pub asset: Address,
    #[topic]
    pub source: Address,
    /// One of the `REASON_*` constants.
    pub reason: u32,
    /// The measured value: age in seconds for `STALE`, the price otherwise.
    pub value: i128,
    /// The threshold applied (seconds, price or bps, matching `reason`).
    pub threshold: i128,
    /// What `value` was compared against (see the module table).
    pub reference: i128,
}

/// Emitted when a DQ configuration is set or cleared for an asset.
#[contractevent]
#[derive(Clone)]
pub struct DqConfigUpdatedEvent {
    #[topic]
    pub asset: Address,
    pub enabled: bool,
}

/// Sets the DQ configuration for `asset`. Admin-only.
pub fn set_config(env: &Env, asset: Address, cfg: DqConfig) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    let invalid = cfg.min_price < 0
        || (cfg.max_price != 0 && cfg.max_price < cfg.min_price)
        || cfg.max_step_bps as i128 > BPS
        || cfg.max_drift_bps as i128 > BPS
        || (cfg.max_drift_bps > 0 && cfg.drift_window_secs == 0)
        || (cfg.max_drift_bps > 0 && cfg.max_drift_bps < cfg.max_step_bps);
    if invalid {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let key = DqKey::Config(asset.clone());
    env.storage().persistent().set(&key, &cfg);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    env.storage()
        .persistent()
        .remove(&DqKey::Anchor(asset.clone()));
    DqConfigUpdatedEvent {
        asset,
        enabled: true,
    }
    .publish(env);
}

/// Removes the DQ configuration for `asset`. Admin-only.
pub fn clear_config(env: &Env, asset: Address) {
    get_admin(env).require_auth();
    env.storage()
        .persistent()
        .remove(&DqKey::Config(asset.clone()));
    env.storage()
        .persistent()
        .remove(&DqKey::Anchor(asset.clone()));
    DqConfigUpdatedEvent {
        asset,
        enabled: false,
    }
    .publish(env);
}

pub fn get_config(env: &Env, asset: &Address) -> Option<DqConfig> {
    env.storage()
        .persistent()
        .get(&DqKey::Config(asset.clone()))
}

pub fn get_anchor(env: &Env, asset: &Address) -> Option<DqAnchor> {
    env.storage()
        .persistent()
        .get(&DqKey::Anchor(asset.clone()))
}

/// Per-round screening context, built once per aggregation pass.
pub struct DqRound {
    cfg: DqConfig,
    last: i128,
    anchor: i128,
}

/// Loads the DQ context for `asset`, or `None` when DQ is not configured (one
/// storage read on the common path). Re-bases an expired drift anchor on the
/// last published aggregate.
pub fn begin(env: &Env, asset: &Address) -> Option<DqRound> {
    let cfg = get_config(env, asset)?;
    let last = env
        .storage()
        .persistent()
        .get::<_, AggregatePrice>(&DataKey::Aggregate(asset.clone()))
        .map(|a| a.price)
        .unwrap_or(0);
    let now = env.ledger().timestamp();
    let mut anchor = 0;
    if cfg.max_drift_bps > 0 && last > 0 {
        anchor = match get_anchor(env, asset) {
            Some(a) if now.saturating_sub(a.set_at) < cfg.drift_window_secs => a.price,
            _ => {
                let key = DqKey::Anchor(asset.clone());
                env.storage().persistent().set(
                    &key,
                    &DqAnchor {
                        price: last,
                        set_at: now,
                    },
                );
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
                last
            }
        };
    }
    Some(DqRound { cfg, last, anchor })
}

/// `true` when `|price - reference| / reference` exceeds `bps`. Compared by
/// cross-multiplication so truncation can never admit a value past the bound.
pub fn exceeds_bps(price: i128, reference: i128, bps: u32) -> bool {
    (price - reference).saturating_abs().saturating_mul(BPS) > reference.saturating_mul(bps as i128)
}

/// Screens one input. Returns `true` when it may enter aggregation; otherwise
/// emits a [`DqInputRejectedEvent`] and returns `false`.
pub fn screen(
    env: &Env,
    round: &DqRound,
    asset: &Address,
    source: &Address,
    entry: &PriceEntry,
) -> bool {
    let cfg = &round.cfg;
    let price = entry.price;
    let age = env
        .ledger()
        .timestamp()
        .saturating_sub(entry.ledger_timestamp);
    let rejection = if cfg.max_staleness_secs > 0 && age > cfg.max_staleness_secs {
        Some((
            REASON_STALE,
            age as i128,
            cfg.max_staleness_secs as i128,
            entry.ledger_timestamp as i128,
        ))
    } else if price < cfg.min_price {
        Some((REASON_BOUNDS, price, cfg.min_price, cfg.min_price))
    } else if cfg.max_price > 0 && price > cfg.max_price {
        Some((REASON_BOUNDS, price, cfg.max_price, cfg.max_price))
    } else if cfg.max_step_bps > 0
        && round.last > 0
        && exceeds_bps(price, round.last, cfg.max_step_bps)
    {
        Some((REASON_STEP, price, cfg.max_step_bps as i128, round.last))
    } else if cfg.max_drift_bps > 0
        && round.anchor > 0
        && exceeds_bps(price, round.anchor, cfg.max_drift_bps)
    {
        Some((REASON_DRIFT, price, cfg.max_drift_bps as i128, round.anchor))
    } else {
        None
    };
    match rejection {
        None => true,
        Some((reason, value, threshold, reference)) => {
            DqInputRejectedEvent {
                asset: asset.clone(),
                source: source.clone(),
                reason,
                value,
                threshold,
                reference,
            }
            .publish(env);
            false
        }
    }
}
