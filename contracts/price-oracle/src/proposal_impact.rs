//! # Proposal simulation with worst-case economic impact analysis (#545)
//!
//! See `docs/proposal-impact.md`.
//!
//! [`simulate`] applies a proposal's parameter changes to a copy of the
//! current [`EconState`] and reports quorum feasibility, fee and source-income
//! deltas, and a worst case that assumes the most adverse participation
//! ([`WORST_CASE_DISSENT_BPS`]). [`attach`] stores the result against the
//! proposal id; [`require_cleared`] refuses to let a proposal touching a
//! critical parameter pass without an attached, non-harmful analysis.

use crate::errors::ErrorCode;
use crate::param_registry;
use soroban_sdk::{contracttype, panic_with_error, Env, Symbol, Vec};

/// Assumed worst-case share (bps) of voting power that is absent or dissents.
/// A quorum above `10_000 - WORST_CASE_DISSENT_BPS` can then never be met.
pub const WORST_CASE_DISSENT_BPS: i128 = 1_000;
/// Source income may fall by at most this much (bps) before flagging.
pub const MAX_SOURCE_INCOME_DROP_BPS: i128 = 3_000;
/// Fees may rise by at most this much (bps) before flagging.
pub const MAX_FEE_RISE_BPS: i128 = 5_000;
/// Parameters for which an analysis is mandatory.
pub const CRITICAL: &[&str] = &["quorum", "fee", "min_sources", "max_deviation"];

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamDelta {
    pub name: Symbol,
    pub value: i128,
}

/// Modelled economic state (explicit assumptions).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EconState {
    pub quorum_bps: i128,
    pub fee: i128,
    pub active_sources: i128,
    pub min_sources: i128,
    pub queries_per_period: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SimulationReport {
    pub touches_critical: bool,
    pub quorum_feasible_worst_case: bool,
    pub sources_sufficient: bool,
    pub fee_change_bps: i128,
    pub source_income_change_bps: i128,
    pub harmful: bool,
}

#[contracttype]
#[derive(Clone)]
enum ImpactKey {
    Report(u64),
}

fn change_bps(old: i128, new: i128) -> i128 {
    if old == 0 {
        return if new == 0 { 0 } else { 10_000 };
    }
    (new - old) * 10_000 / old
}

fn is_critical(env: &Env, name: &Symbol) -> bool {
    CRITICAL.iter().any(|c| Symbol::new(env, c) == *name)
}

/// Pure simulation: no storage writes.
pub fn simulate(env: &Env, base: &EconState, deltas: &Vec<ParamDelta>) -> SimulationReport {
    let mut s = base.clone();
    let mut touches_critical = false;
    for d in deltas.iter() {
        touches_critical |= is_critical(env, &d.name);
        if d.name == Symbol::new(env, "quorum") {
            s.quorum_bps = d.value;
        } else if d.name == Symbol::new(env, "fee") {
            s.fee = d.value;
        } else if d.name == Symbol::new(env, "min_sources") {
            s.min_sources = d.value;
        }
    }
    let fee_change_bps = change_bps(base.fee, s.fee);
    // Income per source = fee * queries / sources; quantity demand assumed
    // inelastic (documented assumption), so income tracks fee directly.
    let per_src = |st: &EconState| {
        if st.active_sources == 0 {
            0
        } else {
            st.fee * st.queries_per_period / st.active_sources
        }
    };
    let source_income_change_bps = change_bps(per_src(base), per_src(&s));
    let quorum_feasible_worst_case =
        s.quorum_bps > 5_000 && s.quorum_bps <= 10_000 - WORST_CASE_DISSENT_BPS;
    let sources_sufficient = s.active_sources >= s.min_sources;
    let harmful = !quorum_feasible_worst_case
        || !sources_sufficient
        || fee_change_bps > MAX_FEE_RISE_BPS
        || source_income_change_bps < -MAX_SOURCE_INCOME_DROP_BPS;
    SimulationReport {
        touches_critical,
        quorum_feasible_worst_case,
        sources_sufficient,
        fee_change_bps,
        source_income_change_bps,
        harmful,
    }
}

/// Simulate, validate deltas against the registry, and attach to the proposal.
pub fn attach(
    env: &Env,
    proposal_id: u64,
    base: &EconState,
    deltas: &Vec<ParamDelta>,
) -> SimulationReport {
    for d in deltas.iter() {
        if let Some(spec) = param_registry::PARAMS
            .iter()
            .find(|p| Symbol::new(env, p.name) == d.name)
        {
            if d.value < spec.min || d.value > spec.max {
                panic_with_error!(env, ErrorCode::ParamOutOfBounds);
            }
        } else {
            panic_with_error!(env, ErrorCode::ParamNotRegistered);
        }
    }
    let r = simulate(env, base, deltas);
    env.storage()
        .persistent()
        .set(&ImpactKey::Report(proposal_id), &r);
    r
}

pub fn get_report(env: &Env, proposal_id: u64) -> Option<SimulationReport> {
    env.storage().persistent().get(&ImpactKey::Report(proposal_id))
}

/// Gate for execution: every proposal needs an attached report; critical
/// proposals additionally must not be flagged harmful. Not bypassable.
pub fn require_cleared(env: &Env, proposal_id: u64) {
    let r = get_report(env, proposal_id)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::SimulationRequired));
    if r.touches_critical && r.harmful {
        panic_with_error!(env, ErrorCode::ProposalHarmful);
    }
}
