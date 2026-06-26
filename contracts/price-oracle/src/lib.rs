#![no_std]

mod admin;
mod assets;
mod errors;
mod events;
mod history;
mod pause;
mod prices;
mod sources;
mod storage;
mod timelock;
mod types;

#[cfg(test)]
mod override_tests;

#[cfg(test)]
mod prop_tests;

pub use types::{
    AggregatePrice, AggregationMethod, Asset, DataKey, ErrorCode, OracleSources, PriceData,
    PriceEntry, PriceHistoryEntry, PriceOverrideEntry,
};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Symbol, Vec};

use crate::storage::read_registered_assets;

#[contract]
pub struct PriceOracleContract;

#[contractimpl]
impl PriceOracleContract {
    // --- Admin ---

    pub fn initialize(
        env: Env,
        admin: Address,
        min_sources_required: u32,
        max_history_length: u32,
        decimals: u32,
        description: String,
    ) {
        admin::initialize(
            &env,
            admin,
            min_sources_required,
            max_history_length,
            decimals,
            description,
        );
    }

    pub fn upgrade(env: Env, new_wasm_hash: soroban_sdk::BytesN<32>) {
        admin::upgrade(&env, new_wasm_hash);
    }

    pub fn set_admin(env: Env, new_admin: Address) {
        admin::set_admin(&env, new_admin);
    }

    pub fn get_admin_address(env: Env) -> Address {
        admin::get_admin_address(&env)
    }

    pub fn set_min_sources_required(env: Env, new_min: u32) {
        admin::set_min_sources_required(&env, new_min);
    }

    pub fn get_min_sources_required(env: Env) -> u32 {
        admin::get_min_sources_required(&env)
    }

    pub fn set_max_history_length(env: Env, new_max: u32) {
        admin::set_max_history_length(&env, new_max);
    }

    pub fn get_max_history_length(env: Env) -> u32 {
        admin::get_max_history_length(&env)
    }

    pub fn set_resolution(env: Env, new_resolution: u32) {
        admin::set_resolution(&env, new_resolution);
    }

    pub fn get_resolution(env: Env) -> u32 {
        admin::get_resolution(&env)
    }

    pub fn set_decimals(env: Env, new_decimals: u32) {
        admin::set_decimals(&env, new_decimals);
    }

    pub fn get_decimals(env: Env) -> u32 {
        admin::get_decimals(&env)
    }

    pub fn set_description(env: Env, new_description: String) {
        admin::set_description(&env, new_description);
    }

    pub fn get_description(env: Env) -> String {
        admin::get_description(&env)
    }

    pub fn set_timestamp_threshold(env: Env, threshold: u64) {
        admin::set_timestamp_threshold(&env, threshold);
    }

    pub fn get_timestamp_threshold(env: Env) -> u64 {
        admin::get_timestamp_threshold(&env)
    }

    pub fn set_max_price_deviation(env: Env, deviation_basis_points: u32) {
        admin::set_max_price_deviation(&env, deviation_basis_points);
    }

    pub fn get_max_price_deviation(env: Env) -> u32 {
        admin::get_max_price_deviation(&env)
    }

    pub fn set_heartbeat_interval(env: Env, interval: u64) {
        admin::set_heartbeat_interval(&env, interval);
    }

    pub fn get_heartbeat_interval(env: Env) -> u64 {
        admin::get_heartbeat_interval(&env)
    }

    // --- Sources ---

    pub fn add_source(env: Env, source: Address, name: String) {
        sources::add_source(&env, source, name);
    }

    pub fn remove_source(env: Env, source: Address) {
        sources::remove_source(&env, source);
    }

    pub fn is_source(env: Env, source: Address) -> bool {
        sources::is_source(&env, source)
    }

    pub fn get_oracle_sources(env: Env) -> OracleSources {
        sources::get_oracle_sources(&env)
    }

    pub fn submit_heartbeat(env: Env, source: Address) {
        sources::submit_heartbeat(&env, source);
    }

    pub fn is_source_inactive(env: Env, source: Address) -> bool {
        sources::is_source_inactive(&env, source)
    }

    pub fn get_inactive_sources(env: Env) -> u32 {
        sources::get_inactive_sources(&env)
    }

    pub fn get_source_last_heartbeat(env: Env, source: Address) -> u64 {
        sources::get_source_last_heartbeat(&env, source)
    }

    // --- Assets ---

    pub fn register_asset(env: Env, asset: Address) {
        assets::register_asset(&env, asset);
    }

    pub fn unregister_asset(env: Env, asset: Address) {
        assets::unregister_asset(&env, asset);
    }

    pub fn is_asset_registered(env: Env, asset: Address) -> bool {
        assets::is_asset_registered(&env, asset)
    }

    // --- Prices ---

    pub fn submit_price(env: Env, source: Address, asset: Address, price: i128, timestamp: u64) {
        prices::submit_price(&env, source, asset, price, timestamp);
    }

    pub fn get_price(env: Env, asset: Address, max_age: u64) -> Option<AggregatePrice> {
        prices::get_price(&env, asset, max_age)
    }

    pub fn get_source_price(env: Env, asset: Address, source: Address) -> PriceEntry {
        prices::get_source_price(&env, asset, source)
    }

    pub fn get_all_prices(env: Env, asset: Address) -> Vec<PriceEntry> {
        prices::get_all_prices(&env, asset)
    }

    pub fn override_price(
        env: Env,
        asset: Address,
        price: i128,
        reason: String,
        expiry_ledger: u32,
    ) {
        prices::override_price(&env, asset, price, reason, expiry_ledger);
    }

    pub fn remove_price_override(env: Env, asset: Address) {
        prices::remove_price_override(&env, asset);
    }

    pub fn get_price_override(env: Env, asset: Address) -> Option<PriceOverrideEntry> {
        prices::get_price_override(&env, asset)
    }

    pub fn get_latest_ledger(env: Env) -> u32 {
        env.ledger().sequence()
    }

    // --- History ---

    pub fn get_historical_price(env: Env, asset: Address, ledger: u32) -> PriceHistoryEntry {
        history::get_historical_price(&env, asset, ledger)
    }

    pub fn has_historical_price(env: Env, asset: Address, ledger: u32) -> bool {
        history::has_historical_price(&env, asset, ledger)
    }

    pub fn get_historical_prices(
        env: Env,
        asset: Address,
        start_ledger: u32,
        end_ledger: u32,
    ) -> Vec<PriceHistoryEntry> {
        history::get_historical_prices(&env, asset, start_ledger, end_ledger)
    }

    // --- SEP-40 Oracle Interface ---

    pub fn decimals(env: Env) -> u32 {
        Self::get_decimals(env)
    }

    pub fn base(env: Env) -> Asset {
        Asset::Other(Symbol::new(&env, "USD"))
    }

    pub fn assets(env: Env) -> Vec<Asset> {
        let registered = read_registered_assets(&env);
        let mut result: Vec<Asset> = Vec::new(&env);
        for i in 0..registered.len() {
            result.push_back(Asset::Stellar(registered.get_unchecked(i)));
        }
        result
    }

    pub fn resolution(env: Env) -> u32 {
        admin::get_resolution(&env)
    }

    pub fn lastprice(env: Env, asset: Asset) -> Option<PriceData> {
        prices::lastprice(&env, asset)
    }

    pub fn price(env: Env, asset: Asset, timestamp: u64) -> Option<PriceData> {
        prices::price(&env, asset, timestamp)
    }

    pub fn prices(env: Env, asset: Asset, records: u32) -> Option<Vec<PriceData>> {
        prices::prices(&env, asset, records)
    }

    // --- Pause ---

    pub fn pause(env: Env) {
        pause::pause(&env);
    }

    pub fn unpause(env: Env) {
        pause::unpause(&env);
    }

    pub fn is_paused(env: Env) -> bool {
        pause::is_paused(&env)
    }

    // --- Timelock ---

    pub fn propose_operation(env: Env, op_type: u32, data: soroban_sdk::Bytes) -> u32 {
        let op_enum = match op_type {
            0 => types::OperationType::Upgrade,
            1 => types::OperationType::SetAdmin,
            2 => types::OperationType::SetMinSources,
            3 => types::OperationType::SetMaxHistory,
            4 => types::OperationType::SetResolution,
            5 => types::OperationType::SetDecimals,
            6 => types::OperationType::SetDescription,
            7 => types::OperationType::SetTimestampThreshold,
            _ => panic!("Invalid operation type"),
        };
        timelock::propose_operation(&env, op_enum, &data)
    }

    pub fn execute_operation(env: Env, op_id: u32) {
        timelock::execute_operation(&env, op_id);
    }

    pub fn cancel_operation(env: Env, op_id: u32) {
        timelock::cancel_operation(&env, op_id);
    }

    pub fn get_timelock_duration(env: Env) -> u32 {
        timelock::get_timelock_duration(&env)
    }

    pub fn set_timelock_duration(env: Env, duration: u32) {
        timelock::set_timelock_duration(&env, duration);
    }
}

#[cfg(test)]
mod test_helpers;

mod test;
