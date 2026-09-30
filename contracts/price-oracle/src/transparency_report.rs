//! # Public transparency report from on-chain events (#543)
//!
//! See `docs/transparency-report.md`.
//!
//! [`build_report`] is a pure function over [`EventRecord`]s — values any
//! third party can decode from public contract events — so a report is
//! reproducible from chain data alone. [`publish_report`] stores one report per
//! fixed [`CADENCE_SECS`] period (idempotent: re-publishing a period must yield
//! an identical report, otherwise it is rejected).

use crate::errors::ErrorCode;
use soroban_sdk::{contracttype, panic_with_error, symbol_short, Address, Env, Vec};

/// Report cadence: one report per 7 days.
pub const CADENCE_SECS: u64 = 7 * 24 * 3_600;

/// Kind of public event feeding the report.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    /// A price submission by `source` (participation).
    Submission = 0,
    /// A round aggregated successfully.
    RoundOk = 1,
    /// A round aggregated in degraded mode / failed quorum.
    RoundDegraded = 2,
    /// A published price was later corrected.
    Correction = 3,
    /// Fee paid by a consumer; `amount` in stroops.
    Fee = 4,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventRecord {
    pub kind: EventKind,
    pub source: Option<Address>,
    pub amount: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransparencyReport {
    pub period: u64,
    /// Events considered (all kinds) within the period.
    pub event_count: u32,
    /// Distinct sources with >= 1 submission.
    pub participating_sources: u32,
    pub submissions: u32,
    pub rounds_total: u32,
    /// RoundOk / rounds_total in bps; 0 when rounds_total == 0.
    pub uptime_bps: u32,
    /// RoundDegraded / rounds_total in bps; 0 when rounds_total == 0.
    pub degradation_bps: u32,
    pub corrections: u32,
    /// Sum of Fee amounts.
    pub total_cost: i128,
    /// True when the period contained no events.
    pub empty: bool,
}

#[contracttype]
#[derive(Clone)]
enum ReportKey {
    Report(u64),
}

pub fn period_of(timestamp: u64) -> u64 {
    timestamp / CADENCE_SECS
}

fn bps(n: u32, d: u32) -> u32 {
    if d == 0 {
        0
    } else {
        ((n as u64) * 10_000 / d as u64) as u32
    }
}

/// Deterministic report for `period`; events outside it are ignored.
pub fn build_report(env: &Env, period: u64, events: &Vec<EventRecord>) -> TransparencyReport {
    let mut sources: Vec<Address> = Vec::new(env);
    let (mut n, mut subs, mut ok, mut deg, mut corr) = (0u32, 0u32, 0u32, 0u32, 0u32);
    let mut cost: i128 = 0;
    for e in events.iter() {
        if period_of(e.timestamp) != period {
            continue;
        }
        n += 1;
        match e.kind {
            EventKind::Submission => {
                subs += 1;
                if let Some(s) = e.source {
                    if !sources.contains(&s) {
                        sources.push_back(s);
                    }
                }
            }
            EventKind::RoundOk => ok += 1,
            EventKind::RoundDegraded => deg += 1,
            EventKind::Correction => corr += 1,
            EventKind::Fee => cost += e.amount,
        }
    }
    let rounds = ok + deg;
    TransparencyReport {
        period,
        event_count: n,
        participating_sources: sources.len(),
        submissions: subs,
        rounds_total: rounds,
        uptime_bps: bps(ok, rounds),
        degradation_bps: bps(deg, rounds),
        corrections: corr,
        total_cost: cost,
        empty: n == 0,
    }
}

/// Publish the report for a closed period. Only past periods may be
/// published; a second publish must match the stored report exactly.
pub fn publish_report(env: &Env, period: u64, events: &Vec<EventRecord>) -> TransparencyReport {
    if period >= period_of(env.ledger().timestamp()) {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let r = build_report(env, period, events);
    let key = ReportKey::Report(period);
    let store = env.storage().persistent();
    if let Some(prev) = store.get::<_, TransparencyReport>(&key) {
        if prev != r {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
        return prev;
    }
    store.set(&key, &r);
    env.events().publish((symbol_short!("tr_report"), period), r.clone());
    r
}

pub fn get_report(env: &Env, period: u64) -> Option<TransparencyReport> {
    env.storage().persistent().get(&ReportKey::Report(period))
}
