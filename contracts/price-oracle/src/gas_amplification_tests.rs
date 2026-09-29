#![cfg(test)]

//! # #417 — Gas cost attribution and amplification
//!
//! Measures CPU cost per endpoint and per caller, and the *amplification
//! ratio* of every core state-modifying endpoint:
//!
//! ```text
//! amplification = cost imposed on others / cost paid by the caller
//! ```
//!
//! "Cost imposed" is the extra CPU a fixed victim workload (an honest source's
//! `submit_price` plus a consumer's `get_price`) pays after the caller's action,
//! compared with the same workload measured just before it.
//!
//! Every measurement is printed as a `GAS_SAMPLE,` line consumed by
//! `scripts/gas_dashboard.py`:
//!
//! ```text
//! cargo test -p price-oracle --lib gas_amplification -- --nocapture --test-threads=1 \
//!   | python3 scripts/gas_dashboard.py
//! ```
//!
//! See `docs/gas-dashboard.md`.

extern crate std;

use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    Address, Env, String,
};
use std::{println, vec::Vec as StdVec};

use crate::test_helpers::*;
use crate::PriceOracleContractClient;

/// Ratio above which a caller is considered to be amplifying cost.
const AMPLIFICATION_THRESHOLD: f64 = 1.0;
/// Consecutive windows above the threshold needed to flag a caller.
const SUSTAINED_WINDOWS: usize = 3;
/// Samples per endpoint. Each round admits one source, and every admitted
/// source widens the submit footprint (see `docs/gas-dashboard.md`).
const ROUNDS: u32 = 12;

fn set_ledger(e: &Env, seq: u32) {
    e.ledger().set(LedgerInfo {
        timestamp: 1_000 + seq as u64,
        protocol_version: 26,
        sequence_number: seq,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 10,
        min_persistent_entry_ttl: 10,
        max_entry_ttl: 6_312_000,
    });
}

/// Returns the CPU instructions consumed by `f`.
fn cpu<F: FnOnce()>(e: &Env, f: F) -> u64 {
    e.cost_estimate().budget().reset_default();
    f();
    e.cost_estimate().budget().cpu_instruction_cost()
}

/// Amplification ratio: cost imposed on others divided by cost paid.
pub fn amplification_ratio(imposed: u64, paid: u64) -> f64 {
    if paid == 0 {
        return f64::INFINITY;
    }
    imposed as f64 / paid as f64
}

/// Returns the callers whose ratio stayed above `threshold` for at least
/// `windows` consecutive samples. Raw totals are deliberately ignored: a
/// caller paying a lot but imposing little is never flagged.
pub fn sustained_amplifiers<'a>(
    samples: &[(&'a str, u64, u64)],
    threshold: f64,
    windows: usize,
) -> StdVec<&'a str> {
    let mut flagged: StdVec<&str> = StdVec::new();
    let mut callers: StdVec<&str> = samples.iter().map(|s| s.0).collect();
    callers.dedup();
    for caller in callers {
        let mut run = 0usize;
        for (c, paid, imposed) in samples {
            if *c != caller {
                continue;
            }
            run = if amplification_ratio(*imposed, *paid) > threshold {
                run + 1
            } else {
                0
            };
            if run >= windows && !flagged.contains(&caller) {
                flagged.push(caller);
            }
        }
    }
    flagged
}

fn emit(endpoint: &str, caller: &str, paid: u64, imposed: u64) {
    println!("GAS_SAMPLE,{endpoint},{caller},{paid},{imposed}");
}

struct Victim {
    source: Address,
    asset: Address,
}

/// Cost of the fixed victim workload at the current ledger.
fn victim_cost(e: &Env, client: &PriceOracleContractClient<'_>, v: &Victim, seq: u32) -> u64 {
    set_ledger(e, seq);
    let ts = 1_000 + seq as u64;
    cpu(e, || {
        client.submit_price(&v.source, &v.asset, &1_000i128, &ts)
    }) + cpu(e, || {
        client.get_price(&v.asset, &0u64);
    })
}

/// Measures (paid, imposed) for `action` against the victim workload.
fn measure_action<F: FnOnce()>(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    v: &Victim,
    seq: &mut u32,
    action: F,
) -> (u64, u64) {
    let before = victim_cost(e, client, v, *seq);
    *seq += 1;
    set_ledger(e, *seq);
    let paid = cpu(e, action);
    *seq += 1;
    let after = victim_cost(e, client, v, *seq);
    *seq += 1;
    (paid, after.saturating_sub(before))
}

#[test]
fn gas_amplification_state_modifying_endpoints() {
    let e = Env::default();
    let (client, _admin, source, asset) = setup_basic(&e);
    let v = Victim { source, asset };
    let mut seq = 100u32;
    client.set_finality_ledgers(&1u32);

    // submit_price from a second admitted source on the victim's asset.
    let other = register_test_source(&e, &client, "Other");
    for round in 0..ROUNDS {
        let ts = 1_000 + seq as u64 + 1;
        let (paid, imposed) = measure_action(&e, &client, &v, &mut seq, || {
            client.submit_price(&other, &v.asset, &(1_000 + round as i128), &ts)
        });
        emit("submit_price", "source", paid, imposed);

        let new_src = Address::generate(&e);
        let (paid, imposed) = measure_action(&e, &client, &v, &mut seq, || {
            client.add_source(&new_src, &String::from_str(&e, "S"));
        });
        emit("add_source", "admin", paid, imposed);

        let new_asset = Address::generate(&e);
        let (paid, imposed) = measure_action(&e, &client, &v, &mut seq, || {
            client.register_asset(&new_asset);
        });
        emit("register_asset", "admin", paid, imposed);

        let (paid, imposed) = measure_action(&e, &client, &v, &mut seq, || {
            client.mark_price_pending(&v.asset);
        });
        emit("mark_price_pending", "anyone", paid, imposed);
        let committed = seq - 2;

        let (paid, imposed) = measure_action(&e, &client, &v, &mut seq, || {
            client.try_finalize_price(&v.asset, &committed);
        });
        emit("try_finalize_price", "anyone", paid, imposed);
    }
}

#[test]
fn gas_amplification_ratio_definition() {
    assert_eq!(amplification_ratio(300, 100), 3.0);
    assert_eq!(amplification_ratio(0, 100), 0.0);
    assert!(amplification_ratio(1, 0).is_infinite());
}

#[test]
fn gas_amplification_synthetic_high_amplifier_is_flagged() {
    // "whale" pays far more in total but imposes little on others;
    // "griefer" pays little per call and imposes 50x that on others.
    let mut samples: StdVec<(&str, u64, u64)> = StdVec::new();
    for _ in 0..10 {
        samples.push(("whale", 5_000_000, 100_000));
        samples.push(("griefer", 10_000, 500_000));
    }
    // A single spike is noise, not a sustained amplifier.
    samples.push(("spiky", 10_000, 500_000));
    samples.push(("spiky", 10_000, 0));

    let flagged = sustained_amplifiers(&samples, AMPLIFICATION_THRESHOLD, SUSTAINED_WINDOWS);
    assert_eq!(flagged, std::vec!["griefer"]);
}
