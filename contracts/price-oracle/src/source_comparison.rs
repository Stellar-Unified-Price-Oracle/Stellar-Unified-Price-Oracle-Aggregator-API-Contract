//! #401 — Cross-source comparison dashboard (data layer)
//!
//! When enabled for an asset, every published round is indexed as a
//! [`ComparisonRound`]: the aggregate, every *admitted* source's value (a
//! registered, eligible source with a stored submission) and whether that value
//! was *counted* in the published aggregate. A bounded window of
//! [`WINDOW_ROUNDS`] rounds is kept per asset.
//!
//! [`report`] turns that window into the dashboard's data:
//!
//! * **Deviation** — per-source mean signed deviation from the aggregate (bps).
//! * **Direction persistence** — share of admitted rounds whose deviation has
//!   the source's dominant sign.
//! * **Collusion signal** — for each pair of sources, the cosine similarity of
//!   their deviation vectors over shared rounds. Two independent honest
//!   sources deviate with uncorrelated signs (similarity ≈ 0); a coordinated
//!   set leans the same way every round (similarity ≈ 10 000) even when each
//!   series stays inside its own deviation bound.
//! * **Influence** — mean leave-one-out shift: how far the published median
//!   would have moved had the source's counted value been removed (bps).
//! * **Exclusion** — sources admitted in at least [`MIN_ROUNDS`] rounds but
//!   counted in fewer than [`EXCLUSION_COUNTED_BPS`] of them.
//!
//! The analysis is a pure read and never changes a price or a source set;
//! removal is the source lifecycle's job (#402). Thresholds and their
//! false-positive behaviour are documented in `docs/source-comparison.md`.

use soroban_sdk::{contractevent, contracttype, Address, Env, Vec};

use crate::storage::{
    check_registered_asset, compute_median, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD,
};

/// Rounds retained per asset.
pub const WINDOW_ROUNDS: u32 = 16;
/// Minimum shared/admitted rounds before any flag is raised.
pub const MIN_ROUNDS: u32 = 4;
/// Pair similarity at or above which a pair is flagged (0.8).
pub const COLLUSION_SIMILARITY_BPS: i128 = 8_000;
/// Direction persistence both members of a flagged pair must reach (70 %).
pub const PERSISTENCE_BPS: u32 = 7_000;
/// A source counted in fewer than this share of admitted rounds is flagged.
pub const EXCLUSION_COUNTED_BPS: u32 = 2_000;

const BPS: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparisonRound {
    pub aggregate: i128,
    pub sources: Vec<Address>,
    pub prices: Vec<i128>,
    pub counted: Vec<bool>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceComparisonStats {
    pub source: Address,
    pub rounds_admitted: u32,
    pub rounds_counted: u32,
    pub mean_deviation_bps: i128,
    pub direction_persistence_bps: u32,
    pub influence_bps: i128,
    pub excluded: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollusionPair {
    pub a: Address,
    pub b: Address,
    pub similarity_bps: i128,
    pub shared_rounds: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComparisonReport {
    pub rounds: u32,
    pub sources: Vec<SourceComparisonStats>,
    /// Flagged pairs only.
    pub collusion: Vec<CollusionPair>,
}

#[contracttype]
#[derive(Clone)]
enum CmpKey {
    Enabled(Address),
    Rounds(Address),
}

#[contractevent]
#[derive(Clone)]
pub struct SourceComparisonToggledEvent {
    #[topic]
    pub asset: Address,
    pub enabled: bool,
}

/// Enables or disables round indexing for `asset`. Admin-only. Disabling also
/// drops the indexed window.
pub fn set_enabled(env: &Env, asset: Address, enabled: bool) {
    get_admin(env).require_auth();
    check_registered_asset(env, &asset);
    if enabled {
        env.storage()
            .persistent()
            .set(&CmpKey::Enabled(asset.clone()), &true);
    } else {
        env.storage()
            .persistent()
            .remove(&CmpKey::Enabled(asset.clone()));
        env.storage()
            .persistent()
            .remove(&CmpKey::Rounds(asset.clone()));
    }
    SourceComparisonToggledEvent { asset, enabled }.publish(env);
}

pub fn is_enabled(env: &Env, asset: &Address) -> bool {
    env.storage()
        .persistent()
        .has(&CmpKey::Enabled(asset.clone()))
}

pub fn get_rounds(env: &Env, asset: &Address) -> Vec<ComparisonRound> {
    env.storage()
        .persistent()
        .get(&CmpKey::Rounds(asset.clone()))
        .unwrap_or_else(|| Vec::new(env))
}

/// Aggregation hook: appends one published round to the window.
pub fn record(
    env: &Env,
    asset: &Address,
    aggregate: i128,
    admitted: &Vec<Address>,
    admitted_prices: &Vec<i128>,
    counted: &Vec<Address>,
) {
    let mut flags = Vec::new(env);
    for s in admitted.iter() {
        flags.push_back(counted.contains(&s));
    }
    let mut rounds = get_rounds(env, asset);
    rounds.push_back(ComparisonRound {
        aggregate,
        sources: admitted.clone(),
        prices: admitted_prices.clone(),
        counted: flags,
    });
    while rounds.len() > WINDOW_ROUNDS {
        rounds.pop_front();
    }
    let key = CmpKey::Rounds(asset.clone());
    env.storage().persistent().set(&key, &rounds);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn report(env: &Env, asset: &Address) -> ComparisonReport {
    analyze(env, &get_rounds(env, asset))
}

fn deviation_bps(price: i128, aggregate: i128) -> i128 {
    if aggregate <= 0 {
        return 0;
    }
    (price - aggregate).saturating_mul(BPS) / aggregate
}

fn isqrt(n: u128) -> u128 {
    if n < 2 {
        return n;
    }
    let mut x = n;
    let mut y = x.div_ceil(2);
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

/// Deviation of `source` in round `r`, or `None` when it was not admitted.
fn deviation_in(r: &ComparisonRound, source: &Address) -> Option<i128> {
    let idx = r.sources.first_index_of(source)?;
    Some(deviation_bps(r.prices.get_unchecked(idx), r.aggregate))
}

/// Pure analysis over a window of rounds; exposed for synthetic-scenario tests.
pub fn analyze(env: &Env, rounds: &Vec<ComparisonRound>) -> ComparisonReport {
    let mut sources: Vec<Address> = Vec::new(env);
    for r in rounds.iter() {
        for s in r.sources.iter() {
            if !sources.contains(&s) {
                sources.push_back(s);
            }
        }
    }

    let mut stats: Vec<SourceComparisonStats> = Vec::new(env);
    for s in sources.iter() {
        let (mut admitted, mut counted, mut pos, mut neg) = (0u32, 0u32, 0u32, 0u32);
        let (mut dev_sum, mut infl_sum) = (0i128, 0i128);
        for r in rounds.iter() {
            let Some(idx) = r.sources.first_index_of(&s) else {
                continue;
            };
            admitted += 1;
            let d = deviation_bps(r.prices.get_unchecked(idx), r.aggregate);
            dev_sum += d;
            if d > 0 {
                pos += 1;
            } else if d < 0 {
                neg += 1;
            }
            if r.counted.get_unchecked(idx) {
                counted += 1;
                let mut with = Vec::new(env);
                let mut without = Vec::new(env);
                for j in 0..r.sources.len() {
                    if r.counted.get_unchecked(j) {
                        with.push_back(r.prices.get_unchecked(j));
                        if j != idx {
                            without.push_back(r.prices.get_unchecked(j));
                        }
                    }
                }
                if !without.is_empty() {
                    let shift = compute_median(&with) - compute_median(&without);
                    infl_sum += deviation_bps(r.aggregate + shift.abs(), r.aggregate);
                }
            }
        }
        let excluded = admitted >= MIN_ROUNDS
            && (counted as i128) * BPS < (admitted as i128) * EXCLUSION_COUNTED_BPS as i128;
        stats.push_back(SourceComparisonStats {
            source: s,
            rounds_admitted: admitted,
            rounds_counted: counted,
            mean_deviation_bps: if admitted > 0 {
                dev_sum / admitted as i128
            } else {
                0
            },
            direction_persistence_bps: if admitted > 0 {
                (pos.max(neg) as i128 * BPS / admitted as i128) as u32
            } else {
                0
            },
            influence_bps: if counted > 0 {
                infl_sum / counted as i128
            } else {
                0
            },
            excluded,
        });
    }

    let mut collusion: Vec<CollusionPair> = Vec::new(env);
    for i in 0..sources.len() {
        for j in (i + 1)..sources.len() {
            let (a, b) = (sources.get_unchecked(i), sources.get_unchecked(j));
            let (mut dot, mut na, mut nb, mut shared) = (0i128, 0i128, 0i128, 0u32);
            for r in rounds.iter() {
                if let (Some(da), Some(db)) = (deviation_in(&r, &a), deviation_in(&r, &b)) {
                    dot += da * db;
                    na += da * da;
                    nb += db * db;
                    shared += 1;
                }
            }
            if shared < MIN_ROUNDS || na == 0 || nb == 0 {
                continue;
            }
            let norm = isqrt((na as u128) * (nb as u128)) as i128;
            let similarity_bps = if norm > 0 { dot * BPS / norm } else { 0 };
            let persistent = stats.get_unchecked(i).direction_persistence_bps >= PERSISTENCE_BPS
                && stats.get_unchecked(j).direction_persistence_bps >= PERSISTENCE_BPS;
            if similarity_bps >= COLLUSION_SIMILARITY_BPS && persistent {
                collusion.push_back(CollusionPair {
                    a,
                    b,
                    similarity_bps,
                    shared_rounds: shared,
                });
            }
        }
    }

    ComparisonReport {
        rounds: rounds.len(),
        sources: stats,
        collusion,
    }
}
