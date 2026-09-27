//! Invariant specification and violation-detection harness (#471).
//!
//! Each invariant in `docs/security/invariants.md` maps to an `INV_*` id below.
//! A seeded hostile sequence generator drives the public API; after every step all
//! safety invariants are asserted, liveness invariants are checked at the end of
//! the sequence, and per-invariant exercise counts are reported so coverage gaps
//! are visible. Failing sequences are minimized to a short reproducer.

use std::{collections::BTreeMap, vec::Vec};

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

use crate::test_helpers::{ledger_default, setup_contract};

pub const INV_REGISTRY: &str = "S1 source registry equals admitted-minus-removed set";
pub const INV_ASSETS: &str = "S2 asset registry equals registered set";
pub const INV_REJECT_UNREGISTERED: &str = "S3 unregistered/removed source cannot submit";
pub const INV_AGG_BOUNDED: &str = "S4 aggregate lies within accepted submissions";
pub const INV_AGG_MONOTONIC: &str = "S5 aggregate timestamp never decreases";
pub const INV_QUORUM: &str = "S6 aggregate never published without quorum";
pub const INV_LIVE: &str = "L1 quorum of fresh submissions eventually yields a price";

const SAFETY: [&str; 6] = [
    INV_REGISTRY,
    INV_ASSETS,
    INV_REJECT_UNREGISTERED,
    INV_AGG_BOUNDED,
    INV_AGG_MONOTONIC,
    INV_QUORUM,
];

const MIN_SOURCES: u32 = 2;
const SOURCE_POOL: usize = 5;
const ASSET_POOL: usize = 3;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Op {
    AddSource(usize),
    RemoveSource(usize),
    RegisterAsset(usize),
    Submit(usize, usize, i128),
    AdvanceTime(u64),
}

#[derive(Debug)]
struct Violation {
    invariant: &'static str,
    step: usize,
}

type Coverage = BTreeMap<&'static str, u32>;

/// xorshift64* — deterministic, dependency-free.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

fn gen_ops(seed: u64, len: usize) -> Vec<Op> {
    let mut r = Rng(seed | 1);
    (0..len)
        .map(|_| match r.below(10) {
            0 | 1 => Op::AddSource(r.below(SOURCE_POOL as u64) as usize),
            2 => Op::RemoveSource(r.below(SOURCE_POOL as u64) as usize),
            3 => Op::RegisterAsset(r.below(ASSET_POOL as u64) as usize),
            4 => Op::AdvanceTime(1 + r.below(120)),
            // Hostile prices: extremes, zero, negatives and ordinary values.
            _ => {
                let price = match r.below(6) {
                    0 => i128::MAX / 4,
                    1 => 0,
                    2 => -(r.below(1_000) as i128),
                    _ => 1_000 + r.below(1_000) as i128,
                };
                Op::Submit(
                    r.below(SOURCE_POOL as u64) as usize,
                    r.below(ASSET_POOL as u64) as usize,
                    price,
                )
            }
        })
        .collect()
}

/// Executes `ops` against a fresh contract. `inject_bug` corrupts the model's
/// handling of removals to prove the harness detects and minimizes violations.
fn execute(ops: &[Op], inject_bug: bool) -> Result<Coverage, Violation> {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_700_000_000);
    let (c, _) = setup_contract(&e);
    c.set_min_sources_required(&MIN_SOURCES);

    let sources: Vec<Address> = (0..SOURCE_POOL).map(|_| Address::generate(&e)).collect();
    let assets: Vec<Address> = (0..ASSET_POOL).map(|_| Address::generate(&e)).collect();

    let mut reg_sources = [false; SOURCE_POOL];
    let mut reg_assets = [false; ASSET_POOL];
    let mut accepted: Vec<Vec<i128>> = vec![Vec::new(); ASSET_POOL];
    let mut last_ts = [0u64; ASSET_POOL];
    let mut quorum_seen = [false; ASSET_POOL];
    let mut submitters: Vec<Vec<usize>> = vec![Vec::new(); ASSET_POOL];
    let mut cov: Coverage = SAFETY.iter().chain([&INV_LIVE]).map(|n| (*n, 0)).collect();

    for (step, op) in ops.iter().enumerate() {
        let fail = |invariant| Err(Violation { invariant, step });
        match *op {
            Op::AddSource(i) => {
                if c.try_add_source(&sources[i], &String::from_str(&e, "s"))
                    .is_ok()
                {
                    reg_sources[i] = true;
                }
            }
            Op::RemoveSource(i) => {
                if c.try_remove_source(&sources[i]).is_ok() {
                    // A removed source no longer counts towards any pending quorum.
                    for subs in submitters.iter_mut() {
                        subs.retain(|x| *x != i);
                    }
                    if !inject_bug {
                        reg_sources[i] = false;
                    }
                }
            }
            Op::RegisterAsset(i) => {
                if c.try_register_asset(&assets[i]).is_ok() {
                    reg_assets[i] = true;
                }
            }
            Op::AdvanceTime(dt) => e.ledger().with_mut(|l| {
                l.timestamp += dt;
                l.sequence_number += 1;
            }),
            Op::Submit(s, a, price) => {
                let ts = e.ledger().timestamp();
                let ok = c
                    .try_submit_price(&sources[s], &assets[a], &price, &ts)
                    .is_ok();
                if !reg_sources[s] {
                    *cov.get_mut(INV_REJECT_UNREGISTERED).unwrap() += 1;
                    if ok {
                        return fail(INV_REJECT_UNREGISTERED);
                    }
                } else if ok {
                    accepted[a].push(price);
                    if !submitters[a].contains(&s) {
                        submitters[a].push(s);
                    }
                    if submitters[a].len() as u32 >= MIN_SOURCES {
                        quorum_seen[a] = true;
                    }
                }
            }
        }

        // ── Safety invariants, checked after every step ──
        *cov.get_mut(INV_REGISTRY).unwrap() += 1;
        let on_chain = c.get_oracle_sources().sources;
        for (i, s) in sources.iter().enumerate() {
            let count = on_chain.iter().filter(|x| x == s).count();
            if count > 1 || (count == 1) != reg_sources[i] {
                return fail(INV_REGISTRY);
            }
        }
        *cov.get_mut(INV_ASSETS).unwrap() += 1;
        for (i, a) in assets.iter().enumerate() {
            if c.is_asset_registered(a) != reg_assets[i] {
                return fail(INV_ASSETS);
            }
        }
        for (i, a) in assets.iter().enumerate() {
            if !reg_assets[i] {
                continue;
            }
            if let Some(p) = c.get_price(a, &0u64) {
                *cov.get_mut(INV_AGG_BOUNDED).unwrap() += 1;
                *cov.get_mut(INV_AGG_MONOTONIC).unwrap() += 1;
                *cov.get_mut(INV_QUORUM).unwrap() += 1;
                let lo = accepted[i].iter().min().copied();
                let hi = accepted[i].iter().max().copied();
                if !p.is_override
                    && !matches!((lo, hi), (Some(l), Some(h)) if l <= p.price && p.price <= h)
                {
                    return fail(INV_AGG_BOUNDED);
                }
                if p.timestamp < last_ts[i] {
                    return fail(INV_AGG_MONOTONIC);
                }
                last_ts[i] = p.timestamp;
                if !p.is_override && p.num_sources < MIN_SOURCES {
                    return fail(INV_QUORUM);
                }
            }
        }
    }

    // ── Liveness: checked once the sequence has settled ──
    for (i, a) in assets.iter().enumerate() {
        if quorum_seen[i] {
            *cov.get_mut(INV_LIVE).unwrap() += 1;
            if c.get_price(a, &0u64).is_none() {
                return Err(Violation {
                    invariant: INV_LIVE,
                    step: ops.len(),
                });
            }
        }
    }
    Ok(cov)
}

/// Truncates at the failing step, then greedily drops ops while the same
/// invariant still fails (one-at-a-time delta debugging).
fn minimize(ops: &[Op], inject_bug: bool, invariant: &'static str, step: usize) -> Vec<Op> {
    let mut cur: Vec<Op> = ops[..(step + 1).min(ops.len())].to_vec();
    let mut i = 0;
    while i < cur.len() {
        let mut cand = cur.clone();
        cand.remove(i);
        match execute(&cand, inject_bug) {
            Err(v) if v.invariant == invariant => cur = cand,
            _ => i += 1,
        }
    }
    cur
}

fn scenario(seed: u64) -> Vec<Op> {
    // Every sequence starts from a live oracle so aggregation invariants are reachable.
    let mut ops = vec![
        Op::RegisterAsset(0),
        Op::AddSource(0),
        Op::AddSource(1),
        Op::Submit(0, 0, 1_000),
        Op::Submit(1, 0, 1_100),
    ];
    ops.extend(gen_ops(seed, 40));
    ops
}

#[test]
fn invariants_hold_across_randomized_hostile_sequences() {
    let mut total: Coverage = BTreeMap::new();
    for seed in 1..=12u64 {
        let ops = scenario(seed);
        match execute(&ops, false) {
            Ok(cov) => {
                for (k, v) in cov {
                    *total.entry(k).or_default() += v;
                }
            }
            Err(v) => {
                let repro = minimize(&ops, false, v.invariant, v.step);
                panic!(
                    "invariant violated: {} — minimal reproducer: {repro:?}",
                    v.invariant
                );
            }
        }
    }
    // Report exercised (not merely passed) invariants; any zero is a coverage gap.
    for (k, v) in &total {
        println!("invariant coverage: {v:>5}  {k}");
    }
    let gaps: Vec<_> = total
        .iter()
        .filter(|(_, v)| **v == 0)
        .map(|(k, _)| *k)
        .collect();
    assert!(gaps.is_empty(), "invariants never exercised: {gaps:?}");
}

#[test]
fn injected_violation_is_detected_and_minimized() {
    let ops = scenario(3);
    let mut with_removal = ops.clone();
    with_removal.push(Op::AddSource(4));
    with_removal.push(Op::RemoveSource(4));
    let v = execute(&with_removal, true).expect_err("injected bug must be detected");
    assert_eq!(v.invariant, INV_REGISTRY);
    let repro = minimize(&with_removal, true, v.invariant, v.step);
    assert!(repro.len() <= 4, "reproducer not minimal: {repro:?}");
    assert!(matches!(repro.last(), Some(Op::RemoveSource(_))));
}
