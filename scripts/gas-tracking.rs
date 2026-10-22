//! Gas Usage Tracking for Stellar Unified Price Oracle
//!
//! This module measures CPU and memory instruction counts for each public contract
//! function across varying input sizes using the Soroban test environment's built-in
//! budget tracking (`env.budget()`).
//!
//! Run with:
//!   cargo test -p price-oracle --lib gas_tracking -- --nocapture
//!
//! Output is printed as a formatted table to stdout.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Budget, Ledger, LedgerInfo},
    Address, Bytes, Env, String,
};

use crate::{PriceOracleContract, PriceOracleContractClient};

// ── helpers ──────────────────────────────────────────────────────────────────

fn new_env() -> Env {
    let e = Env::default();
    e.mock_all_auths();
    e
}

fn deploy(e: &Env) -> PriceOracleContractClient<'_> {
    let id = e.register(PriceOracleContract, ());
    PriceOracleContractClient::new(e, &id)
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

/// Measure CPU instructions and memory bytes consumed by `f`.
fn measure<F: FnOnce()>(e: &Env, f: F) -> (u64, u64) {
    e.budget().reset_default();
    f();
    (e.budget().cpu_instruction_count(), e.budget().memory_bytes_count())
}

fn print_row(fn_name: &str, variant: &str, cpu: u64, mem: u64) {
    println!("{:<35} {:<25} {:>14} {:>14}", fn_name, variant, cpu, mem);
}

fn print_header() {
    println!();
    println!(
        "{:<35} {:<25} {:>14} {:>14}",
        "Function", "Variant", "CPU (instr.)", "Mem (bytes)"
    );
    println!("{}", "-".repeat(92));
}

// ── individual function benchmarks ───────────────────────────────────────────

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
    print_row("initialize", "default params", cpu, mem);
}

fn bench_add_source(e: &Env, client: &PriceOracleContractClient<'_>, n: u32) {
    // Pre-populate n-1 sources, then measure the nth add.
    for i in 0..(n - 1) {
        let src = Address::generate(e);
        let name = soroban_sdk::String::from_str(e, &alloc_str(i));
        client.add_source(&src, &name);
    }
    let new_src = Address::generate(e);
    let (cpu, mem) = measure(e, || {
        client.add_source(&new_src, &String::from_str(e, "NewSource"));
    });
    print_row(
        "add_source",
        &format!("after {} sources", n - 1),
        cpu,
        mem,
    );
}

fn bench_register_asset(e: &Env, client: &PriceOracleContractClient<'_>) {
    let asset = Address::generate(e);
    let (cpu, mem) = measure(e, || {
        client.register_asset(&asset);
    });
    print_row("register_asset", "1 asset", cpu, mem);
}

fn bench_submit_price(e: &Env, client: &PriceOracleContractClient<'_>, n_sources: u32) {
    set_ledger(e, 1, 1_000_000);
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&1u32);

    let mut sources = soroban_sdk::Vec::new(e);
    for i in 0..n_sources {
        let src = Address::generate(e);
        client.add_source(&src, &String::from_str(e, &alloc_str(i)));
        sources.push_back(src);
    }
    // Warm: submit from all but last source so aggregation is ready.
    for i in 0..(n_sources.saturating_sub(1)) {
        let src = sources.get(i).unwrap();
        client.submit_price(&src, &asset, &(65_000 * 10_i128.pow(7)), &1_000_000u64);
    }
    let last_src = sources.get(n_sources - 1).unwrap();
    let (cpu, mem) = measure(e, || {
        client.submit_price(&last_src, &asset, &(65_001 * 10_i128.pow(7)), &1_000_001u64);
    });
    print_row(
        "submit_price",
        &format!("{} sources", n_sources),
        cpu,
        mem,
    );
}

fn bench_get_price(e: &Env, client: &PriceOracleContractClient<'_>, n_sources: u32) {
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&n_sources);
    for i in 0..n_sources {
        let src = Address::generate(e);
        client.add_source(&src, &String::from_str(e, &alloc_str(i)));
        client.submit_price(&src, &asset, &((65_000 + i as i128) * 10_i128.pow(7)), &1_000_000u64);
    }
    let (cpu, mem) = measure(e, || {
        client.get_price(&asset);
    });
    print_row("get_price", &format!("{} sources", n_sources), cpu, mem);
}

fn bench_get_all_prices(e: &Env, client: &PriceOracleContractClient<'_>, n_sources: u32) {
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&1u32);
    for i in 0..n_sources {
        let src = Address::generate(e);
        client.add_source(&src, &String::from_str(e, &alloc_str(i)));
        client.submit_price(&src, &asset, &((65_000 + i as i128) * 10_i128.pow(7)), &1_000_000u64);
    }
    let (cpu, mem) = measure(e, || {
        client.get_all_prices(&asset);
    });
    print_row(
        "get_all_prices",
        &format!("{} sources", n_sources),
        cpu,
        mem,
    );
}

fn bench_get_historical_price(e: &Env, client: &PriceOracleContractClient<'_>, n_history: u32) {
    let asset = Address::generate(e);
    client.register_asset(&asset);
    client.set_min_sources_required(&1u32);
    let src = Address::generate(e);
    client.add_source(&src, &String::from_str(e, "HistSrc"));

    for i in 0..n_history {
        set_ledger(e, i + 1, 1_000_000 + i as u64);
        client.submit_price(&src, &asset, &((65_000 + i as i128) * 10_i128.pow(7)), &(1_000_000 + i as u64));
    }
    let target_ledger = 1u32;
    let (cpu, mem) = measure(e, || {
        client.get_historical_price(&asset, &target_ledger);
    });
    print_row(
        "get_historical_price",
        &format!("{} history entries", n_history),
        cpu,
        mem,
    );
}

fn bench_upgrade(e: &Env, client: &PriceOracleContractClient<'_>) {
    // Use a zeroed wasm hash as a stand-in; the test env accepts any hash.
    let wasm_hash = soroban_sdk::BytesN::from_array(e, &[0u8; 32]);
    let (cpu, mem) = measure(e, || {
        // upgrade will panic if the wasm isn't uploaded, so we catch the budget before
        // the storage write and just measure the auth + lookup overhead.
        let _ = core::panic::catch_unwind(core::panic::AssertUnwindSafe(|| {
            client.upgrade(&wasm_hash);
        }));
    });
    print_row("upgrade", "zeroed wasm hash", cpu, mem);
}

/// Cheap no-alloc string helper that returns a fixed-length label.
fn alloc_str(i: u32) -> soroban_sdk::xdr::ScString {
    // We can't use std alloc in no_std, but this module is #[cfg(test)] which
    // runs under std. Use a simple fixed buffer.
    let _ = i; // label reuse is fine for gas benchmarks
    soroban_sdk::xdr::ScString::default()
}

// ── entry-point test ──────────────────────────────────────────────────────────

/// Runs all gas benchmarks and prints results as a table.
///
/// Execute with:
///   cargo test -p price-oracle --lib gas_tracking::tests::gas_report -- --nocapture
#[test]
fn gas_report() {
    print_header();

    // ── initialize ──
    {
        let e = new_env();
        let client = deploy(&e);
        bench_initialize(&e, &client);
    }

    // ── add_source: 1 source, 10 sources, 50 sources ──
    for n in [1u32, 10, 50] {
        let e = new_env();
        let client = deploy(&e);
        let admin = Address::generate(&e);
        client.initialize(
            &admin,
            &1u32,
            &200u32,
            &18u32,
            &String::from_str(&e, "test"),
        );
        bench_add_source(&e, &client, n);
    }

    // ── register_asset ──
    {
        let e = new_env();
        let client = deploy(&e);
        let admin = Address::generate(&e);
        client.initialize(&admin, &1u32, &100u32, &18u32, &String::from_str(&e, "test"));
        bench_register_asset(&e, &client);
    }

    // ── submit_price: 1, 10, 50 sources ──
    for n in [1u32, 10, 50] {
        let e = new_env();
        let client = deploy(&e);
        let admin = Address::generate(&e);
        client.initialize(&admin, &1u32, &200u32, &18u32, &String::from_str(&e, "test"));
        bench_submit_price(&e, &client, n);
    }

    // ── get_price: 1, 10, 50 sources ──
    for n in [1u32, 10, 50] {
        let e = new_env();
        let client = deploy(&e);
        let admin = Address::generate(&e);
        client.initialize(&admin, &n, &200u32, &18u32, &String::from_str(&e, "test"));
        set_ledger(&e, 1, 1_000_000);
        bench_get_price(&e, &client, n);
    }

    // ── get_all_prices: 1, 10, 50 sources ──
    for n in [1u32, 10, 50] {
        let e = new_env();
        let client = deploy(&e);
        let admin = Address::generate(&e);
        client.initialize(&admin, &1u32, &200u32, &18u32, &String::from_str(&e, "test"));
        set_ledger(&e, 1, 1_000_000);
        bench_get_all_prices(&e, &client, n);
    }

    // ── get_historical_price: 10, 50, 100 history entries ──
    for n in [10u32, 50, 100] {
        let e = new_env();
        let client = deploy(&e);
        let admin = Address::generate(&e);
        client.initialize(&admin, &1u32, &200u32, &18u32, &String::from_str(&e, "test"));
        bench_get_historical_price(&e, &client, n);
    }

    // ── upgrade ──
    {
        let e = new_env();
        let client = deploy(&e);
        let admin = Address::generate(&e);
        client.initialize(&admin, &1u32, &100u32, &18u32, &String::from_str(&e, "test"));
        bench_upgrade(&e, &client);
    }

    println!();
}
