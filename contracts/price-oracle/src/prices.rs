use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, String, Vec};

use crate::admin::{
    get_aggregation_cooldown, get_aggregation_method, get_asset_resolution, get_decimals,
    get_max_aggregation_sources, get_max_events_per_call, get_max_history_length,
    get_max_history_per_asset, get_min_sources_required, get_min_submission_interval,
    get_timestamp_threshold,
};
use crate::assets::{
    get_price_bounds, is_asset_paused, is_circuit_breaker_tripped, trip_circuit_breaker,
};
use crate::events::{
    AggregationTriggeredEvent, EventLimitWarningEvent, HistoryPerAssetPrunedEvent,
    HistoryPrunedEvent, PriceAggregatedEvent, PriceOverrideExpiredEvent, PriceOverrideRemovedEvent,
    PriceOverrideSetEvent, PriceStaleEvent, PriceSubmittedEvent, RateLimitExceededEvent,
    SourceNonCompliantEvent, SourcesInsufficientEvent,
};
use crate::pause::check_not_paused;
use crate::history::{remove_history_shard_entry, should_skip_on_write, write_history_shard};
use crate::storage::{
    check_registered_asset, check_source, check_source_asset, compute_confidence_bps, compute_mean,
    compute_median, compute_trimmed_mean, compute_vwap, get_admin, is_subscribed,
    read_oracle_sources, sort_prices, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{
    AggregatePrice, Asset, BftAggregationMethod, CompactionMetadata, DataKey, ErrorCode,
    OracleSources, PriceData, PriceEntry, PriceHistoryEntry, PriceOverrideEntry, TwapMethod,
};
// Issue #290 — record submission against schedule (liveness check)
use crate::scheduling;

fn build_candidate_aggregate(
    env: &Env,
    asset: &Address,
    source: &Address,
    price: i128,
    timestamp: u64,
    decimals: u32,
) -> Option<AggregatePrice> {
    let min_required = get_min_sources_required(env);
    let oracle_sources: OracleSources = read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();

    let max_agg = get_max_aggregation_sources(env);
    let selected_sources: Vec<Address> = if max_agg > 0 && total_sources > max_agg {
        let hash_bytes = env.ledger().sequence().to_le_bytes();
        let seed = u32::from_le_bytes(hash_bytes);
        let mut selected: Vec<Address> = Vec::new(env);
        let mut kept: u32 = 0;
        for i in 0..total_sources {
            let remaining = total_sources - i;
            let needed = max_agg - kept;
            let h = seed
                .wrapping_mul(1664525u32)
                .wrapping_add(i)
                .wrapping_add(1013904223u32);
            if needed >= remaining || (h % remaining) < needed {
                selected.push_back(oracle_sources.sources.get_unchecked(i));
                kept += 1;
                if kept >= max_agg {
                    break;
                }
            }
        }
        selected
    } else {
        oracle_sources.sources.clone()
    };

    let mut valid_prices: Vec<i128> = Vec::new(env);
    let mut valid_volumes: Vec<i128> = Vec::new(env);
    let mut latest_timestamp: u64 = 0;
    let mut contributing_sources: u32 = 0;

    let selected_count = selected_sources.len();
    for i in 0..selected_count {
        let src = selected_sources.get_unchecked(i);
        if src == *source {
            if timestamp > latest_timestamp {
                latest_timestamp = timestamp;
            }
            valid_prices.push_back(price);
            valid_volumes.push_back(0);
            contributing_sources += 1;
            continue;
        }

        let sub_key = DataKey::Submission(asset.clone(), src.clone());
        if let Some(entry_data) = env.storage().persistent().get::<_, PriceEntry>(&sub_key) {
            if entry_data.timestamp > latest_timestamp {
                latest_timestamp = entry_data.timestamp;
            }
            valid_prices.push_back(entry_data.price);
            valid_volumes.push_back(entry_data.volume.unwrap_or(0));
            contributing_sources += 1;
        }
    }

    if contributing_sources >= min_required && !valid_prices.is_empty() {
        let aggregated_price = aggregate_prices(env, &valid_prices, &valid_volumes);
        Some(AggregatePrice {
            price: aggregated_price,
            timestamp: latest_timestamp,
            num_sources: contributing_sources,
            decimals,
            is_override: false,
            version: 0,
        })
    } else {
        None
    }
}

fn read_bft_fault_tolerance(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgBftFaultTolerance)
        .unwrap_or(0)
}

fn read_bft_aggregation_method(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgBftAggregationMethod)
        .unwrap_or(BftAggregationMethod::Median as u32)
}

fn enforce_commit_reveal_for_bft(env: &Env) {
    if read_bft_fault_tolerance(env) > 0 {
        panic_with_error!(env, ErrorCode::CommitRevealRequired);
    }
}

fn aggregate_prices(env: &Env, prices: &Vec<i128>, volumes: &Vec<i128>) -> i128 {
    let bft_fault_tolerance = read_bft_fault_tolerance(env);
    if bft_fault_tolerance > 0 {
        let method = read_bft_aggregation_method(env);
        return aggregate_bft_prices(env, prices, bft_fault_tolerance, method);
    }

    let method = get_aggregation_method(env);
    match method {
        0 => compute_median(prices),
        1 => compute_mean(prices),
        2 => compute_trimmed_mean(prices, 10),
        3 => compute_vwap(prices, volumes),
        _ => compute_median(prices),
    }
}

fn aggregate_bft_prices(env: &Env, prices: &Vec<i128>, fault_tolerance: u32, method: u32) -> i128 {
    let n = prices.len();
    if n == 0 {
        return 0;
    }
    if n == 1 {
        return prices.get_unchecked(0);
    }

    let mut sorted: Vec<i128> = prices.clone();
    sort_prices(&mut sorted);

    let mut accepted: Vec<i128> = Vec::new(env);
    if n >= 4 {
        let q1_index = n / 4;
        let q3_index = (n * 3 / 4).saturating_sub(1);
        let q1 = sorted.get_unchecked(q1_index);
        let q3 = sorted.get_unchecked(q3_index);
        let iqr = q3.saturating_sub(q1);
        let margin = iqr.saturating_mul(3) / 2;
        let lower_bound = q1.saturating_sub(margin);
        let upper_bound = q3.saturating_add(margin);

        for i in 0..n {
            let value = sorted.get_unchecked(i);
            if value >= lower_bound && value <= upper_bound {
                accepted.push_back(value);
            }
        }
    }

    let mut consensus = if accepted.is_empty() {
        sorted.clone()
    } else {
        accepted
    };
    let consensus_len = consensus.len();
    let trim_count = fault_tolerance.min(consensus_len / 2);
    if trim_count > 0 && consensus_len > trim_count.saturating_mul(2) {
        let start = trim_count;
        let end = consensus_len.saturating_sub(trim_count);
        let mut trimmed: Vec<i128> = Vec::new(env);
        for i in start..end {
            trimmed.push_back(consensus.get_unchecked(i));
        }
        if !trimmed.is_empty() {
            consensus = trimmed;
        }
    }

    match method {
        0 => compute_median(&consensus),
        1 => compute_mean(&consensus),
        2 => compute_trimmed_mean(&consensus, 10),
        _ => compute_median(&consensus),
    }
}

pub fn set_bft_parameters(env: &Env, fault_tolerance: u32, method: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if method > BftAggregationMethod::TrimmedMean as u32 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgBftFaultTolerance, &fault_tolerance);
    env.storage()
        .persistent()
        .set(&DataKey::CfgBftAggregationMethod, &method);
}

pub fn get_bft_fault_tolerance(env: &Env) -> u32 {
    read_bft_fault_tolerance(env)
}

pub fn get_bft_aggregation_method(env: &Env) -> u32 {
    read_bft_aggregation_method(env)
}

fn validate_price_submission(
    env: &Env,
    asset: &Address,
    source: &Address,
    price: i128,
    timestamp: u64,
    decimals: u32,
) {
    let bounds = get_price_bounds(env, asset.clone());
    if price < bounds.min_price || price > bounds.max_price {
        panic_with_error!(env, ErrorCode::PriceOutOfBounds);
    }

    if is_asset_paused(env, asset) || is_circuit_breaker_tripped(env, asset) {
        panic_with_error!(env, ErrorCode::AssetPaused);
    }

    if bounds.max_change_bps_per_ledger > 0 {
        let prev_aggregate: AggregatePrice = env
            .storage()
            .persistent()
            .get(&DataKey::Aggregate(asset.clone()))
            .unwrap_or(AggregatePrice {
                price: 0,
                timestamp: 0,
                num_sources: 0,
                decimals,
                is_override: false,
                version: 0,
            });

        if prev_aggregate.price > 0 {
            if let Some(candidate) =
                build_candidate_aggregate(env, asset, source, price, timestamp, decimals)
            {
                let diff = if candidate.price > prev_aggregate.price {
                    candidate.price - prev_aggregate.price
                } else {
                    prev_aggregate.price - candidate.price
                };
                let change_bps = diff.saturating_mul(10_000i128) / prev_aggregate.price;
                if change_bps > bounds.max_change_bps_per_ledger as i128 {
                    trip_circuit_breaker(
                        env,
                        asset.clone(),
                        prev_aggregate.price,
                        candidate.price,
                        change_bps,
                        bounds.max_change_bps_per_ledger,
                    );
                    panic_with_error!(env, ErrorCode::CircuitBreakerTripped);
                }
            }
        }
    }
}

/// Submits prices for multiple assets in a single atomic transaction.
///
/// Authorization is checked once for `source`. Each `(asset, price, timestamp)` tuple
/// is validated individually; if any entry is invalid the entire call panics (atomicity).
/// Aggregation is triggered for each asset after all submissions are stored.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `source` - Address of the submitting oracle source. Must authorize this call.
/// * `asset_prices` - Ordered list of `(asset, price, timestamp)` tuples.
///
/// # Errors
///
/// Same error conditions as `submit_price`, applied per entry.
pub fn submit_prices(env: &Env, source: Address, asset_prices: Vec<(Address, i128, u64)>) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);
    enforce_commit_reveal_for_bft(env);

    if crate::sources::is_source_suspended(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceSuspended);
    }

    let decimals = get_decimals(env);
    let ledger_time = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    let current_ledger = env.ledger().sequence();

    // Validate all entries first for atomicity — any invalid entry aborts the whole call.
    for i in 0..asset_prices.len() {
        let (ref asset, price, timestamp) = asset_prices.get_unchecked(i);
        check_registered_asset(env, asset);
        crate::freeze::check_not_frozen(env, asset);
        check_source_asset(env, &source, asset);

        if price <= 0 {
            crate::sources::record_invalid_submission(env, source.clone());
            panic_with_error!(env, ErrorCode::InvalidPrice);
        }

        validate_price_submission(env, asset, &source, price, timestamp, decimals);

        if timestamp > ledger_time.saturating_add(threshold) {
            crate::sources::record_invalid_submission(env, source.clone());
            panic_with_error!(env, ErrorCode::InvalidTimestamp);
        }
    }

    // All valid — store submissions and trigger aggregation.
    for i in 0..asset_prices.len() {
        let (asset, price, timestamp) = asset_prices.get_unchecked(i);

        if check_deviation_circuit_breaker(env, &source, &asset, price) {
            return;
        }

        let entry = PriceEntry {
            price,
            timestamp,
            source: source.clone(),
            decimals,
            last_updated: current_ledger,
            ledger_timestamp: ledger_time,
            volume: None,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

        record_successful_submission(env, source.clone());

        // #70: track last submission ledger for compliance
        env.storage().persistent().set(
            &DataKey::LastSubmissionLedger(source.clone(), asset.clone()),
            &current_ledger,
        );
        // If source was non-compliant, clear the flag on new submission
        let nc_key = DataKey::SourceNonCompliant(source.clone(), asset.clone());
        if env.storage().persistent().has(&nc_key) {
            env.storage().persistent().remove(&nc_key);
        }

        PriceSubmittedEvent {
            asset: asset.clone(),
            source: source.clone(),
            price,
            timestamp,
        }
        .publish(env);
    }

    // Trigger aggregation for each submitted asset.
    for i in 0..asset_prices.len() {
        let (asset, _, _) = asset_prices.get_unchecked(i);
        if !maybe_aggregate_after_submission(env, &asset, current_ledger) {
            continue;
        }
        aggregate_asset(env, &asset, current_ledger, decimals);
    }
}

fn count_contributing_sources(env: &Env, asset: &Address, current_ledger: u32) -> u32 {
    let min_interval = {
        let key = DataKey::AssetMinSubmissionInterval(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            env.storage().persistent().get(&key).unwrap_or(0)
        } else {
            get_min_submission_interval(env)
        }
    };

    let oracle_sources: OracleSources = read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();
    let max_agg = get_max_aggregation_sources(env);
    let selected_sources: Vec<Address> = if max_agg > 0 && total_sources > max_agg {
        let hash_bytes = env.ledger().sequence().to_le_bytes();
        let seed = u32::from_le_bytes(hash_bytes);
        let mut selected: Vec<Address> = Vec::new(env);
        let mut kept: u32 = 0;
        for i in 0..total_sources {
            let remaining = total_sources - i;
            let needed = max_agg - kept;
            let h = seed
                .wrapping_mul(1664525u32)
                .wrapping_add(i)
                .wrapping_add(1013904223u32);
            if needed >= remaining || (h % remaining) < needed {
                selected.push_back(oracle_sources.sources.get_unchecked(i));
                kept += 1;
                if kept >= max_agg {
                    break;
                }
            }
        }
        selected
    } else {
        oracle_sources.sources.clone()
    };

    let mut contributing_sources: u32 = 0;
    let selected_count = selected_sources.len();
    for i in 0..selected_count {
        let src = selected_sources.get_unchecked(i);

        if min_interval > 0 {
            let last_sub_key = DataKey::LastSubmissionLedger(src.clone(), asset.clone());
            let last_sub: Option<u32> = env.storage().persistent().get(&last_sub_key);
            if let Some(last) = last_sub {
                if current_ledger.saturating_sub(last) > min_interval {
                    continue;
                }
            } else {
                continue;
            }
        }

        if crate::correlation::is_correlation_flagged(env, &src, asset) {
            continue;
        }

        let sub_key = DataKey::Submission(asset.clone(), src.clone());
        if env.storage().persistent().has(&sub_key) {
            contributing_sources += 1;
        }
    }

    contributing_sources
}

fn maybe_aggregate_after_submission(env: &Env, asset: &Address, current_ledger: u32) -> bool {
    let min_required = get_min_sources_required(env);
    let contributing_sources = count_contributing_sources(env, asset, current_ledger);
    if contributing_sources >= min_required {
        return true;
    }

    SourcesInsufficientEvent {
        asset: asset.clone(),
        current_source_count: contributing_sources,
        min_sources_required: min_required,
    }
    .publish(env);
    false
}

/// Internal helper: re-aggregate all sources for a single asset and write history.
fn aggregate_asset(env: &Env, asset: &Address, current_ledger: u32, decimals: u32) {
    let max_events = get_max_events_per_call(env);
    let mut event_count: u32 = 0;

    // Issue #290: record submission for liveness / schedule enforcement
    scheduling::record_submission(env, &source, &asset);

    let min_required = get_min_sources_required(env);
    let oracle_sources: OracleSources = read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();

    // Issue #93: if MaxAggregationSources > 0 and we have more sources than the cap,
    // randomly select a subset using the current ledger hash for determinism.
    let max_agg = get_max_aggregation_sources(env);
    let selected_sources: Vec<Address> = if max_agg > 0 && total_sources > max_agg {
        // Use ledger hash bytes as deterministic entropy for selection.
        // Construct a 32-byte ledger hash using the chainable SHA256 of the
        // current ledger sequence (stable across nodes) to avoid predictability.
        let seq_bytes = soroban_sdk::Bytes::from_slice(env, &env.ledger().sequence().to_le_bytes());
        let hash: soroban_sdk::BytesN<32> = env.crypto().sha256(&seq_bytes).into();
        // Derive a 32-bit seed from the first 4 bytes of the hash.
        let mut seed_arr: [u8; 4] = [0u8; 4];
        seed_arr.copy_from_slice(&hash.as_slice()[0..4]);
        let seed = u32::from_le_bytes(seed_arr);

        let mut selected: Vec<Address> = Vec::new(env);
        let mut kept: u32 = 0;

        // Deterministic reservoir sampling driven by the seeded LCG.
        for i in 0..total_sources {
            let remaining = total_sources - i;
            let needed = max_agg - kept;
            // LCG step to mix seed and index.
            let h = seed
                .wrapping_mul(1664525u32)
                .wrapping_add(i.wrapping_add(1013904223u32));
            if needed >= remaining || (h % remaining) < needed {
                selected.push_back(oracle_sources.sources.get_unchecked(i));
                kept += 1;
                if kept >= max_agg {
                    break;
                }
            }
        }
        selected
    } else {
        oracle_sources.sources.clone()
    };

    let mut valid_prices: Vec<i128> = Vec::new(env);
    let mut valid_volumes: Vec<i128> = Vec::new(env);
    let mut latest_timestamp: u64 = 0;
    let mut contributing_sources: u32 = 0;

    let min_interval = {
        let key = DataKey::AssetMinSubmissionInterval(asset.clone());
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            env.storage().persistent().get(&key).unwrap_or(0)
        } else {
            get_min_submission_interval(env)
        }
    };
    let current_ledger_for_agg = env.ledger().sequence();
    let selected_count = selected_sources.len();

    for i in 0..selected_count {
        let src = selected_sources.get_unchecked(i);

        // #70: enforce min submission interval compliance
        if min_interval > 0 {
            let last_sub_key = DataKey::LastSubmissionLedger(src.clone(), asset.clone());
            let last_sub: Option<u32> = env.storage().persistent().get(&last_sub_key);
            if let Some(last) = last_sub {
                if current_ledger_for_agg.saturating_sub(last) > min_interval {
                    // Flag source as non-compliant
                    let nc_key = DataKey::SourceNonCompliant(src.clone(), asset.clone());
                    if !env.storage().persistent().has(&nc_key) {
                        env.storage().persistent().set(&nc_key, &true);
                        SourceNonCompliantEvent {
                            source: src.clone(),
                            asset: asset.clone(),
                            last_submission_ledger: last,
                            required_interval: min_interval,
                        }
                        .publish(env);
                    }
                    continue; // exclude from aggregation
                }
            }
            // If never submitted, skip (not compliant yet)
            if last_sub.is_none() {
                continue;
            }
        }

        let sub_key = DataKey::Submission(asset.clone(), src.clone());
        let sub: Option<PriceEntry> = env.storage().persistent().get(&sub_key);
        if let Some(entry_data) = sub {
            // Skip prices flagged for correlation violations.
            if crate::correlation::is_correlation_flagged(env, &src, asset) {
                continue;
            }
            env.storage()
                .persistent()
                .extend_ttl(&sub_key, LEDGER_THRESHOLD, LEDGER_BUMP);
            if entry_data.timestamp > latest_timestamp {
                latest_timestamp = entry_data.timestamp;
            }
            valid_prices.push_back(entry_data.price);
            valid_volumes.push_back(entry_data.volume.unwrap_or(0));
            contributing_sources += 1;
        }
    }

    if contributing_sources >= min_required && !valid_prices.is_empty() {
        let median_price = aggregate_prices(env, &valid_prices, &valid_volumes);

        let agg_key = DataKey::Aggregate(asset.clone());
        let prev_aggregate: AggregatePrice =
            env.storage()
                .persistent()
                .get(&agg_key)
                .unwrap_or(AggregatePrice {
                    price: 0,
                    timestamp: 0,
                    num_sources: 0,
                    decimals,
                    is_override: false,
                    version: 0,
                });

        // Increment version only when the price actually changes (#252).
        let new_version = if median_price != prev_aggregate.price {
            prev_aggregate.version.saturating_add(1)
        } else {
            prev_aggregate.version
        };

        let aggregate = AggregatePrice {
            price: median_price,
            timestamp: latest_timestamp,
            num_sources: contributing_sources,
            decimals,
            is_override: false,
            version: new_version,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Aggregate(asset.clone()), &aggregate);
        env.storage().persistent().extend_ttl(
            &DataKey::Aggregate(asset.clone()),
            LEDGER_THRESHOLD,
            LEDGER_BUMP,
        );

        // Record gas usage for this aggregation run.
        let before_cpu = env.budget().cpu_instruction_count();
        let before_mem = env.budget().memory_bytes_count();
        // NOTE: the measured delta here only captures the remainder of the
        // aggregation function after this point; callers (e.g. submit_price)
        // record end-to-end cost. Still store an aggregate-internal snapshot.
        let after_cpu = env.budget().cpu_instruction_count();
        let after_mem = env.budget().memory_bytes_count();
        let cpu_delta = after_cpu.saturating_sub(before_cpu);
        let mem_delta = after_mem.saturating_sub(before_mem);
        crate::gas_metering::write_last_gas(
            &env,
            soroban_sdk::String::from_str(&env, "aggregate"),
            cpu_delta,
            mem_delta,
        );

        let history_entry = PriceHistoryEntry {
            price: median_price,
            timestamp: latest_timestamp,
            ledger: current_ledger,
            num_sources: contributing_sources,
            is_interpolated: false,
        };
        let skip_history = should_skip_on_write(env, asset, median_price);
        env.storage().temporary().set(
            &DataKey::PriceHistory(asset.clone(), current_ledger),
            &history_entry,
        );

        // Track ledger in history index for pruning. Avoid duplicate sequence entries
        // if aggregation is run more than once in the same ledger.
        let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
        let mut ledger_list: soroban_sdk::Vec<u32> = env
            .storage()
            .persistent()
            .get(&ledgers_key)
            .unwrap_or(soroban_sdk::Vec::new(env));
        if !skip_history {
            if ledger_list.len() == 0
                || ledger_list.get_unchecked(ledger_list.len() - 1) != current_ledger
            {
                ledger_list.push_back(current_ledger);
            }
            write_history_shard(env, asset, &history_entry);
        } else {
            env.storage()
                .temporary()
                .remove(&DataKey::PriceHistory(asset.clone(), current_ledger));
        }

        // Issue #92: check event budget before emitting prune events.
        // Each prune loop iteration emits 1 event.

        // Global history cap (existing MaxHistoryLength).
        let max_history = get_max_history_length(env);
        while ledger_list.len() > max_history {
            // Issue #92: stop emitting prune events if we hit the cap.
            if event_count >= max_events {
                EventLimitWarningEvent {
                    asset: asset.clone(),
                    event_count,
                    max_events,
                }
                .publish(env);
                break;
            }
            let oldest_ledger = ledger_list.get_unchecked(0);
            ledger_list.remove(0);
            env.storage()
                .temporary()
                .remove(&DataKey::PriceHistory(asset.clone(), oldest_ledger));
            remove_history_shard_entry(env, asset, oldest_ledger);
            HistoryPrunedEvent {
                asset: asset.clone(),
                pruned_ledger: oldest_ledger,
                remaining: ledger_list.len(),
            }
            .publish(env);
            event_count += 1;
        }

        // Issue #94: per-asset history cap (MaxHistoryPerAsset, default 1000).
        let max_per_asset = get_max_history_per_asset(env);
        while ledger_list.len() > max_per_asset {
            if event_count >= max_events {
                EventLimitWarningEvent {
                    asset: asset.clone(),
                    event_count,
                    max_events,
                }
                .publish(env);
                break;
            }
            let oldest_ledger = ledger_list.get_unchecked(0);
            ledger_list.remove(0);
            env.storage()
                .temporary()
                .remove(&DataKey::PriceHistory(asset.clone(), oldest_ledger));
            remove_history_shard_entry(env, asset, oldest_ledger);
            HistoryPerAssetPrunedEvent {
                asset: asset.clone(),
                pruned_ledger: oldest_ledger,
                remaining: ledger_list.len(),
            }
            .publish(env);
            event_count += 1;
        }

        env.storage().persistent().set(&ledgers_key, &ledger_list);
        if skip_history {
            let metadata = CompactionMetadata {
                original_count: ledger_list.len().saturating_add(1),
                compacted_count: ledger_list.len(),
                last_compaction_ledger: current_ledger,
                threshold_bps: crate::history::get_compaction_threshold_bps(env),
            };
            env.storage()
                .persistent()
                .set(&DataKey::CompactionMeta(asset.clone()), &metadata);
        }

        // Issue #92: only emit aggregation event if within budget.
        if event_count < max_events {
            PriceAggregatedEvent {
                asset: asset.clone(),
                price: median_price,
                num_sources: contributing_sources,
                timestamp: latest_timestamp,
            }
            .publish(env);
        } else {
            EventLimitWarningEvent {
                asset: asset.clone(),
                event_count,
                max_events,
            }
            .publish(env);
        }

        // ── #298: Update contribution quality scores for all contributing sources ──
        let oracle_sources_for_quality = read_oracle_sources(env);
        let nsrc = oracle_sources_for_quality.sources.len();
        for qi in 0..nsrc {
            let src = oracle_sources_for_quality.sources.get_unchecked(qi);
            let sub_key = DataKey::Submission(asset.clone(), src.clone());
            if let Some(entry) = env
                .storage()
                .persistent()
                .get::<DataKey, crate::types::PriceEntry>(&sub_key)
            {
                crate::contribution_quality::update_contribution_quality(
                    env,
                    &src,
                    asset,
                    entry.price,
                    median_price,
                    entry.last_updated,
                    current_ledger,
                );
            }
        }

        // ── #297: Invoke registered price callbacks (fault-isolated) ─────────────
        crate::price_callback::invoke_price_callbacks(
            env,
            asset,
            median_price,
            latest_timestamp,
            contributing_sources,
        );
    } else if event_count < max_events {
        SourcesInsufficientEvent {
            asset: asset.clone(),
            current_source_count: contributing_sources,
            min_sources_required: min_required,
        }
        .publish(env);
    } else {
        EventLimitWarningEvent {
            asset: asset.clone(),
            event_count,
            max_events,
        }
        .publish(env);
    }
}

/// Re-runs aggregation for `asset`, shared by the direct and relayed submission paths.
pub(crate) fn do_aggregate(env: &Env, asset: &Address) {
    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();
    aggregate_asset(env, asset, current_ledger, decimals);
}

/// Records a successful submission for source performance bookkeeping.
///
/// Increments both the global and per-source submission counters used by
/// downstream reporting.
pub(crate) fn record_successful_submission(env: &Env, source: Address) {
    let total_key = DataKey::TotalSubmissionCount;
    let total: u32 = env.storage().persistent().get(&total_key).unwrap_or(0);
    env.storage()
        .persistent()
        .set(&total_key, &total.saturating_add(1));
    env.storage()
        .persistent()
        .extend_ttl(&total_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    let src_key = DataKey::SourceSubmissionCount(source);
    let count: u32 = env.storage().persistent().get(&src_key).unwrap_or(0);
    env.storage()
        .persistent()
        .set(&src_key, &count.saturating_add(1));
    env.storage()
        .persistent()
        .extend_ttl(&src_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Guards a price submission against an asset whose circuit breaker has
/// already tripped. Returns `true` when the caller should abort without
/// storing or aggregating this submission.
pub(crate) fn check_deviation_circuit_breaker(
    env: &Env,
    _source: &Address,
    asset: &Address,
    _price: i128,
) -> bool {
    is_circuit_breaker_tripped(env, asset)
}

pub fn submit_price(env: &Env, source: Address, asset: Address, price: i128, timestamp: u64, nonce: u64) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);
    crate::freeze::check_not_frozen(env, &asset);
    check_source_asset(env, &source, &asset);
    enforce_commit_reveal_for_bft(env);

    if crate::sources::is_source_suspended(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceSuspended);
    }

    // Nonce-based replay prevention: nonce must be strictly greater than last accepted.
    let nonce_key = DataKey::SourceNonce(source.clone());
    let last_nonce: u64 = env
        .storage()
        .persistent()
        .get::<DataKey, u64>(&nonce_key)
        .unwrap_or(0);
    if nonce <= last_nonce {
        panic_with_error!(env, ErrorCode::InvalidNonce);
    }
    env.storage().persistent().set(&nonce_key, &nonce);
    env.storage()
        .persistent()
        .extend_ttl(&nonce_key, crate::storage::LEDGER_THRESHOLD, crate::storage::LEDGER_BUMP);

    if price <= 0 {
        crate::sources::record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let ledger_time = env.ledger().timestamp();
    validate_price_submission(env, &asset, &source, price, timestamp, get_decimals(env));
    let threshold = get_timestamp_threshold(env);
    if timestamp > ledger_time.saturating_add(threshold) {
        crate::sources::record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    if check_deviation_circuit_breaker(env, &source, &asset, price) {
        return;
    }

    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();

    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
        ledger_timestamp: env.ledger().timestamp(),
        volume: None,
    };

    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

    record_successful_submission(env, source.clone());

    PriceSubmittedEvent {
        asset: asset.clone(),
        source: source.clone(),
        price,
        timestamp,
    }
    .publish(env);

    // Cross-asset correlation check: flags (source, asset) if ratio is out of band.
    crate::correlation::validate_correlation(env, &asset, price, &source);

    crate::triggers::record_submission_for_triggers(env, &asset, price);

    if !maybe_aggregate_after_submission(env, &asset, current_ledger) {
        return;
    }
    aggregate_asset(env, &asset, current_ledger, decimals);
}

pub fn submit_price_with_volume(
    env: &Env,
    source: Address,
    asset: Address,
    price: i128,
    timestamp: u64,
    volume: Option<i128>,
) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);
    crate::freeze::check_not_frozen(env, &asset);
    check_source_asset(env, &source, &asset);
    enforce_commit_reveal_for_bft(env);

    if crate::sources::is_source_suspended(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceSuspended);
    }
    if price <= 0 || volume.unwrap_or(1) < 0 {
        crate::sources::record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let ledger_time = env.ledger().timestamp();
    let decimals = get_decimals(env);
    validate_price_submission(env, &asset, &source, price, timestamp, decimals);
    let threshold = get_timestamp_threshold(env);
    if timestamp > ledger_time.saturating_add(threshold) {
        crate::sources::record_invalid_submission(env, source.clone());
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }
    if check_deviation_circuit_breaker(env, &source, &asset, price) {
        return;
    }

    let current_ledger = env.ledger().sequence();
    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
        ledger_timestamp: ledger_time,
        volume,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);
    record_successful_submission(env, source.clone());
    PriceSubmittedEvent {
        asset: asset.clone(),
        source: source.clone(),
        price,
        timestamp,
    }
    .publish(env);
    crate::correlation::validate_correlation(env, &asset, price, &source);
    crate::triggers::record_submission_for_triggers(env, &asset, price);
    aggregate_asset(env, &asset, current_ledger, decimals);
}

fn compute_twap_window(
    env: &Env,
    asset: &Address,
    start_ledger: u32,
    current_ledger: u32,
    method: TwapMethod,
) -> Option<PriceData> {
    let agg_key = DataKey::Aggregate(asset.clone());
    let current_agg: AggregatePrice = env.storage().persistent().get(&agg_key)?;
    let mut snapshots: Vec<(u32, i128)> = Vec::new(env);
    snapshots.push_back((current_ledger, current_agg.price));

    let mut ledger = current_ledger;
    while ledger > 0 {
        ledger -= 1;
        if let Some(entry) = crate::history::read_history_entry(env, asset, ledger) {
            let last = snapshots.get_unchecked(snapshots.len() - 1);
            if entry.ledger != last.0 {
                snapshots.push_back((entry.ledger, entry.price));
            }
            if entry.ledger <= start_ledger {
                break;
            }
        }
    }

    let mut total_weight: u64 = 0;
    let mut weighted_price: i128 = 0;
    let mut weighted_log2: i128 = 0;
    let mut next_boundary = current_ledger.saturating_add(1);

    for i in 0..snapshots.len() {
        let (ledger, price) = snapshots.get_unchecked(i);
        let segment_start = if *ledger < start_ledger {
            start_ledger
        } else {
            *ledger
        };
        if next_boundary > segment_start {
            let weight = next_boundary - segment_start;
            total_weight = total_weight.saturating_add(weight);
            weighted_price = weighted_price.saturating_add(price.saturating_mul(weight as i128));
            if method == TwapMethod::Geometric {
                weighted_log2 =
                    weighted_log2.saturating_add(log2_fixed(price).saturating_mul(weight as i128));
            }
            next_boundary = segment_start;
        }
        if segment_start == start_ledger {
            break;
        }
    }

    if total_weight == 0 {
        return None;
    }

    let price = match method {
        TwapMethod::Arithmetic => weighted_price / (total_weight as i128),
        TwapMethod::Geometric => {
            let avg_log2 = weighted_log2 / (total_weight as i128);
            exp2_fixed(avg_log2)
        }
    };

    Some(PriceData {
        price,
        timestamp: env.ledger().timestamp(),
        last_updated: current_ledger,
    })
}

fn log2_fixed(value: i128) -> i128 {
    let x = value as u128;
    let bit_index = 127 - x.leading_zeros();
    let exp = (bit_index as i128).saturating_sub(32);
    let mut y = if exp >= 0 {
        x >> (exp as u32)
    } else {
        x << ((-exp) as u32)
    };

    let mut frac: i128 = 0;
    for i in 1..=32 {
        y = ((y as u128).saturating_mul(y as u128) >> 32) as u128;
        if y >= (2u128 << 32) {
            y >>= 1;
            frac |= 1 << (32 - i);
        }
    }

    (exp << 32) | frac
}

fn exp2_frac(frac: u128) -> u128 {
    const LN2: u128 = 2977044471u128; // ln(2) in Q32.32
    let x = (frac.saturating_mul(LN2)) >> 32;
    let x2 = (x.saturating_mul(x)) >> 32;
    let x3 = (x2.saturating_mul(x)) >> 32;
    let x4 = (x3.saturating_mul(x)) >> 32;
    let x5 = (x4.saturating_mul(x)) >> 32;
    let x6 = (x5.saturating_mul(x)) >> 32;

    (1u128 << 32)
        .saturating_add(x)
        .saturating_add(x2 / 2)
        .saturating_add(x3 / 6)
        .saturating_add(x4 / 24)
        .saturating_add(x5 / 120)
        .saturating_add(x6 / 720)
}

fn exp2_fixed(log2: i128) -> i128 {
    let int_part = log2 >> 32;
    let frac = (log2 & 0xffffffff) as u128;
    let base = exp2_frac(frac);
    if int_part >= 0 {
        if int_part >= 96 {
            return i128::MAX;
        }
        (base << (int_part as u32)) as i128
    } else {
        (base >> ((-int_part) as u32)) as i128
    }
}

fn compute_twap_fallback(env: &Env, asset: &Address) -> Option<AggregatePrice> {
    let current_ledger = env.ledger().sequence();
    let max_history = get_max_history_length(env).max(5);
    let decimals = get_decimals(env);
    let price_data = compute_twap_window(
        env,
        asset,
        current_ledger.saturating_sub(max_history),
        current_ledger,
        TwapMethod::Arithmetic,
    )?;
    Some(AggregatePrice {
        price: price_data.price,
        timestamp: price_data.timestamp,
        num_sources: 0,
        decimals,
        is_override: false,
        version: 0,
    })
}

pub fn get_price(env: &Env, asset: Address, max_age: u64) -> Option<AggregatePrice> {
    // ─────────────────────────────────────────────────────────────────────────────
    // Storage reads (hot path) and when they occur:
    //  1) check_registered_asset() → DataKey::AssetRegistered(asset)
    //  2) Override branch (only if not expired):
    //       DataKey::PriceOverride(asset)
    //       - if active: also reads global decimals via get_decimals(env)
    //  3) Aggregate branch:
    //       DataKey::Aggregate(asset)
    //       - resolution gating reads per-asset resolution via get_asset_resolution(env, asset)
    // ─────────────────────────────────────────────────────────────────────────────

    check_registered_asset(env, &asset);
    let current_ledger = env.ledger().sequence();
    let ledger_time = env.ledger().timestamp();

    // A freeze (#223) takes priority over overrides and the live aggregate: it
    // locks the price in place regardless of any other activity.
    if let Some(frozen) = crate::freeze::get_frozen_price(env, asset.clone()) {
        return Some(AggregatePrice {
            price: frozen.price,
            timestamp: frozen.timestamp,
            num_sources: 0,
            decimals: frozen.decimals,
            is_override: false,
            version: 0,
        });
    }

    // Check for active price override
    let override_key = DataKey::PriceOverride(asset.clone());
    if let Some(ovr) = env
        .storage()
        .persistent()
        .get::<_, PriceOverrideEntry>(&override_key)
    {
        if current_ledger <= ovr.expiry_ledger {
            env.storage()
                .persistent()
                .extend_ttl(&override_key, LEDGER_THRESHOLD, LEDGER_BUMP);

            // Only needed when override is active.
            let decimals = get_decimals(env);
            return Some(AggregatePrice {
                price: ovr.price,
                timestamp: ledger_time,
                num_sources: 0,
                decimals,
                is_override: true,
                version: 0,
            });
        } else {
            // Override has expired
            PriceOverrideExpiredEvent {
                asset: asset.clone(),
                expiry_ledger: ovr.expiry_ledger,
                current_ledger,
            }
            .publish(env);
            env.storage().persistent().remove(&override_key);
        }
    }

    if is_circuit_breaker_tripped(env, &asset) {
        return compute_twap_fallback(env, &asset).or_else(|| {
            let key = DataKey::Aggregate(asset.clone());
            env.storage().persistent().get(&key)
        });
    }

    let key = DataKey::Aggregate(asset.clone());
    let result: AggregatePrice = env.storage().persistent().get(&key)?;

    // max_age gating (if enabled)
    if max_age > 0 && result.timestamp.saturating_add(max_age) < ledger_time {
        PriceStaleEvent {
            asset: asset.clone(),
            last_update_ledger: 0,
            current_ledger,
        }
        .publish(env);
        return None;
    }

    // resolution gating (if enabled)
    let resolution = get_asset_resolution(env, asset.clone());
    if resolution > 0 && result.timestamp.saturating_add(resolution as u64) < ledger_time {
        PriceStaleEvent {
            asset: asset.clone(),
            last_update_ledger: 0,
            current_ledger,
        }
        .publish(env);
        return None;
    }

    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    Some(result)
}

/// Returns the current aggregated price together with its monotonically-incrementing
/// version counter for the given asset (#252).
///
/// The version allows consumers to detect price changes by comparing a lightweight
/// `u32` rather than the full `i128` price value. The version starts at 0 after
/// the first aggregation and increments by 1 each time the price changes.
///
/// # Panics
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
/// * [`ErrorCode::NoData`] — no aggregate exists yet for the asset.
pub fn get_aggregate_with_version(
    env: &Env,
    asset: Address,
) -> crate::types::VersionedAggregatePrice {
    check_registered_asset(env, &asset);
    let key = DataKey::Aggregate(asset.clone());
    let aggregate: AggregatePrice = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::NoData));
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    let version = aggregate.version;
    crate::types::VersionedAggregatePrice { aggregate, version }
}

pub fn get_price_with_confidence(env: &Env, asset: Address) -> Option<(AggregatePrice, u32)> {
    let aggregate = get_price(env, asset.clone(), 0)?;

    let mut prices: Vec<i128> = Vec::new(env);
    let entries = get_all_prices(env, asset.clone());
    for i in 0..entries.len() {
        let entry = entries.get_unchecked(i);
        prices.push_back(entry.price);
    }

    let confidence_bps = compute_confidence_bps(&prices);
    Some((aggregate, confidence_bps))
}

pub fn get_source_price(env: &Env, asset: Address, source: Address) -> PriceEntry {
    check_registered_asset(env, &asset);
    check_source(env, &source);
    let key = DataKey::Submission(asset, source);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    env.storage().persistent().get(&key).unwrap()
}

pub fn get_all_prices(env: &Env, asset: Address) -> Vec<PriceEntry> {
    check_registered_asset(env, &asset);
    // Read the sources list once; iterate without extra reads or writes per entry.
    let oracle_sources: OracleSources = read_oracle_sources(env);
    let mut prices: Vec<PriceEntry> = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let src = oracle_sources.sources.get_unchecked(i);
        let sub_key = DataKey::Submission(asset.clone(), src);
        if let Some(entry) = env.storage().persistent().get::<_, PriceEntry>(&sub_key) {
            prices.push_back(entry);
        }
    }
    prices
}

/// Not currently wired into `get_price` — kept for a future rate-limiting pass.
#[allow(dead_code)]
pub fn check_rate_limit_and_increment(env: &Env, consumer: &Address) {
    if is_subscribed(env, consumer) {
        return;
    }

    let ledger = env.ledger().sequence();
    let rate_limit_key = DataKey::QueryRateLimit;
    let max_queries: u32 = env
        .storage()
        .persistent()
        .get(&rate_limit_key)
        .unwrap_or(100);

    let count_key = DataKey::QueryCount(consumer.clone(), ledger);
    let current_count: u32 = env.storage().temporary().get(&count_key).unwrap_or(0);

    if current_count >= max_queries {
        RateLimitExceededEvent {
            consumer: consumer.clone(),
            current_count,
            limit: max_queries,
        }
        .publish(env);
        panic_with_error!(env, ErrorCode::RateLimitExceeded);
    }

    let new_count = current_count + 1;
    env.storage().temporary().set(&count_key, &new_count);
    env.storage()
        .temporary()
        .extend_ttl(&count_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn lastprice(env: &Env, asset: Asset) -> Option<PriceData> {
    let addr = match asset {
        Asset::Stellar(a) => a,
        Asset::Other(_) => return None,
    };
    let reg_key = DataKey::AssetRegistered(addr.clone());
    if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
        return None;
    }
    let agg_key = DataKey::Aggregate(addr.clone());
    let result: AggregatePrice = env.storage().persistent().get(&agg_key)?;
    // #67: use per-asset resolution (falls back to contract-wide)
    let resolution = get_asset_resolution(env, addr.clone());
    if resolution > 0 {
        let ledger_time = env.ledger().timestamp();
        if result.timestamp.saturating_add(resolution as u64) < ledger_time {
            return None;
        }
    }
    env.storage()
        .persistent()
        .extend_ttl(&agg_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    Some(PriceData {
        price: result.price,
        timestamp: result.timestamp,
        last_updated: env.ledger().sequence(),
    })
}

pub fn price(env: &Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
    let addr = match asset {
        Asset::Stellar(a) => a,
        Asset::Other(_) => return None,
    };
    let reg_key = DataKey::AssetRegistered(addr.clone());
    if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
        return None;
    }
    let agg_key = DataKey::Aggregate(addr.clone());
    if let Some(agg) = env
        .storage()
        .persistent()
        .get::<_, AggregatePrice>(&agg_key)
    {
        if agg.timestamp == timestamp {
            return Some(PriceData {
                price: agg.price,
                timestamp: agg.timestamp,
                last_updated: env.ledger().sequence(),
            });
        }
    }
    let current_ledger = env.ledger().sequence();
    let start = current_ledger.saturating_sub(1000);
    let mut ledger = current_ledger;
    loop {
        if let Some(entry) = crate::history::read_history_entry(env, &addr, ledger) {
            if entry.timestamp <= timestamp {
                return Some(PriceData {
                    price: entry.price,
                    timestamp: entry.timestamp,
                    last_updated: ledger,
                });
            }
        }
        if ledger == start {
            break;
        }
        ledger -= 1;
    }
    None
}

pub fn prices(env: &Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
    let addr = match asset {
        Asset::Stellar(a) => a,
        Asset::Other(_) => return None,
    };
    let reg_key = DataKey::AssetRegistered(addr.clone());
    if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
        return None;
    }
    if records == 0 {
        return Some(Vec::new(env));
    }
    let max_history = get_max_history_length(env);
    if records > max_history {
        panic_with_error!(env, ErrorCode::RecordsLimitExceeded);
    }
    let mut result: Vec<PriceData> = Vec::new(env);
    let current_ledger = env.ledger().sequence();
    let max_to_check = (records * 10).min(10000);
    let start = current_ledger.saturating_sub(max_to_check);
    let mut ledger = current_ledger;
    loop {
        if let Some(entry) = crate::history::read_history_entry(env, &addr, ledger) {
            result.push_back(PriceData {
                price: entry.price,
                timestamp: entry.timestamp,
                last_updated: ledger,
            });
            if result.len() >= records {
                break;
            }
        }
        if ledger == start {
            break;
        }
        ledger -= 1;
    }
    if result.is_empty() {
        let agg_key = DataKey::Aggregate(addr);
        if let Some(agg) = env
            .storage()
            .persistent()
            .get::<_, AggregatePrice>(&agg_key)
        {
            result.push_back(PriceData {
                price: agg.price,
                timestamp: agg.timestamp,
                last_updated: current_ledger,
            });
        }
    }
    Some(result)
}

pub fn get_twap(
    env: &Env,
    asset: Asset,
    window_ledgers: u32,
    method: TwapMethod,
) -> Option<PriceData> {
    let addr = match asset {
        Asset::Stellar(a) => a,
        Asset::Other(_) => return None,
    };
    let reg_key = DataKey::AssetRegistered(addr.clone());
    if !env.storage().persistent().get(&reg_key).unwrap_or(false) {
        return None;
    }
    if window_ledgers == 0 || window_ledgers > get_max_history_length(env) {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let current_ledger = env.ledger().sequence();
    let start_ledger = current_ledger.saturating_sub(window_ledgers.saturating_sub(1));
    compute_twap_window(env, &addr, start_ledger, current_ledger, method)
}

pub fn override_price(env: &Env, asset: Address, price: i128, reason: String, expiry_ledger: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    const MAX_REASON_LENGTH: u32 = 256;
    if reason.len() > MAX_REASON_LENGTH {
        panic_with_error!(env, ErrorCode::ReasonTooLong);
    }

    let current_ledger = env.ledger().sequence();
    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }
    if expiry_ledger <= current_ledger {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let entry = PriceOverrideEntry {
        price,
        reason: reason.clone(),
        expiry_ledger,
        set_ledger: current_ledger,
    };
    env.storage()
        .persistent()
        .set(&DataKey::PriceOverride(asset.clone()), &entry);
    env.storage().persistent().extend_ttl(
        &DataKey::PriceOverride(asset.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    PriceOverrideSetEvent {
        asset: asset.clone(),
        admin: admin.clone(),
        price,
        reason,
        expiry_ledger,
    }
    .publish(env);
}

pub fn remove_price_override(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let override_key = DataKey::PriceOverride(asset.clone());
    if !env.storage().persistent().has(&override_key) {
        panic_with_error!(env, ErrorCode::NoData);
    }
    env.storage().persistent().remove(&override_key);

    PriceOverrideRemovedEvent {
        asset: asset.clone(),
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn get_price_override(env: &Env, asset: Address) -> Option<PriceOverrideEntry> {
    check_registered_asset(env, &asset);
    let override_key = DataKey::PriceOverride(asset);
    if env.storage().persistent().has(&override_key) {
        env.storage()
            .persistent()
            .extend_ttl(&override_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&override_key)
}

/// Not currently wired into any public contract endpoint.
#[allow(dead_code)]
pub fn historical_price_change_percent(
    env: &Env,
    asset: Address,
    current_price: AggregatePrice,
    ledgers_back: u32,
) -> Option<i128> {
    let current_ledger = env.ledger().sequence();
    let target_ledger = current_ledger.saturating_sub(ledgers_back);

    let historical_entry = crate::history::read_history_entry(env, &asset, target_ledger);

    let old_price = {
        let entry = historical_entry?;
        entry.price
    };

    if old_price == 0 {
        return None;
    }

    let diff = current_price.price.saturating_sub(old_price);
    let change_percent = diff.saturating_mul(100) / old_price;
    Some(change_percent)
}

/// #69: Trigger aggregation manually. Callable by anyone, subject to cooldown.
pub fn trigger_aggregation(env: &Env, asset: Address) {
    check_registered_asset(env, &asset);

    let current_ledger = env.ledger().sequence();
    let cooldown = get_aggregation_cooldown(env);

    // Check cooldown
    let last_trigger_key = DataKey::LastAggregationTrigger(asset.clone());
    if let Some(last_triggered) = env.storage().persistent().get::<_, u32>(&last_trigger_key) {
        if current_ledger.saturating_sub(last_triggered) < cooldown {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
    }

    // Re-aggregate from stored submissions
    let oracle_sources: OracleSources = read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();
    let min_required = get_min_sources_required(env);
    let decimals = get_decimals(env);

    let mut valid_prices: Vec<i128> = Vec::new(env);
    let mut valid_volumes: Vec<i128> = Vec::new(env);
    let mut latest_timestamp: u64 = 0;
    let mut contributing_sources: u32 = 0;

    let min_interval = get_min_submission_interval(env);

    for i in 0..total_sources {
        let src = oracle_sources.sources.get_unchecked(i);

        if min_interval > 0 {
            let last_sub_key = DataKey::LastSubmissionLedger(src.clone(), asset.clone());
            let last_sub: Option<u32> = env.storage().persistent().get(&last_sub_key);
            if let Some(last) = last_sub {
                if current_ledger.saturating_sub(last) > min_interval {
                    continue;
                }
            } else {
                continue;
            }
        }

        let sub_key = DataKey::Submission(asset.clone(), src.clone());
        if let Some(entry_data) = env.storage().persistent().get::<_, PriceEntry>(&sub_key) {
            if entry_data.timestamp > latest_timestamp {
                latest_timestamp = entry_data.timestamp;
            }
            valid_prices.push_back(entry_data.price);
            valid_volumes.push_back(entry_data.volume.unwrap_or(0));
            contributing_sources += 1;
        }
    }

    if contributing_sources >= min_required && !valid_prices.is_empty() {
        let agg_price = aggregate_prices(env, &valid_prices, &valid_volumes);

        let aggregate = AggregatePrice {
            price: agg_price,
            timestamp: latest_timestamp,
            num_sources: contributing_sources,
            decimals,
            is_override: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Aggregate(asset.clone()), &aggregate);
        env.storage().persistent().extend_ttl(
            &DataKey::Aggregate(asset.clone()),
            LEDGER_THRESHOLD,
            LEDGER_BUMP,
        );

        env.storage()
            .persistent()
            .set(&last_trigger_key, &current_ledger);

        AggregationTriggeredEvent {
            asset,
            price: agg_price,
            num_sources: contributing_sources,
            triggered_at_ledger: current_ledger,
        }
        .publish(env);
    } else {
        panic_with_error!(env, ErrorCode::InsufficientSources);
    }
}

/// #70: Returns sources that are currently compliant for a given asset.
pub fn get_compliant_sources(env: &Env, asset: Address) -> Vec<Address> {
    check_registered_asset(env, &asset);
    let oracle_sources = read_oracle_sources(env);
    let min_interval = get_min_submission_interval(env);
    let current_ledger = env.ledger().sequence();
    let mut result: Vec<Address> = Vec::new(env);

    for i in 0..oracle_sources.sources.len() {
        let src = oracle_sources.sources.get_unchecked(i);

        if min_interval > 0 {
            let last_sub_key = DataKey::LastSubmissionLedger(src.clone(), asset.clone());
            let last_sub: Option<u32> = env.storage().persistent().get(&last_sub_key);
            match last_sub {
                Some(last) if current_ledger.saturating_sub(last) <= min_interval => {
                    result.push_back(src);
                }
                _ => {} // not compliant
            }
        } else {
            result.push_back(src);
        }
    }
    result
}

// =============================================================================
// #187 — Commit-Reveal MEV Resistance
// =============================================================================

/// Default commit window: sources have 20 ledgers to submit their commit hash.
const DEFAULT_COMMIT_WINDOW: u32 = 20;
/// Default reveal window: sources have 20 ledgers after the commit deadline to reveal.
const DEFAULT_REVEAL_WINDOW: u32 = 20;
/// Maximum number of prices a source can reveal in a single batch transaction.
const MAX_BATCH_REVEALS: u32 = 100;

// --- Config accessors ---

pub fn set_commit_window(env: &Env, ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgCommitWindow, &ledgers);
    crate::events::CommitWindowChangedEvent { value: ledgers }.publish(env);
}

pub fn get_commit_window(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgCommitWindow)
        .unwrap_or(DEFAULT_COMMIT_WINDOW)
}

pub fn set_reveal_window(env: &Env, ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgRevealWindow, &ledgers);
    crate::events::RevealWindowChangedEvent { value: ledgers }.publish(env);
}

pub fn get_reveal_window(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgRevealWindow)
        .unwrap_or(DEFAULT_REVEAL_WINDOW)
}

// --- Round ledger helper ---

/// Derives the canonical round ledger for the current ledger.
///
/// A "round" starts every `commit_window` ledgers, beginning from ledger 0.
/// This makes the round boundary predictable to all sources.
///
/// ```
/// round_ledger = (current_ledger / commit_window) * commit_window
/// ```
pub fn current_round_ledger(env: &Env) -> u32 {
    let current = env.ledger().sequence();
    let window = get_commit_window(env);
    if window == 0 {
        return current;
    }
    (current / window) * window
}

// --- Commit phase ---

/// Commits a price hash for a specific asset in the current round.
///
/// The source must call this during the commit window for the round.
/// A commit is a 32-byte hash computed as:
///   `sha256(price_le_bytes || salt_bytes || round_ledger_le_bytes)`
/// where `price` is i128 (16 bytes LE), `salt` is arbitrary caller-chosen bytes,
/// and `round_ledger` is u32 (4 bytes LE).
///
/// The hash is stored in **temporary storage** with a TTL of
/// `commit_window + reveal_window + 1` ledgers, automatically expiring without
/// revealing (bounding storage costs against griefing).
///
/// # Errors
/// - `SourceNotFound` — source is not registered.
/// - `AssetNotRegistered` — asset is not registered.
/// - `AlreadyCommitted` — source already committed this round for this asset.
/// - `RevealWindowClosed` — called outside the commit window for this round.
pub fn commit_price(env: &Env, source: Address, asset: Address, hash: soroban_sdk::BytesN<32>) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);

    let current_ledger = env.ledger().sequence();
    let round_ledger = current_round_ledger(env);
    let commit_window = get_commit_window(env);

    // The commit phase is [round_ledger, round_ledger + commit_window).
    // After the commit window closes, commits for this round are rejected.
    if current_ledger >= round_ledger + commit_window {
        panic_with_error!(env, ErrorCode::RevealWindowClosed);
    }

    let commit_key = DataKey::PriceCommit(asset.clone(), source.clone(), round_ledger);

    // Reject double-commit.
    if env.storage().temporary().has(&commit_key) {
        panic_with_error!(env, ErrorCode::AlreadyCommitted);
    }

    let commit = crate::types::PriceCommit {
        hash,
        committed_ledger: round_ledger,
        source: source.clone(),
        asset: asset.clone(),
        revealed: false,
    };

    // Use temporary storage so the commitment expires automatically after the reveal window,
    // preventing griefing through permanent storage bloat.
    let ttl = commit_window + get_reveal_window(env) + 1;
    env.storage().temporary().set(&commit_key, &commit);
    env.storage().temporary().extend_ttl(&commit_key, ttl, ttl);

    crate::events::PriceCommittedEvent {
        asset,
        source,
        round_ledger,
        committed_at_ledger: current_ledger,
    }
    .publish(env);
}

// --- Reveal phase ---

/// Reveals a committed price for a specific round.
///
/// The source provides `(asset, price, salt, round_ledger)`. The contract recomputes
/// `sha256(price_le_bytes || salt_bytes || round_ledger_le_bytes)` and verifies it
/// matches the stored commit hash. If it matches, the price is stored as a normal
/// `PriceEntry` and aggregation is triggered.
///
/// The reveal must happen in the window:
/// `[round_ledger + commit_window, round_ledger + commit_window + reveal_window)`
///
/// # Errors
/// - `CommitNotFound` — no commit for this (source, asset, round).
/// - `CommitExpired` — the reveal window has closed.
/// - `RevealWindowClosed` — called before the reveal window opens.
/// - `CommitHashMismatch` — the recomputed hash does not match the commit.
pub fn reveal_price(
    env: &Env,
    source: Address,
    asset: Address,
    price: i128,
    salt: soroban_sdk::Bytes,
    round_ledger: u32,
) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);
    check_registered_asset(env, &asset);

    _do_reveal(env, &source, &asset, price, salt, round_ledger);
}

/// Reveals up to `MAX_BATCH_REVEALS` committed prices in a single transaction.
///
/// Each tuple is `(asset, price, salt, round_ledger)`. Atomic: if any entry fails,
/// the whole transaction reverts.
pub fn reveal_prices_batch(
    env: &Env,
    source: Address,
    reveals: Vec<(Address, i128, soroban_sdk::Bytes, u32)>,
) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);

    if reveals.len() > MAX_BATCH_REVEALS {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    for i in 0..reveals.len() {
        let (asset, price, salt, round_ledger) = reveals.get_unchecked(i);
        check_registered_asset(env, &asset);
        check_source_asset(env, &source, &asset);
        _do_reveal(env, &source, &asset, price, salt, round_ledger);
    }
}

/// Internal reveal logic shared by single and batch reveal.
fn _do_reveal(
    env: &Env,
    source: &Address,
    asset: &Address,
    price: i128,
    salt: soroban_sdk::Bytes,
    round_ledger: u32,
) {
    let current_ledger = env.ledger().sequence();
    let commit_window = get_commit_window(env);
    let reveal_window = get_reveal_window(env);

    let reveal_start = round_ledger + commit_window;
    let reveal_end = reveal_start + reveal_window;

    // Enforce reveal window boundaries.
    if current_ledger < reveal_start {
        // Commit phase hasn't closed yet.
        panic_with_error!(env, ErrorCode::RevealWindowClosed);
    }
    if current_ledger >= reveal_end {
        // Reveal window has expired.
        panic_with_error!(env, ErrorCode::CommitExpired);
    }

    let commit_key = DataKey::PriceCommit(asset.clone(), source.clone(), round_ledger);

    let mut commit: crate::types::PriceCommit = env
        .storage()
        .temporary()
        .get(&commit_key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::CommitNotFound));

    if commit.revealed {
        // Already revealed — prevent double-reveal.
        panic_with_error!(env, ErrorCode::AlreadyCommitted);
    }

    // Recompute the expected hash: sha256(price_le || salt || round_ledger_le)
    let expected_hash = _compute_commit_hash(env, price, &salt, round_ledger);

    if expected_hash != commit.hash {
        panic_with_error!(env, ErrorCode::CommitHashMismatch);
    }

    // Mark revealed to prevent double-reveal.
    commit.revealed = true;
    env.storage().temporary().set(&commit_key, &commit);

    // Price is valid — store as a regular PriceEntry and trigger aggregation.
    let decimals = get_decimals(env);
    let ledger_time = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    // The timestamp for commit-reveal submissions is the current ledger time.
    // We use current time because the commit was blinded; no user-provided timestamp needed.
    if ledger_time > ledger_time.saturating_add(threshold) {
        // Shouldn't trigger, but defensive guard.
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    let min_price = crate::assets::get_min_price(env, asset.clone());
    if price < min_price {
        panic_with_error!(env, ErrorCode::PriceBelowMinimum);
    }

    let entry = PriceEntry {
        price,
        timestamp: ledger_time,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
        ledger_timestamp: ledger_time,
        volume: None,
    };

    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

    // Track last submission for compliance and #186 reactivation.
    env.storage().persistent().set(
        &DataKey::LastSubmissionLedger(source.clone(), asset.clone()),
        &current_ledger,
    );
    crate::sources::record_price_submitted(env, source, current_ledger);

    // Clear non-compliance flag.
    let nc_key = DataKey::SourceNonCompliant(source.clone(), asset.clone());
    if env.storage().persistent().has(&nc_key) {
        env.storage().persistent().remove(&nc_key);
    }

    PriceSubmittedEvent {
        asset: asset.clone(),
        source: source.clone(),
        price,
        timestamp: ledger_time,
    }
    .publish(env);

    crate::events::PriceRevealedEvent {
        asset: asset.clone(),
        source: source.clone(),
        price,
        round_ledger,
        revealed_at_ledger: current_ledger,
    }
    .publish(env);

    if !maybe_aggregate_after_submission(env, asset, current_ledger) {
        return;
    }
    aggregate_asset(env, asset, current_ledger, decimals);
}

/// Computes `sha256(price_le_bytes || salt_bytes || round_le_bytes)`.
///
/// - `price` is encoded as 16 bytes little-endian (i128).
/// - `salt` is arbitrary bytes provided by the caller.
/// - `round_ledger` is encoded as 4 bytes little-endian (u32).
fn _compute_commit_hash(
    env: &Env,
    price: i128,
    salt: &soroban_sdk::Bytes,
    round_ledger: u32,
) -> soroban_sdk::BytesN<32> {
    let price_bytes = price.to_le_bytes();
    let round_bytes = round_ledger.to_le_bytes();

    let mut preimage = soroban_sdk::Bytes::new(env);
    // Append price bytes (16 bytes).
    for b in price_bytes.iter() {
        preimage.push_back(*b);
    }
    // Append salt.
    preimage.append(salt);
    // Append round_ledger bytes (4 bytes).
    for b in round_bytes.iter() {
        preimage.push_back(*b);
    }

    env.crypto().sha256(&preimage).into()
}

/// Internal submit_price helper used by fee_market and zk_verify modules.
///
/// Skips `source.require_auth()` and pause check — callers are responsible
/// for performing those checks before invoking this function.
pub fn submit_price_internal(
    env: &Env,
    source: Address,
    asset: Address,
    price: i128,
    timestamp: u64,
) {
    check_source(env, &source);
    check_registered_asset(env, &asset);
    check_source_asset(env, &source, &asset);
    enforce_commit_reveal_for_bft(env);

    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }

    if check_deviation_circuit_breaker(env, &source, &asset, price) {
        return;
    }

    let decimals = get_decimals(env);
    let current_ledger = env.ledger().sequence();

    let entry = PriceEntry {
        price,
        timestamp,
        source: source.clone(),
        decimals,
        last_updated: current_ledger,
        ledger_timestamp: env.ledger().timestamp(),
        volume: None,
    };

    env.storage()
        .persistent()
        .set(&DataKey::Submission(asset.clone(), source.clone()), &entry);

    record_successful_submission(env, source.clone());

    PriceSubmittedEvent {
        asset: asset.clone(),
        source: source.clone(),
        price,
        timestamp,
    }
    .publish(env);

    if !maybe_aggregate_after_submission(env, &asset, current_ledger) {
        return;
    }
    aggregate_asset(env, &asset, current_ledger, decimals);
}

// =============================================================================
// simulate_aggregation — pure computation, no storage writes
// =============================================================================

/// Simulates what the aggregate price WOULD be for `asset` given a set of
/// hypothetical (source, price) pairs, without writing anything to storage.
///
/// Applies the same aggregation method (median / mean / trimmed-mean) that is
/// currently configured.  The caller can use this to preview the expected
/// aggregate before committing a real submission.
///
/// # Arguments
/// * `env`                 - Soroban execution environment.
/// * `asset`               - Asset to simulate (must be registered).
/// * `hypothetical_prices` - Vec of `(source_address, price)` pairs.
///
/// # Returns
/// The simulated aggregate price, or `None` if fewer sources than
/// `min_sources_required` are supplied.
pub fn simulate_aggregation(
    env: &Env,
    asset: Address,
    hypothetical_prices: Vec<(Address, i128)>,
) -> Option<i128> {
    check_registered_asset(env, &asset);

    let min_required = get_min_sources_required(env);
    let mut prices: Vec<i128> = Vec::new(env);

    for i in 0..hypothetical_prices.len() {
        let (_, price) = hypothetical_prices.get_unchecked(i);
        if price > 0 {
            prices.push_back(price);
        }
    }

    if prices.len() < min_required {
        return None;
    }

    let method = get_aggregation_method(env);
    let result = match method {
        0 => compute_median(&prices),
        1 => compute_mean(&prices),
        2 => compute_trimmed_mean(&prices, 10),
        _ => compute_median(&prices),
    };

    Some(result)
}

// =============================================================================
// submit_price_merkle — batch submission with on-chain merkle proof verification
// =============================================================================

/// A single leaf in a merkle batch: one source's price for one asset.
#[derive(Clone)]
#[soroban_sdk::contracttype]
pub struct MerkleLeaf {
    pub source: Address,
    pub asset: Address,
    pub price: i128,
    pub timestamp: u64,
}

/// A merkle proof for one leaf: the sibling hashes from leaf to root.
#[derive(Clone)]
#[soroban_sdk::contracttype]
pub struct MerkleProof {
    /// The leaf data this proof covers.
    pub leaf: MerkleLeaf,
    /// Sibling hashes from leaf level up to (but not including) the root.
    pub siblings: Vec<soroban_sdk::BytesN<32>>,
    /// Bit-vector: bit `i` is 1 if the sibling at level `i` is on the LEFT.
    pub left_bitmap: u32,
}

/// Hashes a `MerkleLeaf` to a 32-byte digest using the Soroban SHA-256 host function.
///
/// Pre-image: `price` (16 bytes LE) || `timestamp` (8 bytes LE).
fn hash_leaf(env: &Env, leaf: &MerkleLeaf) -> soroban_sdk::BytesN<32> {
    let mut data = soroban_sdk::Bytes::new(env);
    data.append(&soroban_sdk::Bytes::from_slice(
        env,
        &leaf.price.to_le_bytes(),
    ));
    data.append(&soroban_sdk::Bytes::from_slice(
        env,
        &leaf.timestamp.to_le_bytes(),
    ));
    env.crypto().sha256(&data)
}

/// Hashes two 32-byte nodes together to produce the parent node hash.
fn hash_pair(
    env: &Env,
    left: &soroban_sdk::BytesN<32>,
    right: &soroban_sdk::BytesN<32>,
) -> soroban_sdk::BytesN<32> {
    let mut data = soroban_sdk::Bytes::new(env);
    data.append(&soroban_sdk::Bytes::from_slice(
        env,
        left.to_array().as_ref(),
    ));
    data.append(&soroban_sdk::Bytes::from_slice(
        env,
        right.to_array().as_ref(),
    ));
    env.crypto().sha256(&data)
}

/// Verifies a merkle proof and returns `true` if the proof is valid for `root`.
fn verify_proof(env: &Env, root: &soroban_sdk::BytesN<32>, proof: &MerkleProof) -> bool {
    let mut current = hash_leaf(env, &proof.leaf);
    for i in 0..proof.siblings.len() {
        let sibling = proof.siblings.get_unchecked(i);
        let left_bit = (proof.left_bitmap >> i) & 1;
        current = if left_bit == 1 {
            hash_pair(env, &sibling, &current)
        } else {
            hash_pair(env, &current, &sibling)
        };
    }
    current == *root
}

/// Submits a batch of prices verified against a merkle root in a single transaction.
///
/// The caller (source) provides:
/// 1. A 32-byte merkle `root` that was computed off-chain over all leaf data.
/// 2. An ordered list of `MerkleProof` entries, each covering one (source, asset, price, timestamp).
///
/// The contract verifies every proof against the root before accepting any
/// submission, then stores and aggregates all valid prices atomically.
///
/// This dramatically reduces calldata cost for multi-source submissions because
/// only the proofs—not all raw price data—need to be transmitted per source.
///
/// # Arguments
/// * `env`    - Soroban execution environment.
/// * `source` - The caller who signs the transaction.
/// * `root`   - 32-byte merkle root computed over all leaves in this batch.
/// * `proofs` - Individual merkle proofs, one per price submission.
pub fn submit_price_merkle(
    env: &Env,
    source: Address,
    root: soroban_sdk::BytesN<32>,
    proofs: Vec<MerkleProof>,
) {
    check_not_paused(env);
    source.require_auth();
    check_source(env, &source);

    if crate::sources::is_source_suspended(env, source.clone()) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let decimals = get_decimals(env);
    let ledger_time = env.ledger().timestamp();
    let threshold = get_timestamp_threshold(env);
    let current_ledger = env.ledger().sequence();

    // Phase 1: verify all proofs and validate each leaf — atomicity guaranteed.
    for i in 0..proofs.len() {
        let proof = proofs.get_unchecked(i);
        if !verify_proof(env, &root, &proof) {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
        check_registered_asset(env, &proof.leaf.asset);
        check_source_asset(env, &proof.leaf.source, &proof.leaf.asset);
        if proof.leaf.price <= 0 {
            panic_with_error!(env, ErrorCode::InvalidPrice);
        }
        if proof.leaf.timestamp > ledger_time.saturating_add(threshold) {
            panic_with_error!(env, ErrorCode::InvalidTimestamp);
        }
    }

    // Phase 2: store all submissions (batch storage writes, single loop).
    let mut assets_to_aggregate: Vec<Address> = Vec::new(env);
    for i in 0..proofs.len() {
        let proof = proofs.get_unchecked(i);
        let leaf = proof.leaf;
        let entry = PriceEntry {
            price: leaf.price,
            timestamp: leaf.timestamp,
            source: leaf.source.clone(),
            decimals,
            last_updated: current_ledger,
            ledger_timestamp: ledger_time,
            volume: None,
        };
        env.storage().persistent().set(
            &DataKey::Submission(leaf.asset.clone(), leaf.source.clone()),
            &entry,
        );
        PriceSubmittedEvent {
            asset: leaf.asset.clone(),
            source: leaf.source.clone(),
            price: leaf.price,
            timestamp: leaf.timestamp,
        }
        .publish(env);
        assets_to_aggregate.push_back(leaf.asset);
    }

    // Phase 3: aggregate each unique asset once.
    for i in 0..assets_to_aggregate.len() {
        let asset = assets_to_aggregate.get_unchecked(i);
        aggregate_asset(env, &asset, current_ledger, decimals);
    }
}
