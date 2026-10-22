#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, Env, Vec,
};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyInitialized = 1,
    NotAuthorized = 2,
    SourceAlreadyExists = 3,
    SourceNotFound = 4,
    InsufficientSources = 5,
    InvalidPrice = 6,
    NoData = 7,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Decimals,
    Source(Address),
    Sources,
    Price(Address, Address), // (asset, source)
}

#[contract]
pub struct MinimalOracle;

#[contractimpl]
impl MinimalOracle {
    /// Initialize the contract with an admin address and price decimals.
    /// Panics with `AlreadyInitialized` if called more than once.
    pub fn initialize(env: Env, admin: Address, decimals: u32) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic_with_error!(&env, Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::Decimals, &decimals);
        env.storage()
            .persistent()
            .set(&DataKey::Sources, &Vec::<Address>::new(&env));
    }

    /// Register a new oracle source. Admin-only.
    /// Panics with `SourceAlreadyExists` if the source is already registered.
    pub fn add_source(env: Env, source: Address) {
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if env
            .storage()
            .persistent()
            .has(&DataKey::Source(source.clone()))
        {
            panic_with_error!(&env, Error::SourceAlreadyExists);
        }
        env.storage()
            .persistent()
            .set(&DataKey::Source(source.clone()), &true);
        let mut sources: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Sources)
            .unwrap_or(Vec::new(&env));
        sources.push_back(source);
        env.storage().persistent().set(&DataKey::Sources, &sources);
    }

    /// Remove a registered oracle source. Admin-only.
    /// Panics with `SourceNotFound` if the source is not registered.
    pub fn remove_source(env: Env, source: Address) {
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Source(source.clone()))
        {
            panic_with_error!(&env, Error::SourceNotFound);
        }
        env.storage()
            .persistent()
            .remove(&DataKey::Source(source.clone()));
        let sources: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Sources)
            .unwrap_or(Vec::new(&env));
        let mut new_sources: Vec<Address> = Vec::new(&env);
        for i in 0..sources.len() {
            let s = sources.get_unchecked(i);
            if s != source {
                new_sources.push_back(s);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::Sources, &new_sources);
    }

    /// Submit a price for an asset. Source-only (requires source.require_auth()).
    /// Panics with `SourceNotFound` if the source is not registered.
    /// Panics with `InvalidPrice` if price is zero or negative.
    pub fn submit_price(env: Env, source: Address, asset: Address, price: i128) {
        source.require_auth();
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Source(source.clone()))
        {
            panic_with_error!(&env, Error::SourceNotFound);
        }
        if price <= 0 {
            panic_with_error!(&env, Error::InvalidPrice);
        }
        env.storage()
            .persistent()
            .set(&DataKey::Price(asset, source), &price);
    }

    /// Get the median price for an asset across all registered sources.
    /// Only sources that have submitted a price for the asset are included.
    /// Panics with `NoData` if no sources have submitted a price.
    pub fn get_price(env: Env, asset: Address) -> i128 {
        let sources: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Sources)
            .unwrap_or(Vec::new(&env));
        let mut prices: Vec<i128> = Vec::new(&env);
        for i in 0..sources.len() {
            let src = sources.get_unchecked(i);
            let key = DataKey::Price(asset.clone(), src);
            if let Some(p) = env.storage().persistent().get::<_, i128>(&key) {
                prices.push_back(p);
            }
        }
        if prices.is_empty() {
            panic_with_error!(&env, Error::NoData);
        }
        compute_median(&env, prices)
    }
}

/// Compute the median of a non-empty price list using insertion sort.
fn compute_median(_env: &Env, mut prices: Vec<i128>) -> i128 {
    let n = prices.len();
    for i in 1..n {
        let mut j = i;
        while j > 0 && prices.get_unchecked(j - 1) > prices.get_unchecked(j) {
            let a = prices.get_unchecked(j - 1);
            let b = prices.get_unchecked(j);
            prices.set(j - 1, b);
            prices.set(j, a);
            j -= 1;
        }
    }
    if n % 2 == 1 {
        prices.get_unchecked(n / 2)
    } else {
        let lo = prices.get_unchecked(n / 2 - 1);
        let hi = prices.get_unchecked(n / 2);
        (lo + hi) / 2
    }
}

#[cfg(test)]
mod test;
