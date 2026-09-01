//! # Relayer Dashboard (#267)
//!
//! Aggregates per-relayer operational metrics — volume, accuracy, latency, fee and
//! reward earnings, per-asset breakdown, and a comparative percentile rank against
//! every other approved relayer — into a single [`RelayerDashboard`] snapshot.
//!
//! Follows the same pragmatic, counter-based approach as `analytics.rs`'s
//! `get_source_analytics`: metrics are derived from a handful of running sums/counts
//! updated incrementally on the hot submission path, rather than a full historical
//! time-series, to keep per-submission storage overhead flat.

use soroban_sdk::{Address, Env, Map, Vec};

use crate::types::{DataKey, RelayerAssetStat, RelayerDashboard};

/// Approximate number of ledgers per day, assuming Stellar's ~5-second ledger close
/// time. Used to convert a relayer's submission count into a submissions/day rate.
pub const LEDGERS_PER_DAY: u32 = 17_280;

/// Records incremental per-submission stats (latency + per-asset counts) used by the
/// dashboard. Called internally by `relayer.rs` after a submission is stored.
pub fn record_relayer_submission_stats(
    env: &Env,
    relayer: &Address,
    asset: &Address,
    observation_timestamp: u64,
    ledger_timestamp: u64,
) {
    // Latency: absolute distance between the observation timestamp and ledger close.
    let latency = if ledger_timestamp >= observation_timestamp {
        ledger_timestamp - observation_timestamp
    } else {
        observation_timestamp - ledger_timestamp
    };

    let latency_key = DataKey::RelayerLatencySum(relayer.clone());
    let latency_sum: u64 = env.storage().persistent().get(&latency_key).unwrap_or(0u64);
    env.storage()
        .persistent()
        .set(&latency_key, &latency_sum.saturating_add(latency));
    env.storage().persistent().extend_ttl(
        &latency_key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );

    let history_key = DataKey::RelayerSubmissionHistory(relayer.clone());
    let mut history: Vec<u64> = env
        .storage()
        .persistent()
        .get(&history_key)
        .unwrap_or(Vec::new(env));
    history.push_back(observation_timestamp);
    env.storage().persistent().set(&history_key, &history);
    env.storage().persistent().extend_ttl(
        &history_key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );

    let latency_hist_key = DataKey::RelayerLatencyHistory(relayer.clone());
    let mut latency_history: Vec<u64> = env
        .storage()
        .persistent()
        .get(&latency_hist_key)
        .unwrap_or(Vec::new(env));
    latency_history.push_back(latency);
    env.storage().persistent().set(&latency_hist_key, &latency_history);
    env.storage().persistent().extend_ttl(
        &latency_hist_key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );

    // Per-asset submission count, tracking newly-seen assets in an enumerable list.
    let list_key = DataKey::RelayerAssetList(relayer.clone());
    let mut assets: Vec<Address> = env
        .storage()
        .persistent()
        .get(&list_key)
        .unwrap_or(Vec::new(env));
    if !assets.contains(asset) {
        assets.push_back(asset.clone());
        env.storage().persistent().set(&list_key, &assets);
        env.storage().persistent().extend_ttl(
            &list_key,
            crate::storage::LEDGER_THRESHOLD,
            crate::storage::LEDGER_BUMP,
        );
    }

    let count_key = DataKey::RelayerAssetCount(relayer.clone(), asset.clone());
    let count: u64 = env.storage().persistent().get(&count_key).unwrap_or(0u64);
    env.storage()
        .persistent()
        .set(&count_key, &count.saturating_add(1));
    env.storage().persistent().extend_ttl(
        &count_key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );

    let asset_latency_sum_key = DataKey::RelayerAssetLatencySum(relayer.clone(), asset.clone());
    let asset_latency_sum: u64 = env
        .storage()
        .persistent()
        .get(&asset_latency_sum_key)
        .unwrap_or(0u64);
    env.storage()
        .persistent()
        .set(&asset_latency_sum_key, &asset_latency_sum.saturating_add(latency));
    env.storage().persistent().extend_ttl(
        &asset_latency_sum_key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );

    let asset_latency_count_key = DataKey::RelayerAssetLatencyCount(relayer.clone(), asset.clone());
    let asset_latency_count: u64 = env
        .storage()
        .persistent()
        .get(&asset_latency_count_key)
        .unwrap_or(0u64);
    env.storage()
        .persistent()
        .set(&asset_latency_count_key, &asset_latency_count.saturating_add(1));
    env.storage().persistent().extend_ttl(
        &asset_latency_count_key,
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );
}

/// Builds and returns the aggregated [`RelayerDashboard`] for `relayer`.
///
/// Returns a dashboard populated with zeroed metrics if `relayer` has never
/// submitted or is not (or is no longer) approved.
pub fn get_relayer_dashboard(env: &Env, relayer: Address) -> RelayerDashboard {
    let total_submissions = crate::relayer::get_relayer_submission_count(env, relayer.clone());
    let failed_submissions = crate::relayer_bonds::get_relayer_failure_count(env, relayer.clone());

    let success_rate_denom = total_submissions
        .saturating_add(failed_submissions as u64)
        .max(1);
    let success_rate_bps = ((total_submissions.saturating_mul(10_000)) / success_rate_denom) as u32;

    let latency_sum: u64 = env
        .storage()
        .persistent()
        .get(&DataKey::RelayerLatencySum(relayer.clone()))
        .unwrap_or(0u64);
    let avg_latency_seconds = if total_submissions > 0 {
        latency_sum / total_submissions
    } else {
        0
    };

    let submission_history: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::RelayerSubmissionHistory(relayer.clone()))
        .unwrap_or(Vec::new(env));
    let latency_percentiles = compute_latency_percentiles(env, &relayer);

    let submissions_per_day = match crate::relayer::get_relayer_info(env, relayer.clone()) {
        Some(info) => {
            let elapsed_ledgers = env
                .ledger()
                .sequence()
                .saturating_sub(info.approved_at_ledger);
            let elapsed_days = (elapsed_ledgers / LEDGERS_PER_DAY).max(1) as u64;
            total_submissions / elapsed_days
        }
        None => 0,
    };

    let fee_earnings = crate::relayer::get_relayer_fee_balance(env, relayer.clone());
    let reward_earnings = crate::relayer_bonds::get_relayer_reward_balance(env, relayer.clone());
    let bond_deposited = crate::relayer_bonds::get_relayer_bond_balance(env, relayer.clone());

    let percentile_rank = compute_percentile_rank(env, total_submissions);
    let per_asset = collect_per_asset_stats(env, &relayer);

    RelayerDashboard {
        relayer,
        total_submissions,
        failed_submissions,
        success_rate_bps,
        submissions_per_day,
        avg_latency_seconds,
        submission_history,
        latency_percentiles,
        fee_earnings,
        reward_earnings,
        bond_deposited,
        percentile_rank,
        per_asset,
    }
}

/// Percentile rank (0-100) of `total_submissions` among every approved relayer in the
/// [`DataKey::RelayerRegistry`]: the percentage of relayers whose own submission
/// count is at or below this one's.
fn compute_percentile_rank(env: &Env, total_submissions: u64) -> u32 {
    let registry: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::RelayerRegistry)
        .unwrap_or(Vec::new(env));

    let total_relayers = registry.len();
    if total_relayers == 0 {
        return 0;
    }

    let mut le_count: u32 = 0;
    for i in 0..registry.len() {
        let other = registry.get_unchecked(i);
        let other_count = crate::relayer::get_relayer_submission_count(env, other);
        if other_count <= total_submissions {
            le_count += 1;
        }
    }

    (le_count * 100) / total_relayers
}

fn collect_per_asset_stats(env: &Env, relayer: &Address) -> Vec<RelayerAssetStat> {
    let asset_list: Vec<Address> = env
        .storage()
        .persistent()
        .get(&DataKey::RelayerAssetList(relayer.clone()))
        .unwrap_or(Vec::new(env));

    let mut per_asset: Vec<RelayerAssetStat> = Vec::new(env);
    for i in 0..asset_list.len() {
        let asset = asset_list.get_unchecked(i);
        let submissions: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::RelayerAssetCount(relayer.clone(), asset.clone()))
            .unwrap_or(0u64);
        let total_attempts = submissions.saturating_add(
            crate::relayer_bonds::get_relayer_failure_count(env, relayer.clone()) as u64,
        );
        let success_rate_bps = if total_attempts == 0 {
            0
        } else {
            ((submissions.saturating_mul(10_000)) / total_attempts.max(1)) as u32
        };
        let latency_sum: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::RelayerAssetLatencySum(relayer.clone(), asset.clone()))
            .unwrap_or(0u64);
        let latency_count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::RelayerAssetLatencyCount(relayer.clone(), asset.clone()))
            .unwrap_or(0u64);
        let avg_latency_seconds = if latency_count > 0 {
            latency_sum / latency_count
        } else {
            0
        };

        per_asset.push_back(RelayerAssetStat {
            asset,
            submissions,
            successful_submissions: submissions,
            failed_submissions: crate::relayer_bonds::get_relayer_failure_count(env, relayer.clone()) as u32,
            success_rate_bps,
            avg_latency_seconds,
        });
    }
    per_asset
}

fn compute_latency_percentiles(env: &Env, relayer: &Address) -> Map<u32, u64> {
    let latency_history: Vec<u64> = env
        .storage()
        .persistent()
        .get(&DataKey::RelayerLatencyHistory(relayer.clone()))
        .unwrap_or(Vec::new(env));
    let mut values: Vec<u64> = Vec::new(env);
    for i in 0..latency_history.len() {
        values.push_back(latency_history.get_unchecked(i));
    }
    if values.is_empty() {
        return Map::new(env);
    }

    let mut sorted = Vec::new(env);
    for i in 0..values.len() {
        sorted.push_back(values.get_unchecked(i));
    }
    for i in 0..sorted.len() {
        let mut min_index = i;
        for j in (i + 1)..sorted.len() {
            if sorted.get_unchecked(j) < sorted.get_unchecked(min_index) {
                min_index = j;
            }
        }
        if min_index != i {
            let tmp = sorted.get_unchecked(i);
            sorted.set(i, sorted.get_unchecked(min_index));
            sorted.set(min_index, tmp);
        }
    }

    let mut percentiles = Map::new(env);
    for pct in [50u32, 90u32, 95u32, 99u32] {
        let idx = if pct >= 100 {
            sorted.len().saturating_sub(1)
        } else {
            let scaled = (sorted.len() as u32 * pct).saturating_div(100);
            (scaled.saturating_sub(1)).min(sorted.len().saturating_sub(1)) as u32
        };
        let value = if sorted.len() > idx as u32 {
            sorted.get_unchecked(idx as u32)
        } else {
            0
        };
        percentiles.set(&pct, &value);
    }
    percentiles
}
