#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String, Vec,
};

use crate::{PriceOracleContract, PriceOracleContractClient};

/// Creates a contract client without initializing it.
pub fn create_contract(e: &Env) -> PriceOracleContractClient<'_> {
    e.mock_all_auths();
    let contract_id = e.register(PriceOracleContract, ());
    PriceOracleContractClient::new(e, &contract_id)
}

/// Creates and initializes a contract with sensible defaults; returns (client, admin).
pub fn setup_contract(e: &Env) -> (PriceOracleContractClient<'_>, Address) {
    let admin = Address::generate(e);
    let client = create_contract(e);
    client.initialize(
        &admin,
        &2u32,
        &10u32,
        &18u32,
        &String::from_str(e, "Stellar Price Oracle Aggregator"),
    );
    (client, admin)
}

/// Adds a source with a generated address and the given name; returns the address.
pub fn register_test_source(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    name: &str,
) -> Address {
    let source = Address::generate(e);
    client.add_source(&source, &String::from_str(e, name));
    source
}

/// Registers an asset with a generated address; returns the address.
pub fn register_test_asset(e: &Env, client: &PriceOracleContractClient<'_>) -> Address {
    let asset = Address::generate(e);
    client.register_asset(&asset);
    asset
}

/// Submits a price from the given source for the given asset.
pub fn submit_test_price(
    client: &PriceOracleContractClient<'_>,
    source: &Address,
    asset: &Address,
    price: i128,
    timestamp: u64,
) {
    client.submit_price(source, asset, &price, &timestamp);
}

/// Creates an initialized contract with N sources and M assets; sets min_sources to N.
/// Returns (client, admin, sources, assets).
pub fn setup_full_oracle<'a>(
    e: &'a Env,
    num_sources: u32,
    num_assets: u32,
) -> (
    PriceOracleContractClient<'a>,
    Address,
    Vec<Address>,
    Vec<Address>,
) {
    let (client, admin) = setup_contract(e);
    client.set_min_sources_required(&num_sources);
    let mut sources: Vec<Address> = Vec::new(e);
    let mut assets: Vec<Address> = Vec::new(e);
    for _ in 0..num_sources {
        let source = register_test_source(e, &client, "Source");
        sources.push_back(source);
    }
    for _ in 0..num_assets {
        let asset = register_test_asset(e, &client);
        assets.push_back(asset);
    }
    (client, admin, sources, assets)
}

/// Clears all authorizations on the env to test unauthorized access.
pub fn clear_auth(e: &Env) {
    use soroban_sdk::xdr::SorobanAuthorizationEntry;
    e.set_auths(&[] as &[SorobanAuthorizationEntry]);
}
/// Sets the ledger sequence number and timestamp.
pub fn ledger_default(e: &Env, seq: u32, timestamp: u64) {
    e.ledger().set(LedgerInfo {
        timestamp,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 4096,
    });
}

/// Initializes an already-created contract with the given admin address.
pub fn init_admin(client: &PriceOracleContractClient<'_>, admin: &Address) {
    let e = client.env();
    client.initialize(
        admin,
        &1u32,
        &100u32,
        &18u32,
        &String::from_str(e, "Stellar Price Oracle Aggregator"),
    );
}

/// Sets up a contract with one source and one asset for event testing.
/// Returns (client, admin, source, asset).
pub fn setup_basic(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address, Address) {
    let (client, admin) = setup_contract(e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(e, &client, "Source");
    let asset = register_test_asset(e, &client);
    (client, admin, source, asset)
}
