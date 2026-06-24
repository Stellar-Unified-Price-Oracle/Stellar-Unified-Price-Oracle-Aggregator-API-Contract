#![cfg(test)]

extern crate std;

use soroban_sdk::Address;

use crate::PriceOracleContractClient;

/// Prints all registered oracle sources and their names to stdout.
pub fn print_sources(client: &PriceOracleContractClient<'_>) {
    let sources = client.get_oracle_sources();
    std::println!("=== Sources ({}) ===", sources.sources.len());
    for i in 0..sources.sources.len() {
        let addr = sources.sources.get_unchecked(i);
        let name = sources.metadata.get(addr.clone());
        std::println!("  [{}] {:?}  name={:?}", i, addr, name);
    }
}

/// Prints all registered assets to stdout.
pub fn print_assets(client: &PriceOracleContractClient<'_>) {
    let assets = client.assets();
    std::println!("=== Assets ({}) ===", assets.len());
    for i in 0..assets.len() {
        std::println!("  [{}] {:?}", i, assets.get_unchecked(i));
    }
}

/// Prints current contract configuration parameters to stdout.
pub fn print_config(client: &PriceOracleContractClient<'_>) {
    std::println!("=== Config ===");
    std::println!("  admin:       {:?}", client.get_admin_address());
    std::println!("  min_sources: {}", client.get_min_sources_required());
    std::println!("  max_history: {}", client.get_max_history_length());
    std::println!("  decimals:    {}", client.get_decimals());
    std::println!("  description: {:?}", client.get_description());
    std::println!("  resolution:  {}", client.resolution());
}

/// Prints the aggregated price and all per-source prices for an asset to stdout.
pub fn print_prices(client: &PriceOracleContractClient<'_>, asset: &Address) {
    std::println!("=== Prices for {:?} ===", asset);
    match client.try_get_price(asset, &0u64) {
        Ok(Ok(agg)) => {
            std::println!(
                "  aggregate: price={} ts={} sources={} decimals={}",
                agg.price, agg.timestamp, agg.num_sources, agg.decimals
            );
        }
        _ => {
            std::println!("  aggregate: (unavailable)");
        }
    }
    match client.try_get_all_prices(asset) {
        Ok(Ok(entries)) => {
            for i in 0..entries.len() {
                let e = entries.get_unchecked(i);
                std::println!(
                    "  source[{}] {:?}: price={} ts={}",
                    i, e.source, e.price, e.timestamp
                );
            }
        }
        _ => {}
    }
}
