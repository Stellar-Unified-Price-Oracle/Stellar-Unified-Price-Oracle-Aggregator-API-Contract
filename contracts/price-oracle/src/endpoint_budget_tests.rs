#![cfg(test)]

//! # #692 — Per-endpoint production compute and storage budgets
//!
//! Every public endpoint of the production surface (`docs/endpoint-budgets.md`)
//! is measured under a **representative** input (3 sources, short history) and
//! a **boundary** input (10 sources — the supported maximum — and history at
//! `max_history_length`). Three dimensions are measured per call:
//!
//! * CPU instructions,
//! * memory bytes,
//! * storage growth — the change in retained history bytes reported by
//!   `get_total_storage_budget`.
//!
//! Each measurement is compared with the committed budget in `BUDGETS` plus
//! `TOLERANCE_PCT`. A regression fails `make endpoint-budgets` / CI with the
//! endpoint, input, dimension, measured value, budget and delta.

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::test_helpers::ledger_default;
use crate::types::Asset;
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Allowed growth over a committed budget before the gate fails.
const TOLERANCE_PCT: u64 = 5;

/// `max_history_length` used by every fixture; the boundary input fills it.
const MAX_HISTORY: u32 = 10;

/// (input name, sources, history entries seeded before the measurement).
const INPUTS: &[(&str, u32, u32)] = &[("representative", 3, 3), ("boundary", 10, MAX_HISTORY)];

/// Committed budgets: (endpoint, cpu instructions, memory bytes, storage bytes).
/// Each budget covers every input in `INPUTS`. Update only via the process in
/// `docs/endpoint-budgets.md`.
const BUDGETS: &[(&str, u64, u64, u64)] = &[
    // Writes
    ("initialize", 10_000_000, 5_000_000, 0),
    ("add_source", 10_000_000, 5_000_000, 0),
    ("remove_source", 25_000_000, 10_000_000, 0),
    ("register_asset", 10_000_000, 5_000_000, 0),
    ("submit_price", 25_000_000, 10_000_000, 96),
    ("submit_prices", 25_000_000, 10_000_000, 96),
    ("trigger_aggregation", 25_000_000, 10_000_000, 96),
    ("prune_history", 25_000_000, 10_000_000, 0),
    // Reads
    ("get_price", 10_000_000, 5_000_000, 0),
    ("get_all_prices", 10_000_000, 5_000_000, 0),
    ("get_historical_price", 10_000_000, 5_000_000, 0),
    ("get_historical_prices_paginated", 10_000_000, 5_000_000, 0),
    ("lastprice", 10_000_000, 5_000_000, 0),
    ("price", 10_000_000, 5_000_000, 0),
    ("prices", 10_000_000, 5_000_000, 0),
    ("assets", 5_000_000, 3_000_000, 0),
    ("decimals", 5_000_000, 3_000_000, 0),
    ("resolution", 5_000_000, 3_000_000, 0),
    ("base", 5_000_000, 3_000_000, 0),
];

/// One measurement of one endpoint under one input.
#[derive(Clone, Copy, Debug)]
struct Sample {
    endpoint: &'static str,
    input: &'static str,
    cpu: u64,
    mem: u64,
    storage: u64,
}

fn budget_for(endpoint: &str) -> (u64, u64, u64) {
    BUDGETS
        .iter()
        .find(|(e, _, _, _)| *e == endpoint)
        .map(|(_, c, m, s)| (*c, *m, *s))
        .unwrap_or_else(|| panic!("no committed budget for endpoint={endpoint}"))
}

/// Returns a regression report for every dimension of `s` over budget + tolerance.
fn regressions(s: &Sample) -> std::vec::Vec<std::string::String> {
    let (cpu, mem, storage) = budget_for(s.endpoint);
    let mut out = std::vec::Vec::new();
    for (dim, measured, budget) in [
        ("cpu", s.cpu, cpu),
        ("mem", s.mem, mem),
        ("storage", s.storage, storage),
    ] {
        let ceiling = budget + budget * TOLERANCE_PCT / 100;
        if measured > ceiling {
            out.push(std::format!(
                "ENDPOINT BUDGET EXCEEDED endpoint={} input={} dimension={dim} measured={measured} \
                 budget={budget} ceiling={ceiling} delta=+{} (+{}%)",
                s.endpoint,
                s.input,
                measured - budget,
                (measured - budget) * 100 / budget.max(1),
            ));
        }
    }
    out
}

fn retained_bytes(client: &PriceOracleContractClient<'_>) -> u64 {
    client.get_total_storage_budget().total_estimated_bytes as u64
}

/// Measures one top-level invocation: (cpu, mem, storage growth in bytes).
fn measure<F: FnOnce()>(e: &Env, client: &PriceOracleContractClient<'_>, f: F) -> (u64, u64, u64) {
    let before = retained_bytes(client);
    e.cost_estimate().budget().reset_unlimited();
    f();
    let budget = e.cost_estimate().budget();
    let (cpu, mem) = (budget.cpu_instruction_cost(), budget.memory_bytes_cost());
    let after = retained_bytes(client);
    (cpu, mem, after.saturating_sub(before))
}

struct Fixture<'a> {
    e: &'a Env,
    client: PriceOracleContractClient<'a>,
    sources: Vec<Address>,
    asset: Address,
    /// Ledger of the most recent seeded history entry.
    last_ledger: u32,
    /// Timestamp of the oldest seeded history entry.
    oldest_ts: u64,
}

fn price_of(i: u32) -> i128 {
    1_000_000 + i as i128
}

/// Initialized oracle with `n_sources` sources, one asset and `history`
/// aggregated history entries (one per ledger, every source submitting).
fn fixture(e: &Env, n_sources: u32, history: u32) -> Fixture<'_> {
    e.mock_all_auths();
    e.cost_estimate().budget().reset_unlimited();
    ledger_default(e, 100, 1_000_000);
    let client = PriceOracleContractClient::new(e, &e.register(PriceOracleContract, ()));
    client.initialize(
        &Address::generate(e),
        &2u32,
        &MAX_HISTORY,
        &7u32,
        &String::from_str(e, "endpoint-budgets"),
    );
    let mut sources = Vec::new(e);
    for _ in 0..n_sources {
        let s = Address::generate(e);
        client.add_source(&s, &String::from_str(e, "S"));
        sources.push_back(s);
    }
    let asset = Address::generate(e);
    client.register_asset(&asset);

    let mut last_ledger = 100;
    let mut oldest_ts = 0;
    for h in 0..history {
        last_ledger = 101 + h;
        let ts = 1_000_000 + (h as u64 + 1) * 5;
        if h == 0 {
            oldest_ts = ts;
        }
        ledger_default(e, last_ledger, ts);
        for i in 0..n_sources {
            client.submit_price(&sources.get_unchecked(i), &asset, &price_of(i), &ts);
        }
    }
    Fixture {
        e,
        client,
        sources,
        asset,
        last_ledger,
        oldest_ts,
    }
}

impl Fixture<'_> {
    /// Moves to the next ledger and lets every source but the last submit, so
    /// the next submission from the last source completes a full round.
    fn open_round(&self) -> u64 {
        let ts = 1_000_000 + (self.last_ledger as u64 + 1 - 100) * 5;
        ledger_default(self.e, self.last_ledger + 1, ts);
        let n = self.sources.len();
        for i in 0..n - 1 {
            self.client.submit_price(
                &self.sources.get_unchecked(i),
                &self.asset,
                &price_of(i),
                &ts,
            );
        }
        ts
    }

    fn last_source(&self) -> Address {
        self.sources.get_unchecked(self.sources.len() - 1)
    }
}

/// Runs every endpoint under every input and returns all samples.
fn measure_all() -> std::vec::Vec<Sample> {
    let mut out = std::vec::Vec::new();
    for (input, n_sources, history) in INPUTS {
        let (input, n_sources, history) = (*input, *n_sources, *history);
        let mut push = |endpoint: &'static str, (cpu, mem, storage): (u64, u64, u64)| {
            out.push(Sample {
                endpoint,
                input,
                cpu,
                mem,
                storage,
            });
        };

        // initialize — on a fresh, uninitialized contract.
        {
            let e = Env::default();
            e.mock_all_auths();
            let client = PriceOracleContractClient::new(&e, &e.register(PriceOracleContract, ()));
            let admin = Address::generate(&e);
            let desc = String::from_str(&e, "endpoint-budgets");
            e.cost_estimate().budget().reset_unlimited();
            client.initialize(&admin, &2u32, &MAX_HISTORY, &7u32, &desc);
            let budget = e.cost_estimate().budget();
            push(
                "initialize",
                (budget.cpu_instruction_cost(), budget.memory_bytes_cost(), 0),
            );
        }

        // Read-only endpoints share one fixture.
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            let c = &f.client;
            let sep40 = Asset::Stellar(f.asset.clone());
            push(
                "get_price",
                measure(&e, c, || {
                    c.get_price(&f.asset, &0u64);
                }),
            );
            push(
                "get_all_prices",
                measure(&e, c, || {
                    c.get_all_prices(&f.asset);
                }),
            );
            push(
                "get_historical_price",
                measure(&e, c, || {
                    c.get_historical_price(&f.asset, &f.last_ledger);
                }),
            );
            push(
                "get_historical_prices_paginated",
                measure(&e, c, || {
                    c.get_historical_prices_paginated(&f.asset, &0u32, &50u32);
                }),
            );
            push(
                "lastprice",
                measure(&e, c, || {
                    c.lastprice(&sep40);
                }),
            );
            push(
                "price",
                measure(&e, c, || {
                    c.price(&sep40, &f.oldest_ts);
                }),
            );
            push(
                "prices",
                measure(&e, c, || {
                    c.prices(&sep40, &history);
                }),
            );
            push(
                "assets",
                measure(&e, c, || {
                    c.assets();
                }),
            );
            push(
                "decimals",
                measure(&e, c, || {
                    c.decimals();
                }),
            );
            push(
                "resolution",
                measure(&e, c, || {
                    c.resolution();
                }),
            );
            push(
                "base",
                measure(&e, c, || {
                    c.base();
                }),
            );
        }

        // Every write endpoint gets a fresh fixture so measurements are independent.
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            let s = Address::generate(&e);
            let name = String::from_str(&e, "S");
            push(
                "add_source",
                measure(&e, &f.client, || f.client.add_source(&s, &name)),
            );
        }
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            let s = f.last_source();
            push(
                "remove_source",
                measure(&e, &f.client, || f.client.remove_source(&s)),
            );
        }
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            let a = Address::generate(&e);
            push(
                "register_asset",
                measure(&e, &f.client, || f.client.register_asset(&a)),
            );
        }
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            let ts = f.open_round();
            let s = f.last_source();
            let p = price_of(n_sources - 1);
            push(
                "submit_price",
                measure(&e, &f.client, || {
                    f.client.submit_price(&s, &f.asset, &p, &ts)
                }),
            );
        }
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            let ts = f.open_round();
            let s = f.last_source();
            let mut batch = Vec::new(&e);
            batch.push_back((f.asset.clone(), price_of(n_sources - 1), ts));
            push(
                "submit_prices",
                measure(&e, &f.client, || f.client.submit_prices(&s, &batch)),
            );
        }
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            push(
                "trigger_aggregation",
                measure(&e, &f.client, || f.client.trigger_aggregation(&f.asset)),
            );
        }
        {
            let e = Env::default();
            let f = fixture(&e, n_sources, history);
            push(
                "prune_history",
                measure(&e, &f.client, || {
                    f.client.prune_history(&f.asset, &1u32);
                }),
            );
        }
    }
    out
}

/// The CI gate: every endpoint, under every input, within its committed
/// CPU, memory and storage budget. Every committed budget must be exercised by
/// every input, so an endpoint cannot silently drop out of the gate.
#[test]
fn endpoint_budgets_within_committed_budgets() {
    let samples = measure_all();
    for (endpoint, _, _, _) in BUDGETS {
        for (input, _, _) in INPUTS {
            assert!(
                samples
                    .iter()
                    .any(|s| s.endpoint == *endpoint && s.input == *input),
                "endpoint={endpoint} input={input} has a budget but was not measured"
            );
        }
    }
    let mut failures = std::vec::Vec::new();
    for s in &samples {
        std::println!(
            "ENDPOINT_BUDGET endpoint={} input={} cpu={} mem={} storage={}",
            s.endpoint,
            s.input,
            s.cpu,
            s.mem,
            s.storage
        );
        failures.extend(regressions(s));
    }
    assert!(
        failures.is_empty(),
        "{} endpoint budget regression(s); update process in docs/endpoint-budgets.md:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

/// The gate must reject an inflated measurement with an actionable message, so
/// it cannot silently become a no-op.
#[test]
fn endpoint_budget_gate_rejects_inflated_cost() {
    let (cpu, mem, storage) = budget_for("submit_price");
    let inflated = Sample {
        endpoint: "submit_price",
        input: "boundary",
        cpu: cpu * 2,
        mem,
        storage: storage + 96,
    };
    let report = regressions(&inflated);
    assert_eq!(report.len(), 2, "cpu and storage regressions expected");
    assert!(report[0].contains("endpoint=submit_price input=boundary dimension=cpu"));
    assert!(report[1].contains("dimension=storage"));

    let within = Sample {
        endpoint: "submit_price",
        input: "boundary",
        cpu,
        mem,
        storage,
    };
    assert!(regressions(&within).is_empty());
}
