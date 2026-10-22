#![cfg(test)]

//! # #695 — Ledger burst submission capacity test
//!
//! Drives simultaneous source submissions across many assets, ledger by
//! ledger, against target ledger and RPC capacity:
//!
//! * a **quiet** phase below capacity,
//! * a **burst** phase where every source submits for every asset in the same
//!   ledgers (offered load above capacity),
//! * a **drain** phase back at quiet load, until the backlog is gone.
//!
//! Each ledger admits queued submissions in arrival order until one of the
//! targets would be exceeded: ledger CPU, Soroban transactions per ledger, or
//! RPC `sendTransaction` requests per ledger. Admitted submissions really
//! execute against the contract and are metered; the rest stay queued. A newer
//! submission for a (source, asset) pair that is still queued replaces the
//! queued one, as a submitter would only ever send its latest price.
//!
//! The test reports sustainable throughput, queue growth per ledger and the
//! first resource limit reached. Run with `make burst-capacity`; results are
//! described in `docs/ledger-burst-capacity.md`.

use soroban_sdk::{testutils::Address as _, Address, Env, String, Vec};

use crate::test_helpers::ledger_default;
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Target ledger-wide CPU instruction capacity available to the oracle.
const LEDGER_CPU_TARGET: u64 = 500_000_000;
/// Target Soroban transactions per ledger available to the oracle.
const LEDGER_TX_TARGET: u32 = 100;
/// Target RPC `sendTransaction` requests per ledger (one per submission).
const RPC_SUBMIT_TARGET: u32 = 60;

/// Source set: the supported maximum (`gas_budget_tests::N_SOURCES`).
const SOURCES: u32 = 10;
const ASSETS: u32 = 12;
/// Quiet load: this many sources submit for this many assets per ledger.
const QUIET_SOURCES: u32 = 3;
const QUIET_ASSETS: u32 = 4;

const QUIET_LEDGERS: u32 = 5;
const BURST_LEDGERS: u32 = 3;
/// Upper bound on drain ledgers before the test fails.
const MAX_DRAIN_LEDGERS: u32 = 20;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Limit {
    LedgerCpu,
    LedgerTx,
    RpcSubmit,
}

impl Limit {
    fn name(self) -> &'static str {
        match self {
            Limit::LedgerCpu => "ledger_cpu",
            Limit::LedgerTx => "ledger_tx",
            Limit::RpcSubmit => "rpc_submit",
        }
    }
}

/// Per-ledger outcome.
#[derive(Debug)]
struct LedgerReport {
    ledger: u32,
    phase: &'static str,
    offered: u32,
    coalesced: u32,
    admitted: u32,
    cpu_used: u64,
    queue_after: u32,
    limit: Option<Limit>,
}

struct Rig<'a> {
    e: &'a Env,
    client: PriceOracleContractClient<'a>,
    sources: Vec<Address>,
    assets: Vec<Address>,
    /// Pending (source index, asset index), oldest first.
    queue: std::collections::VecDeque<(u32, u32)>,
    /// Worst single-submission CPU seen so far; used to decide whether the
    /// next submission still fits the ledger CPU target before sending it.
    worst_cpu: u64,
    ledger: u32,
}

fn rig(e: &Env) -> Rig<'_> {
    e.mock_all_auths();
    e.cost_estimate().budget().reset_unlimited();
    ledger_default(e, 100, 1_000_000);
    let client = PriceOracleContractClient::new(e, &e.register(PriceOracleContract, ()));
    client.initialize(
        &Address::generate(e),
        &3u32,
        &50u32,
        &7u32,
        &String::from_str(e, "burst-capacity"),
    );
    let mut sources = Vec::new(e);
    for _ in 0..SOURCES {
        let s = Address::generate(e);
        client.add_source(&s, &String::from_str(e, "S"));
        sources.push_back(s);
    }
    let mut assets = Vec::new(e);
    for _ in 0..ASSETS {
        let a = Address::generate(e);
        client.register_asset(&a);
        assets.push_back(a);
    }
    Rig {
        e,
        client,
        sources,
        assets,
        queue: std::collections::VecDeque::new(),
        worst_cpu: 0,
        ledger: 100,
    }
}

/// Honest values spread ±0.2% around 1 000 000.
fn price_of(source: u32) -> i128 {
    1_000_000 - 2_000 + (source as i128 * 4_000) / SOURCES as i128
}

impl Rig<'_> {
    /// Closes one ledger: enqueues `n_sources × n_assets` new submissions, then
    /// admits from the queue until a target would be exceeded.
    fn run_ledger(&mut self, phase: &'static str, n_sources: u32, n_assets: u32) -> LedgerReport {
        self.ledger += 1;
        let ts = 1_000_000 + (self.ledger as u64 - 100) * 5;
        ledger_default(self.e, self.ledger, ts);

        let (mut offered, mut coalesced) = (0u32, 0u32);
        for a in 0..n_assets {
            for s in 0..n_sources {
                offered += 1;
                if self.queue.contains(&(s, a)) {
                    coalesced += 1;
                } else {
                    self.queue.push_back((s, a));
                }
            }
        }

        let (mut admitted, mut cpu_used, mut limit) = (0u32, 0u64, None);
        while !self.queue.is_empty() {
            if admitted >= RPC_SUBMIT_TARGET {
                limit = Some(Limit::RpcSubmit);
                break;
            }
            if admitted >= LEDGER_TX_TARGET {
                limit = Some(Limit::LedgerTx);
                break;
            }
            if cpu_used + self.worst_cpu > LEDGER_CPU_TARGET {
                limit = Some(Limit::LedgerCpu);
                break;
            }
            let (s, a) = self.queue.pop_front().unwrap();
            let source = self.sources.get_unchecked(s);
            let asset = self.assets.get_unchecked(a);
            self.e.cost_estimate().budget().reset_unlimited();
            self.client.submit_price(&source, &asset, &price_of(s), &ts);
            let cpu = self.e.cost_estimate().budget().cpu_instruction_cost();
            self.worst_cpu = self.worst_cpu.max(cpu);
            cpu_used += cpu;
            admitted += 1;
        }

        let report = LedgerReport {
            ledger: self.ledger,
            phase,
            offered,
            coalesced,
            admitted,
            cpu_used,
            queue_after: self.queue.len() as u32,
            limit,
        };
        std::println!(
            "BURST ledger={} phase={} offered={} coalesced={} admitted={} cpu_used={} \
             queue={} limit={}",
            report.ledger,
            report.phase,
            report.offered,
            report.coalesced,
            report.admitted,
            report.cpu_used,
            report.queue_after,
            report.limit.map(Limit::name).unwrap_or("none"),
        );
        report
    }
}

#[test]
fn burst_capacity_reports_throughput_queue_and_first_limit() {
    let e = Env::default();
    let mut r = rig(&e);
    let mut reports = std::vec::Vec::new();

    for _ in 0..QUIET_LEDGERS {
        reports.push(r.run_ledger("quiet", QUIET_SOURCES, QUIET_ASSETS));
    }
    for _ in 0..BURST_LEDGERS {
        reports.push(r.run_ledger("burst", SOURCES, ASSETS));
    }
    let mut drained = false;
    for _ in 0..MAX_DRAIN_LEDGERS {
        let report = r.run_ledger("drain", QUIET_SOURCES, QUIET_ASSETS);
        let empty = report.queue_after == 0;
        reports.push(report);
        if empty {
            drained = true;
            break;
        }
    }

    // Every ledger stays inside every target.
    for rep in &reports {
        assert!(
            rep.cpu_used <= LEDGER_CPU_TARGET,
            "ledger {} over CPU",
            rep.ledger
        );
        assert!(
            rep.admitted <= LEDGER_TX_TARGET,
            "ledger {} over tx",
            rep.ledger
        );
        assert!(
            rep.admitted <= RPC_SUBMIT_TARGET,
            "ledger {} over RPC",
            rep.ledger
        );
    }

    // Below capacity: nothing queues and no limit is reached.
    for rep in reports.iter().filter(|r| r.phase == "quiet") {
        assert_eq!(
            rep.queue_after, 0,
            "quiet ledger {} queued work",
            rep.ledger
        );
        assert_eq!(rep.limit, None, "quiet ledger {} hit a limit", rep.ledger);
    }

    // Above capacity: a limit is reached and the backlog grows.
    let burst: std::vec::Vec<&LedgerReport> =
        reports.iter().filter(|r| r.phase == "burst").collect();
    let first_limit = reports
        .iter()
        .find_map(|r| r.limit)
        .expect("burst load must reach a resource limit");
    assert!(burst.iter().all(|r| r.limit.is_some()));
    assert!(burst.iter().all(|r| r.queue_after > 0));

    // Sustainable throughput: submissions admitted per saturated ledger.
    let saturated: std::vec::Vec<&LedgerReport> =
        reports.iter().filter(|r| r.limit.is_some()).collect();
    let sustainable = saturated.iter().map(|r| r.admitted).sum::<u32>() / saturated.len() as u32;
    let peak_queue = reports.iter().map(|r| r.queue_after).max().unwrap();
    let growth: std::vec::Vec<i64> = burst
        .iter()
        .scan(0i64, |prev, r| {
            let d = r.queue_after as i64 - *prev;
            *prev = r.queue_after as i64;
            Some(d)
        })
        .collect();
    std::println!(
        "BURST summary sources={SOURCES} assets={ASSETS} sustainable_per_ledger={sustainable} \
         worst_submit_cpu={} peak_queue={peak_queue} burst_queue_growth={growth:?} \
         first_limit={}",
        r.worst_cpu,
        first_limit.name(),
    );
    assert!(sustainable > 0);

    // Recovery: back at quiet load the backlog drains completely.
    assert!(
        drained,
        "queue did not drain within {MAX_DRAIN_LEDGERS} ledgers"
    );

    // Conservation: every offered submission was admitted or superseded.
    let offered: u32 = reports.iter().map(|r| r.offered).sum();
    let coalesced: u32 = reports.iter().map(|r| r.coalesced).sum();
    let admitted: u32 = reports.iter().map(|r| r.admitted).sum();
    assert_eq!(
        offered,
        admitted + coalesced,
        "submissions lost or duplicated"
    );

    // Multi-asset traffic landed: every asset has a published aggregate.
    for i in 0..ASSETS {
        assert!(r
            .client
            .get_price(&r.assets.get_unchecked(i), &0u64)
            .is_some());
    }
}
