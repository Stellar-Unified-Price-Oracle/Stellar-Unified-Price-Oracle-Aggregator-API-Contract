#![cfg(test)]

//! # #419 — Adversarial gas-budget regression gates
//!
//! Every hot-path endpoint is measured against a deterministic **adversarial**
//! input corpus and must stay within its committed CPU-instruction budget
//! (`BUDGETS`) plus `TOLERANCE_PCT`. A regression fails `make test` / CI with
//! the endpoint, the corpus input and the delta. Budgets are the maximum cost
//! over the corpus; see `docs/gas-budget.md` for derivation, per-endpoint
//! analysis and the budget-update process.

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::{PriceOracleContract, PriceOracleContractClient};

/// Allowed growth over a committed budget before the gate fails.
const TOLERANCE_PCT: u64 = 5;

/// Sources per asset in the corpus — every one of them lands in the median
/// window. 10 is the largest value for which `submit_price` still fits the
/// network's 100-entry footprint limit (11 sources → 102 entries).
const N_SOURCES: u32 = 10;
/// Assets per `submit_prices` batch in the corpus. With every asset fully
/// populated, a 2-asset batch already needs 152 footprint entries (> 100),
/// so 1 is the largest batch callable on-network.
const BATCH_LEN: u32 = 1;

/// Committed CPU-instruction budgets, one per (endpoint, adversarial input).
/// Update only via the process in `docs/gas-budget.md`.
const BUDGETS: &[(&str, &str, u64)] = &[
    (
        "submit_price",
        "nth_of_10_near_miss_reverse_sorted",
        6_559_000,
    ),
    ("submit_prices", "batch_1_asset_10_sources", 6_262_000),
    ("get_all_prices", "asset_with_10_sources", 756_000),
    (
        "trigger_aggregation",
        "10_near_miss_reverse_sorted",
        1_287_000,
    ),
];

fn budget_for(endpoint: &str, input: &str) -> u64 {
    BUDGETS
        .iter()
        .find(|(e, i, _)| *e == endpoint && *i == input)
        .map(|(_, _, b)| *b)
        .unwrap_or_else(|| panic!("no committed budget for {endpoint}/{input}"))
}

/// Fails with the endpoint, input and delta when `measured` exceeds budget + tolerance.
fn assert_within_budget(endpoint: &str, input: &str, measured: u64) {
    let budget = budget_for(endpoint, input);
    let ceiling = budget + budget * TOLERANCE_PCT / 100;
    if measured > ceiling {
        panic!(
            "GAS BUDGET EXCEEDED endpoint={endpoint} input={input} measured={measured} \
             budget={budget} ceiling={ceiling} delta=+{} (+{}%)",
            measured - budget,
            (measured - budget) * 100 / budget.max(1),
        );
    }
}

/// Measures the CPU instructions of a single top-level invocation.
fn measure<F: FnOnce()>(e: &Env, f: F) -> u64 {
    e.cost_estimate().budget().reset_unlimited();
    f();
    e.cost_estimate().budget().cpu_instruction_cost()
}

struct Corpus<'a> {
    e: &'a Env,
    client: PriceOracleContractClient<'a>,
    sources: Vec<Address>,
}

fn corpus(e: &Env) -> Corpus<'_> {
    e.mock_all_auths();
    e.cost_estimate().budget().reset_unlimited();
    let id = e.register(PriceOracleContract, ());
    let client = PriceOracleContractClient::new(e, &id);
    client.initialize(
        &Address::generate(e),
        &2u32,
        &10u32,
        &7u32,
        &String::from_str(e, "gas-budget"),
    );
    let mut sources = Vec::new(e);
    for _ in 0..N_SOURCES {
        let s = Address::generate(e);
        client.add_source(&s, &String::from_str(e, "S"));
        sources.push_back(s);
    }
    Corpus { e, client, sources }
}

impl Corpus<'_> {
    fn asset(&self) -> Address {
        let a = Address::generate(self.e);
        self.client.register_asset(&a);
        a
    }

    /// Adversarial price for source `i`: reverse-sorted, alternating ±1
    /// "near-miss" values that all stay inside the median window.
    fn adversarial(i: u32) -> i128 {
        let base = 1_000_000i128 + (N_SOURCES - i) as i128;
        if i % 2 == 0 {
            base + 1
        } else {
            base - 1
        }
    }

    /// Submits from the first `n` sources using `price_of`.
    fn fill(&self, asset: &Address, n: u32, price_of: fn(u32) -> i128) {
        let ts = self.e.ledger().timestamp();
        for i in 0..n {
            self.client
                .submit_price(&self.sources.get_unchecked(i), asset, &price_of(i), &ts);
        }
    }
}

fn friendly(_i: u32) -> i128 {
    1_000_000
}

fn measure_nth_submit(price_of: fn(u32) -> i128) -> u64 {
    let e = Env::default();
    let c = corpus(&e);
    let asset = c.asset();
    c.fill(&asset, N_SOURCES - 1, price_of);
    let last = c.sources.get_unchecked(N_SOURCES - 1);
    let ts = e.ledger().timestamp();
    let price = price_of(N_SOURCES - 1);
    measure(&e, || c.client.submit_price(&last, &asset, &price, &ts))
}

#[test]
fn gas_submit_price_worst_case() {
    let cpu = measure_nth_submit(Corpus::adversarial);
    std::println!("submit_price nth_of_10_near_miss_reverse_sorted cpu={cpu}");
    assert_within_budget("submit_price", "nth_of_10_near_miss_reverse_sorted", cpu);
}

#[test]
fn gas_submit_prices_max_batch() {
    let e = Env::default();
    let c = corpus(&e);
    let ts = e.ledger().timestamp();
    let mut batch = Vec::new(&e);
    for _ in 0..BATCH_LEN {
        let asset = c.asset();
        c.fill(&asset, N_SOURCES - 1, Corpus::adversarial);
        batch.push_back((asset, Corpus::adversarial(N_SOURCES - 1), ts));
    }
    let last = c.sources.get_unchecked(N_SOURCES - 1);
    let cpu = measure(&e, || c.client.submit_prices(&last, &batch));
    std::println!("submit_prices batch_1_asset_10_sources cpu={cpu}");
    assert_within_budget("submit_prices", "batch_1_asset_10_sources", cpu);
}

#[test]
fn gas_get_all_prices_largest_set() {
    let e = Env::default();
    let c = corpus(&e);
    let asset = c.asset();
    c.fill(&asset, N_SOURCES, Corpus::adversarial);
    let cpu = measure(&e, || {
        c.client.get_all_prices(&asset);
    });
    std::println!("get_all_prices asset_with_10_sources cpu={cpu}");
    assert_within_budget("get_all_prices", "asset_with_10_sources", cpu);
}

#[test]
fn gas_trigger_aggregation_full_scan() {
    let e = Env::default();
    let c = corpus(&e);
    let asset = c.asset();
    c.fill(&asset, N_SOURCES, Corpus::adversarial);
    let cpu = measure(&e, || c.client.trigger_aggregation(&asset));
    std::println!("trigger_aggregation 10_near_miss_reverse_sorted cpu={cpu}");
    assert_within_budget("trigger_aggregation", "10_near_miss_reverse_sorted", cpu);
}

/// The Nth caller's cost must not be materially amplified by the input the
/// previous N-1 callers chose: adversarial history may cost at most 10% more
/// than friendly (all-equal) history.
#[test]
fn gas_marginal_cost_not_amplified_by_prior_callers() {
    let friendly_cpu = measure_nth_submit(friendly);
    let adversarial_cpu = measure_nth_submit(Corpus::adversarial);
    std::println!("marginal submit_price friendly={friendly_cpu} adversarial={adversarial_cpu}");
    assert!(
        adversarial_cpu * 100 <= friendly_cpu * 110,
        "GAS AMPLIFICATION endpoint=submit_price input=nth_of_10_near_miss_reverse_sorted \
         friendly={friendly_cpu} adversarial={adversarial_cpu}"
    );
}

/// The gate itself must fail, and name endpoint/input/delta, on a regression.
#[test]
#[should_panic(
    expected = "GAS BUDGET EXCEEDED endpoint=submit_price input=nth_of_10_near_miss_reverse_sorted"
)]
fn gas_gate_rejects_inflated_cost() {
    let budget = budget_for("submit_price", "nth_of_10_near_miss_reverse_sorted");
    assert_within_budget(
        "submit_price",
        "nth_of_10_near_miss_reverse_sorted",
        budget + budget * (TOLERANCE_PCT + 1) / 100 + 1,
    );
}
