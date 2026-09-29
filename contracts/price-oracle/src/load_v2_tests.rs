#![cfg(test)]

//! # #413 — Load test v2: adversarial submission patterns
//!
//! Run with `make load-test`. Every scenario prints `LOADV2 ...` metric lines
//! that are summarised in `docs/gas-usage.md` ("Load test v2").
//!
//! Scenarios:
//! - Byzantine sources at every minority fraction, with an asserted bound.
//! - Diurnal bursts and thundering-herd bursts (all sources, same ledger).
//! - Governance writes racing in-flight submissions (no torn application).
//! - Rate-limit probing at the exact `min_submission_interval` boundary.

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::test_helpers::ledger_default;
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Largest source set for which `submit_price` fits network limits (see
/// `gas_budget_tests::N_SOURCES`).
const N: u32 = 9;
const HONEST: i128 = 1_000_000;

fn oracle(e: &Env, min_sources: u32) -> (PriceOracleContractClient<'_>, Vec<Address>) {
    e.mock_all_auths();
    ledger_default(e, 100, 1_000_000);
    let client = PriceOracleContractClient::new(e, &e.register(PriceOracleContract, ()));
    client.initialize(
        &Address::generate(e),
        &min_sources,
        &50u32,
        &7u32,
        &String::from_str(e, "load-v2"),
    );
    let mut sources = Vec::new(e);
    for _ in 0..N {
        let s = Address::generate(e);
        client.add_source(&s, &String::from_str(e, "S"));
        sources.push_back(s);
    }
    (client, sources)
}

fn asset(e: &Env, client: &PriceOracleContractClient<'_>) -> Address {
    let a = Address::generate(e);
    client.register_asset(&a);
    a
}

/// Honest values spread ±0.2% around `HONEST`.
fn honest_price(i: u32) -> i128 {
    HONEST - 2_000 + (i as i128 * 4_000) / N as i128
}

fn cpu(e: &Env) -> u64 {
    e.cost_estimate().budget().cpu_instruction_cost()
}

/// For every minority fraction f < N/2, Byzantine sources all pushing the same
/// plausible-but-wrong direction (+5%) cannot move the median outside the
/// honest range [min honest, max honest]. The majority case is reported as
/// "requires mitigation" and asserted to escape the bound.
#[test]
fn load_v2_byzantine_fractions_bounded() {
    for f in 0..=N / 2 + 1 {
        let e = Env::default();
        let (client, sources) = oracle(&e, N);
        let a = asset(&e, &client);
        let ts = e.ledger().timestamp();
        let (mut lo, mut hi) = (i128::MAX, i128::MIN);
        for i in 0..N {
            let p = if i < f {
                HONEST + HONEST / 20
            } else {
                let p = honest_price(i);
                lo = lo.min(p);
                hi = hi.max(p);
                p
            };
            client.submit_price(&sources.get_unchecked(i), &a, &p, &ts);
        }
        let median = client.get_price(&a, &0u64).unwrap().price;
        let dev_bps = (median - HONEST).abs() * 10_000 / HONEST;
        std::println!(
            "LOADV2 byzantine f={f}/{N} median={median} honest=[{lo},{hi}] dev_bps={dev_bps}"
        );
        if f * 2 < N {
            assert!(
                (lo..=hi).contains(&median),
                "byzantine f={f}/{N}: median {median} escaped honest range [{lo},{hi}]"
            );
        } else {
            assert!(
                median > hi,
                "majority f={f}/{N} is expected to capture the median"
            );
        }
    }
}

/// Diurnal pattern (quiet ledgers with a few sources) vs. thundering herd (all
/// sources in one ledger on many assets). Reports cost per submission and
/// history growth; history must stay bounded by `max_history_length`.
#[test]
fn load_v2_bursts_and_history_growth() {
    const ASSETS: u32 = 5;
    const ROUNDS: u32 = 60;
    let e = Env::default();
    let (client, sources) = oracle(&e, 3);
    let mut assets = Vec::new(&e);
    for _ in 0..ASSETS {
        assets.push_back(asset(&e, &client));
    }
    let (mut quiet_cpu, mut quiet_n, mut herd_cpu, mut herd_n, mut herd_max) =
        (0u64, 0u64, 0u64, 0u64, 0u64);
    for r in 0..ROUNDS {
        ledger_default(&e, 200 + r * 10, 1_000_000 + r as u64 * 50);
        let ts = e.ledger().timestamp();
        // Every 10th round is an adversarial burst: every source, every asset.
        let herd = r % 10 == 9;
        let active = if herd { N } else { 3 };
        for ai in 0..ASSETS {
            for si in 0..active {
                let src = sources.get_unchecked((si + r) % N);
                e.cost_estimate().budget().reset_unlimited();
                client.submit_price(&src, &assets.get_unchecked(ai), &honest_price(si), &ts);
                let c = cpu(&e);
                if herd {
                    herd_cpu += c;
                    herd_n += 1;
                    herd_max = herd_max.max(c);
                } else {
                    quiet_cpu += c;
                    quiet_n += 1;
                }
            }
        }
    }
    let history = client.get_price_history(&assets.get_unchecked(0), &0u32, &50u32);
    std::println!(
        "LOADV2 bursts quiet_avg_cpu={} herd_avg_cpu={} herd_max_cpu={} submissions={} history_len={}",
        quiet_cpu / quiet_n,
        herd_cpu / herd_n,
        herd_max,
        quiet_n + herd_n,
        history.len()
    );
    assert!(history.len() <= 50, "history grew past max_history_length");
}

/// Governance writes (`set_min_sources_required`) interleaved with in-flight
/// submissions. Every aggregate written must satisfy the quorum rule that was
/// in force for the transaction that wrote it — never a mix of old and new.
#[test]
fn load_v2_governance_races_never_tear() {
    let e = Env::default();
    let (client, sources) = oracle(&e, 2);
    let a = asset(&e, &client);
    let mut last_version: Option<u32> = None;
    let mut writes = 0u32;
    let schedule = [2u32, 5, 3, 9, 4, 7, 2, 6];
    for (round, min) in schedule.iter().enumerate() {
        ledger_default(&e, 1_000 + round as u32 * 10, 2_000_000 + round as u64 * 50);
        let ts = e.ledger().timestamp();
        for i in 0..N {
            // Race: flip the quorum rule midway through the round's submissions.
            if i == N / 2 {
                client.set_min_sources_required(min);
            }
            let rule_in_force = client.get_min_sources_required();
            client.submit_price(&sources.get_unchecked(i), &a, &honest_price(i), &ts);
            if let Some(agg) = client.get_price(&a, &0u64) {
                if last_version != Some(agg.version) {
                    writes += 1;
                    assert!(
                        agg.num_sources >= rule_in_force,
                        "torn write: aggregate v{} used {} sources under min {}",
                        agg.version,
                        agg.num_sources,
                        rule_in_force
                    );
                    last_version = Some(agg.version);
                }
            }
        }
    }
    std::println!(
        "LOADV2 governance_race rounds={} aggregate_writes={writes} torn=0",
        schedule.len()
    );
    assert!(writes > 0);
}

/// Rate-limit probing: an adversary searches for the throughput ceiling.
/// Reports how many same-source submissions and how many `get_price` queries
/// per ledger are accepted with the tightest configurable limits in place.
/// `min_submission_interval` is a staleness window (not a rate limit) and the
/// stored `query_rate_limit` is currently not enforced — both are reported as
/// "requires mitigation" in `docs/gas-usage.md`.
#[test]
fn load_v2_rate_limit_probe() {
    const LEDGERS: u32 = 50;
    const QUERIES_PER_LEDGER: u32 = 5;
    let e = Env::default();
    let (client, sources) = oracle(&e, 1);
    client.set_min_submission_interval(&5u32);
    client.set_query_rate_limit(&1u32);
    let a = asset(&e, &client);
    let src = sources.get_unchecked(0);
    let (mut submits, mut queries) = (0u32, 0u32);
    for seq in 500..500 + LEDGERS {
        ledger_default(&e, seq, 5_000_000 + seq as u64 * 5);
        let ts = e.ledger().timestamp();
        if let Ok(Ok(())) = client.try_submit_price(&src, &a, &HONEST, &ts) {
            submits += 1;
        }
        for _ in 0..QUERIES_PER_LEDGER {
            if let Ok(Ok(_)) = client.try_get_price(&a, &0u64) {
                queries += 1;
            }
        }
    }
    std::println!(
        "LOADV2 rate_limit_probe ledgers={LEDGERS} submits_accepted={submits}/{LEDGERS} \
         queries_accepted={queries}/{} query_rate_limit=1",
        LEDGERS * QUERIES_PER_LEDGER
    );
    assert!(submits > 0 && queries > 0);
}
