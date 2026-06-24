#![no_std]

mod admin;
mod assets;
mod errors;
mod events;
mod history;
mod prices;
mod sources;
mod storage;
mod types;

pub use types::{
    AggregatePrice, Asset, DataKey, ErrorCode, OracleSources, PriceData, PriceEntry,
    PriceHistoryEntry,
};

use soroban_sdk::{contract, contractimpl, Address, Env, String, Symbol, Vec};

use crate::storage::read_registered_assets;

#[contract]
pub struct PriceOracleContract;

#[contractimpl]
impl PriceOracleContract {
    pub fn __constructor(_env: Env) {}

    // --- Admin ---

    pub fn initialize(
        env: Env,
        admin: Address,
        min_sources_required: u32,
        max_history_length: u32,
        decimals: u32,
        description: String,
    ) {
        admin::initialize(&env, admin, min_sources_required, max_history_length, decimals, description);
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

    pub fn get_price(env: Env, asset: Address) -> AggregatePrice {
        prices::get_price(&env, asset)
    }

    pub fn get_source_price(env: Env, asset: Address, source: Address) -> PriceEntry {
        prices::get_source_price(&env, asset, source)
    }

    pub fn get_all_prices(env: Env, asset: Address) -> Vec<PriceEntry> {
        prices::get_all_prices(&env, asset)
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
}

mod test;
