//! #823 — Core suites against the compiled release WASM.
//!
//! The rest of the test suite registers the contract from Rust source
//! (`env.register(PriceOracleContract, ())`), so a regression that only shows up
//! in the optimised `wasm32v1-none` artifact (symbol export, host-function
//! usage, size-driven behaviour) would pass CI unnoticed.
//!
//! This module registers the **release WASM** instead:
//!
//! ```ignore
//! let id = env.register(WASM, ());
//! ```
//!
//! and then drives the SEP-40 conformance surface and the attack-regression
//! corpus through the resulting contract id. It is gated behind the
//! `wasm-conformance` feature because it needs a pre-built artifact at
//! `target/wasm32v1-none/release/price_oracle.wasm`; CI builds that artifact
//! first and then runs `cargo test -p price-oracle --features wasm-conformance
//! --lib wasm_conformance_tests`.
//!
//! See `docs/wasm-conformance.md`.

#![cfg(all(test, feature = "wasm-conformance"))]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, IntoVal, Symbol, TryFromVal, Val, Vec,
};

use crate::test_helpers::{ledger_default, register_test_asset, register_test_source};
use crate::types::ErrorCode;
use crate::{Asset, PriceOracleContractClient};

/// The release artifact under test.
const WASM: &[u8] = include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm");

/// Registers the compiled release WASM and returns a client bound to it.
///
/// Unlike [`crate::test_helpers::create_contract`], nothing here is compiled
/// from source: every call below crosses the real WASM boundary.
fn deploy_wasm(e: &Env) -> PriceOracleContractClient<'_> {
    e.mock_all_auths();
    let contract_id = e.register(WASM, ());
    PriceOracleContractClient::new(e, &contract_id)
}

/// Deploys the WASM contract and initializes it with `decimals` precision.
fn wasm_oracle(e: &Env, decimals: u32) -> PriceOracleContractClient<'_> {
    let admin = Address::generate(e);
    let client = deploy_wasm(e);
    client.initialize(
        &admin,
        &1u32,
        &10u32,
        &decimals,
        &soroban_sdk::String::from_str(e, "WASM conformance"),
    );
    client
}

/// Argument list for a raw `Env::invoke_contract` call.
fn args(e: &Env, values: &[Val]) -> Vec<Val> {
    let mut vals: Vec<Val> = Vec::new(e);
    for v in values {
        vals.push_back(*v);
    }
    vals
}

/// Calls the WASM contract with the SEP-40 symbol name, bypassing the generated
/// client so the exported symbol itself is under test.
fn call<T>(e: &Env, addr: &Address, name: &str, values: &[Val]) -> T
where
    T: TryFromVal<Env, Val>,
{
    let func = Symbol::new(e, name);
    e.invoke_contract::<T>(addr, &func, args(e, values))
}

// ---------------------------------------------------------------------------
// SEP-40 conformance surface, against the WASM
// ---------------------------------------------------------------------------

/// SEP40-R01 — `base()` is exported by the WASM and returns the denomination.
#[test]
fn wasm_sep40_r01_base_is_exported() {
    let e = Env::default();
    let client = wasm_oracle(&e, 18);
    let base: Asset = call(&e, &client.address, "base", &[]);
    assert_eq!(base, Asset::Other(Symbol::new(&e, "USD")));
    assert_eq!(client.base(), base);
}

/// SEP40-R02 — `assets()` is exported and lists quoted assets as `Stellar`.
#[test]
fn wasm_sep40_r02_assets_are_listed() {
    let e = Env::default();
    let client = wasm_oracle(&e, 18);
    register_test_asset(&e, &client);
    register_test_asset(&e, &client);

    let listed: Vec<Asset> = call(&e, &client.address, "assets", &[]);
    assert_eq!(listed.len(), 2);
    assert!(matches!(listed.get_unchecked(0), Asset::Stellar(_)));
}

/// SEP40-R03 — `decimals()` is exported and reports the configured precision.
#[test]
fn wasm_sep40_r03_decimals_is_exported() {
    let e = Env::default();
    for d in [0u32, 6, 7, 18] {
        let client = wasm_oracle(&e, d);
        let reported: u32 = call(&e, &client.address, "decimals", &[]);
        assert_eq!(reported, d);
    }
}

/// SEP40-R04 — `resolution()` is exported and returns the tick period.
#[test]
fn wasm_sep40_r04_resolution_is_exported() {
    let e = Env::default();
    let client = wasm_oracle(&e, 18);
    client.set_resolution(&300u32);
    let reported: u32 = call(&e, &client.address, "resolution", &[]);
    assert_eq!(reported, 300);
}

/// SEP40-R05 — `lastprice()` is exported and returns the latest aggregate.
#[test]
fn wasm_sep40_r05_lastprice_is_exported() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000_000);
    let client = wasm_oracle(&e, 18);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "S1");
    client.submit_price(&src, &asset, &2_000_000i128, &e.ledger().timestamp());

    let price: crate::PriceData = call(&e, &client.address, "lastprice", &[asset.into_val(&e)]);
    assert_eq!(price.price, 2_000_000);
}

// ---------------------------------------------------------------------------
// Attack-regression pins, against the WASM
// ---------------------------------------------------------------------------

/// `replay` — a replayed nonce is rejected by the WASM build, not just by the
/// native one.
#[test]
fn wasm_attack_replay_nonce_is_rejected() {
    let e = Env::default();
    let client = wasm_oracle(&e, 18);
    client.set_min_sources_required(&1u32);
    let src = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);
    let ts = e.ledger().timestamp();

    client.submit_price_with_nonce(&src, &asset, &1_000i128, &ts, &10u64);
    assert_eq!(
        client.try_submit_price_with_nonce(&src, &asset, &1_000i128, &ts, &10u64),
        Err(Ok(ErrorCode::InvalidNonce.into()))
    );
}

/// `authorization` — an unauthenticated admin call is refused by the WASM.
#[test]
fn wasm_attack_unauthorized_admin_call_is_refused() {
    let e = Env::default();
    let client = wasm_oracle(&e, 18);
    // Drop the blanket auth mock: the caller is now unauthenticated.
    e.set_auths(&[]);
    assert!(client.try_set_resolution(&300u32).is_err());
}
