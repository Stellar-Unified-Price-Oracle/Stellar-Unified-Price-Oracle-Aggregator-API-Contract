use soroban_sdk::{Env, Vec};

use crate::admin::get_resolution;
use crate::pause::is_paused;
use crate::sources::is_source_inactive;
use crate::storage::{read_oracle_sources, read_registered_assets};
use crate::types::{DataKey, HealthReport};

pub fn health_check(env: &Env) -> HealthReport {
    let oracle_sources = read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();

    // Count inactive/suspended sources
    let mut suspended_source_count: u32 = 0;
    for i in 0..oracle_sources.sources.len() {
        let source = oracle_sources.sources.get_unchecked(i);
        if is_source_inactive(env, source) {
            suspended_source_count += 1;
        }
    }
    let active_sources = total_sources.saturating_sub(suspended_source_count);

    let registered_assets: Vec<soroban_sdk::Address> = read_registered_assets(env);
    let total_assets = registered_assets.len();

    let resolution = get_resolution(env);
    let current_time = env.ledger().timestamp();

    let mut assets_with_prices: u32 = 0;
    let mut stale_price_count: u32 = 0;
    let mut last_aggregation_ledger: u32 = 0;

    for i in 0..registered_assets.len() {
        let asset = registered_assets.get_unchecked(i);
        let aggregate: Option<crate::types::AggregatePrice> = env
            .storage()
            .persistent()
            .get(&DataKey::Aggregate(asset.clone()));

        if let Some(agg) = aggregate {
            assets_with_prices += 1;

            // Track most recent aggregation ledger
            // AggregatePrice doesn't have a ledger field; use last_updated via history
            // We approximate using the history ledgers list
            let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
            let ledgers: Option<soroban_sdk::Vec<u32>> =
                env.storage().persistent().get(&ledgers_key);
            if let Some(leds) = ledgers {
                if !leds.is_empty() {
                    let last = leds.get_unchecked(leds.len() - 1);
                    if last > last_aggregation_ledger {
                        last_aggregation_ledger = last;
                    }
                }
            }

            // Check staleness when resolution is configured
            if resolution > 0 {
                let age = current_time.saturating_sub(agg.timestamp);
                if age > resolution as u64 {
                    stale_price_count += 1;
                }
            }
        }
    }

    HealthReport {
        total_sources,
        active_sources,
        total_assets,
        assets_with_prices,
        is_aggregation_paused: is_paused(env),
        last_aggregation_ledger,
        stale_price_count,
        suspended_source_count,
    }
}
