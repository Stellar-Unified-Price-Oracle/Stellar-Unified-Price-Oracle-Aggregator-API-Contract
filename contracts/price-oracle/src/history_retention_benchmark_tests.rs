#![cfg(test)]

//! # #694 — Worst-case history retention benchmark
//!
//! Fills every asset's history to the configured retention limit
//! (`max_history_length`) and measures writes, reads and pruning there:
//!
//! * writes on the way up, at the limit, and past it — every write past the
//!   limit evicts the oldest entry in the same call;
//! * bounded reads (point lookup, one full page, SEP-40 `prices`) at the limit
//!   and again after the evicting writes;
//! * explicit single-entry pruning (`prune_history`) at the limit.
//!
//! Boundary tests assert that the steady-state cost past the limit is flat
//! (bounded, not growing with the number of writes) and that eviction always
//! removes exactly the oldest entry. Run with `make retention-bench`; results
//! are described in `docs/history-retention-benchmark.md`.

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::test_helpers::ledger_default;
use crate::types::Asset;
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Retention limit under test: the top of the recommended
/// `max_history_length` range in `docs/deployment.md` (100–500).
const RETENTION_LIMIT: u32 = 500;
/// Assets filled to the limit — each one reaches it independently.
const ASSETS: u32 = 2;
/// Writes past the limit; each one evicts the oldest entry.
const EXTRA_WRITES: u32 = 5;
/// Steady-state costs past the limit may vary by at most this much.
const STEADY_STATE_TOLERANCE_PCT: u64 = 10;
/// Page size for reads (`history::MAX_PAGE_SIZE`).
const PAGE: u32 = 50;

/// First ledger that receives a history entry.
const FIRST_LEDGER: u32 = 101;

fn ts_for(ledger: u32) -> u64 {
    1_000_000 + ledger as u64 * 5
}

/// (cpu, mem) of a single top-level invocation.
fn measure<F: FnOnce()>(e: &Env, f: F) -> (u64, u64) {
    e.cost_estimate().budget().reset_unlimited();
    f();
    let budget = e.cost_estimate().budget();
    (budget.cpu_instruction_cost(), budget.memory_bytes_cost())
}

struct Bench<'a> {
    e: &'a Env,
    client: PriceOracleContractClient<'a>,
    source: Address,
    assets: Vec<Address>,
    /// Ledger of the most recent write.
    ledger: u32,
}

fn bench(e: &Env) -> Bench<'_> {
    e.mock_all_auths();
    e.cost_estimate().budget().reset_unlimited();
    ledger_default(e, 100, ts_for(100));
    let client = PriceOracleContractClient::new(e, &e.register(PriceOracleContract, ()));
    client.initialize(
        &Address::generate(e),
        &1u32,
        &RETENTION_LIMIT,
        &7u32,
        &String::from_str(e, "retention-bench"),
    );
    let source = Address::generate(e);
    client.add_source(&source, &String::from_str(e, "S"));
    let mut assets = Vec::new(e);
    for _ in 0..ASSETS {
        let a = Address::generate(e);
        client.register_asset(&a);
        assets.push_back(a);
    }
    Bench {
        e,
        client,
        source,
        assets,
        ledger: FIRST_LEDGER - 1,
    }
}

impl Bench<'_> {
    /// Advances one ledger and writes one history entry per asset. Returns the
    /// (cpu, mem) of each asset's write.
    fn write_round(&mut self) -> std::vec::Vec<(u64, u64)> {
        self.ledger += 1;
        let ts = ts_for(self.ledger);
        ledger_default(self.e, self.ledger, ts);
        let mut costs = std::vec::Vec::new();
        for i in 0..ASSETS {
            let asset = self.assets.get_unchecked(i);
            let price = 1_000_000 + (self.ledger % 7) as i128;
            costs.push(measure(self.e, || {
                self.client.submit_price(&self.source, &asset, &price, &ts)
            }));
        }
        costs
    }

    /// Every retained ledger for `asset`, oldest first, walked page by page.
    fn retained_ledgers(&self, asset: &Address) -> std::vec::Vec<u32> {
        let mut out = std::vec::Vec::new();
        let mut cursor = 0u32;
        loop {
            let (entries, next) = self
                .client
                .get_historical_prices_paginated(asset, &cursor, &PAGE);
            for i in 0..entries.len() {
                out.push(entries.get_unchecked(i).ledger);
            }
            match next {
                Some(n) => cursor = n,
                None => break,
            }
        }
        out
    }

    /// Bounded reads at the current state: (name, cpu, mem).
    fn reads(&self, asset: &Address) -> std::vec::Vec<(&'static str, u64, u64)> {
        let oldest = self.retained_ledgers(asset)[0];
        let sep40 = Asset::Stellar(asset.clone());
        let (c1, m1) = measure(self.e, || {
            self.client.get_historical_price(asset, &oldest);
        });
        let (c2, m2) = measure(self.e, || {
            self.client
                .get_historical_prices_paginated(asset, &0u32, &PAGE);
        });
        let (c3, m3) = measure(self.e, || {
            self.client.prices(&sep40, &PAGE);
        });
        std::vec![
            ("get_historical_price_oldest", c1, m1),
            ("get_historical_prices_paginated_page", c2, m2),
            ("prices_page", c3, m3),
        ]
    }

    /// Asserts `asset` retains exactly the `RETENTION_LIMIT` most recent
    /// ledgers, contiguous and ending at the current ledger.
    fn assert_retains_most_recent(&self, asset: &Address) {
        let ledgers = self.retained_ledgers(asset);
        let expected_oldest = self.ledger + 1 - RETENTION_LIMIT;
        assert_eq!(ledgers.len() as u32, RETENTION_LIMIT, "retained count");
        assert_eq!(
            ledgers[0], expected_oldest,
            "oldest retained entry must be the oldest not yet evicted"
        );
        for w in ledgers.windows(2) {
            assert_eq!(w[1], w[0] + 1, "retained history must be contiguous");
        }
        assert_eq!(*ledgers.last().unwrap(), self.ledger);
        assert_eq!(
            self.client.get_storage_budget(asset).entry_count,
            RETENTION_LIMIT
        );
    }
}

fn within_pct(values: &[u64], pct: u64) -> bool {
    let min = *values.iter().min().unwrap();
    let max = *values.iter().max().unwrap();
    max <= min + min * pct / 100
}

/// Fills every asset to the retention limit, then writes past it. Reports the
/// write cost on the way up and at the limit, and asserts the evicting
/// steady state is flat and evicts exactly the oldest entry each time.
#[test]
fn retention_bench_writes_and_eviction_at_limit() {
    let e = Env::default();
    let mut b = bench(&e);

    let first = b.write_round();
    for _ in 1..RETENTION_LIMIT - 1 {
        b.write_round();
    }
    let at_limit = b.write_round();
    for i in 0..ASSETS {
        let asset = b.assets.get_unchecked(i);
        assert_eq!(
            b.client.get_storage_budget(&asset).entry_count,
            RETENTION_LIMIT
        );
    }

    let mut evicting: std::vec::Vec<std::vec::Vec<(u64, u64)>> = std::vec::Vec::new();
    for _ in 0..EXTRA_WRITES {
        evicting.push(b.write_round());
        for i in 0..ASSETS {
            b.assert_retains_most_recent(&b.assets.get_unchecked(i));
        }
    }

    for i in 0..ASSETS as usize {
        std::println!(
            "RETENTION limit={RETENTION_LIMIT} asset={i} write=first cpu={} mem={}",
            first[i].0,
            first[i].1
        );
        std::println!(
            "RETENTION limit={RETENTION_LIMIT} asset={i} write=at_limit cpu={} mem={}",
            at_limit[i].0,
            at_limit[i].1
        );
        let cpu: std::vec::Vec<u64> = evicting.iter().map(|r| r[i].0).collect();
        let mem: std::vec::Vec<u64> = evicting.iter().map(|r| r[i].1).collect();
        for (k, (c, m)) in cpu.iter().zip(mem.iter()).enumerate() {
            std::println!(
                "RETENTION limit={RETENTION_LIMIT} asset={i} write=evicting_{} cpu={c} mem={m}",
                k + 1
            );
        }
        assert!(
            within_pct(&cpu, STEADY_STATE_TOLERANCE_PCT),
            "asset {i}: evicting-write cpu not bounded at the limit: {cpu:?}"
        );
        assert!(
            within_pct(&mem, STEADY_STATE_TOLERANCE_PCT),
            "asset {i}: evicting-write mem not bounded at the limit: {mem:?}"
        );
    }
}

/// Bounded reads cost the same at the limit and after further evicting
/// writes, and explicit single-entry pruning removes exactly the oldest entry.
#[test]
fn retention_bench_reads_and_pruning_at_limit() {
    let e = Env::default();
    let mut b = bench(&e);
    for _ in 0..RETENTION_LIMIT {
        b.write_round();
    }
    let asset = b.assets.get_unchecked(0);
    let before = b.reads(&asset);

    for _ in 0..EXTRA_WRITES {
        b.write_round();
    }
    b.assert_retains_most_recent(&asset);
    let after = b.reads(&asset);

    for ((name, c0, m0), (_, c1, m1)) in before.iter().zip(after.iter()) {
        std::println!(
            "RETENTION limit={RETENTION_LIMIT} read={name} at_limit_cpu={c0} at_limit_mem={m0} \
             after_eviction_cpu={c1} after_eviction_mem={m1}"
        );
        assert!(
            within_pct(&[*c0, *c1], STEADY_STATE_TOLERANCE_PCT),
            "{name}: read cpu not bounded at the limit ({c0} -> {c1})"
        );
        assert!(
            within_pct(&[*m0, *m1], STEADY_STATE_TOLERANCE_PCT),
            "{name}: read mem not bounded at the limit ({m0} -> {m1})"
        );
    }

    let oldest = b.retained_ledgers(&asset)[0];
    let mut pruned = 0;
    let (cpu, mem) = measure(&e, || {
        pruned = b.client.prune_history(&asset, &(RETENTION_LIMIT - 1));
    });
    std::println!(
        "RETENTION limit={RETENTION_LIMIT} prune=prune_history_one_entry cpu={cpu} mem={mem}"
    );
    assert_eq!(pruned, 1);
    let ledgers = b.retained_ledgers(&asset);
    assert_eq!(ledgers.len() as u32, RETENTION_LIMIT - 1);
    assert_eq!(
        ledgers[0],
        oldest + 1,
        "prune_history must evict the oldest entry"
    );
}
