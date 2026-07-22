//! Gas Usage Tracking for Stellar Unified Price Oracle
//!
//! Measures CPU instruction counts and memory bytes for each public contract
//! function across varying input sizes using the Soroban test environment
//! budget API (`env.budget()`).
//!
//! Run with:
//!   cargo test -p price-oracle --lib gas_tracking -- --nocapture

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Budget, Ledger, LedgerInfo},
    Address, Env, String,
};

use crate::{PriceOracleContract, PriceOracleContractClient};

// ── helpers ───────────────────────────────────────────────────────────────────

fn new_env() -> Env {
    let e = Env::default();
    e.mock_all_auths();
    e
}

fn deploy(e: &Env) -> PriceOracleContractClient<'_> {
    let id = e.register(PriceOracleContract, ());
    PriceOracleContractClient::new(e, &id)
}

fn init(e: &Env, client: &PriceOracleContractClient<'_>, min_src: u32, max_hist: u32) {
    let admin = Address::generate(e);
    client.initialize(
        &admin,
        &min_src,
        &max_hist,
        &18u32,
        &String::from_str(e, "test"),
    );
}

fn set_ledger(e: &Env, seq: u32, ts: u64) {
    e.ledger().set(LedgerInfo {
        timestamp: ts,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 4_096,
    });
}

/// Returns (cpu_instructions, memory_bytes) consumed by `f`.
fn measure<F: FnOnce()>(e: &Env, f: F) -> (u64, u64) {
    e.budget().reset_default();
    f();
    (
        e.budget().cpu_instruction_count(),
        e.budget().memory_bytes_count(),
    )
}

fn row(fn_name: &str, variant: &str, cpu: u64, mem: u64) {
    println!(
        "{:<38} {:<28} {:>14} {:>14}",
        fn_name, variant, cpu, mem
    );
}

fn header() {
    println!();
    println!(
        "{:<38} {:<28} {:>14} {:>14}",
        "Function", "Variant", "CPU (instr.)", "Mem (bytes)"
    );
    println!("{}", "-".repeat(98));
}

// ── benchmarks ────────────────────────────────────────────────────────────────

fn bench_initialize(e: &Env, client: &PriceOracleContractClient<'_>) {
    let admin = Address::generate(e);
    let (cpu, mem) = measure(e, || {
        client.initialize(
            &admin,
            &2u32,
            &100u32,
            &18u32,
            &String::from_str(e, "Stellar Price Oracle Aggregator"),
        );
    });
    row("initialize", "default params", cpu, mem);
}

fn bench_add_source(e: &Env, client: &PriceOracleContractClient<'_>, existing: u32) {
    for i in 0..existing {
        let src = Address::generate(e);
        client.add_source(&src, &String::from_str(e, "src"));
        let _ = i;
    }
    let new_src = Address::generate(e);
    let (cpu, mem) = measure(e, || {
        client.add_source(&new_src, &String::from_str(e, "new"));
    });
    row(
        "add_source",
        &alloc_label("after ", existing, " existing"),
        cpu,
        mem,
    );
}

fn bench_register_asset(e: &Env, client: &PriceOracleContractClient<'_>) {
    let asset = Address::generate(e);
    let (cpu, mem) = measure(e, || {
        client.register_asset(&asset);
    });
    row("register_asset", "1 asset", cpu, mem);
}

fn bench_submit_price(e: &Env, client: &PriceOracleContractClient<'_>, n_sources: u32) {
    set_ledger(e, 1, 1_000_000);
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&1u32);

    // Register all sources; submit prices from all but the last.
    let mut sources: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(e);
    for _ in 0..n_sources {
        let src = Address::generate(e);
        client.add_source(&src, &String::from_str(e, "src"));
        sources.push_back(src);
    }
    for i in 0..n_sources.saturating_sub(1) {
        let src = sources.get(i).unwrap();
        client.submit_price(&src, &asset, &(65_000 * 10_i128.pow(7)), &1_000_000u64);
    }
    let last = sources.get(n_sources - 1).unwrap();
    let (cpu, mem) = measure(e, || {
        client.submit_price(&last, &asset, &(65_001 * 10_i128.pow(7)), &1_000_001u64);
    });
    row(
        "submit_price",
        &alloc_label("", n_sources, " sources"),
        cpu,
        mem,
    );
}

fn bench_get_price(e: &Env, client: &PriceOracleContractClient<'_>, n_sources: u32) {
    set_ledger(e, 1, 1_000_000);
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&n_sources);
    for i in 0..n_sources {
        let src = Address::generate(e);
        client.add_source(&src, &String::from_str(e, "src"));
        client.submit_price(
            &src,
            &asset,
            &((65_000 + i as i128) * 10_i128.pow(7)),
            &1_000_000u64,
        );
    }
    let (cpu, mem) = measure(e, || {
        client.get_price(&asset);
    });
    row(
        "get_price",
        &alloc_label("", n_sources, " sources"),
        cpu,
        mem,
    );
}

fn bench_get_all_prices(e: &Env, client: &PriceOracleContractClient<'_>, n_sources: u32) {
    set_ledger(e, 1, 1_000_000);
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&1u32);
    for i in 0..n_sources {
        let src = Address::generate(e);
        client.add_source(&src, &String::from_str(e, "src"));
        client.submit_price(
            &src,
            &asset,
            &((65_000 + i as i128) * 10_i128.pow(7)),
            &1_000_000u64,
        );
    }
    let (cpu, mem) = measure(e, || {
        client.get_all_prices(&asset);
    });
    row(
        "get_all_prices",
        &alloc_label("", n_sources, " sources"),
        cpu,
        mem,
    );
}

fn bench_get_historical_price(e: &Env, client: &PriceOracleContractClient<'_>, n_history: u32) {
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&1u32);
    let src = Address::generate(e);
    client.add_source(&src, &String::from_str(e, "src"));
    for i in 0..n_history {
        set_ledger(e, i + 1, 1_000_000 + i as u64);
        client.submit_price(
            &src,
            &asset,
            &((65_000 + i as i128) * 10_i128.pow(7)),
            &(1_000_000 + i as u64),
        );
    }
    let (cpu, mem) = measure(e, || {
        client.get_historical_price(&asset, &1u32);
    });
    row(
        "get_historical_price",
        &alloc_label("", n_history, " history entries"),
        cpu,
        mem,
    );
}

fn bench_upgrade(e: &Env, client: &PriceOracleContractClient<'_>) {
    // Upload a minimal valid WASM to the env so upgrade() can succeed.
    // The smallest valid Soroban WASM is an empty contract; use the same
    // contract WASM that is already registered in this test environment.
    // We measure the budget for the dispatch/auth overhead by uploading
    // the contract's own WASM bytes and using the resulting hash.
    let wasm: &[u8] = include_bytes!(
        "../../../../target/wasm32v1-none/release/price_oracle.wasm"
    );
    let wasm_hash = e.deployer().upload_contract_wasm(wasm);
    let (cpu, mem) = measure(e, || {
        client.upgrade(&wasm_hash);
    });
    row("upgrade", "same wasm", cpu, mem);
}

/// Simple no-alloc label builder for test output (uses a fixed-size buffer on stack).
fn alloc_label(prefix: &str, n: u32, suffix: &str) -> &'static str {
    // We're in a test (std available). Leak a small string for the label.
    let s = std::format!("{}{}{}", prefix, n, suffix);
    Box::leak(s.into_boxed_str())
}

// ── entry-point test ──────────────────────────────────────────────────────────

/// Runs all gas benchmarks and prints a formatted table.
///
/// Usage:
///   cargo test -p price-oracle --lib gas_tracking::gas_report -- --nocapture
#[test]
fn gas_report() {
    header();

    // initialize
    {
        let e = new_env();
        let c = deploy(&e);
        bench_initialize(&e, &c);
    }

    // add_source: 0 existing, 10 existing, 49 existing (→ 50th add)
    for existing in [0u32, 10, 49] {
        let e = new_env();
        let c = deploy(&e);
        init(&e, &c, 1, 200);
        bench_add_source(&e, &c, existing);
    }

    // register_asset
    {
        let e = new_env();
        let c = deploy(&e);
        init(&e, &c, 1, 100);
        bench_register_asset(&e, &c);
    }

    // submit_price: 1, 10, 50 sources
    for n in [1u32, 10, 50] {
        let e = new_env();
        let c = deploy(&e);
        init(&e, &c, 1, 200);
        bench_submit_price(&e, &c, n);
    }

    // get_price: 1, 10, 50 sources
    for n in [1u32, 10, 50] {
        let e = new_env();
        let c = deploy(&e);
        init(&e, &c, n, 200);
        bench_get_price(&e, &c, n);
    }

    // get_all_prices: 1, 10, 50 sources
    for n in [1u32, 10, 50] {
        let e = new_env();
        let c = deploy(&e);
        init(&e, &c, 1, 200);
        bench_get_all_prices(&e, &c, n);
    }

    // get_historical_price: 10, 50, 100 history entries
    for n in [10u32, 50, 100] {
        let e = new_env();
        let c = deploy(&e);
        init(&e, &c, 1, 200);
        bench_get_historical_price(&e, &c, n);
    }

    // upgrade (requires compiled wasm; skip gracefully if not built)
    // Uncomment after running `make build` first:
    // {
    //     let e = new_env();
    //     let c = deploy(&e);
    //     init(&e, &c, 1, 100);
    //     bench_upgrade(&e, &c);
    // }

    println!();
}
