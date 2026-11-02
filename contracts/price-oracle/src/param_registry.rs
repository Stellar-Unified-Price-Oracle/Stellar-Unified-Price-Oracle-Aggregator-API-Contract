//! # Parameter registry with typed bounds, units, owners and history (#544)
//!
//! See `docs/param-registry.md`.
//!
//! Every configurable parameter is described by a [`ParamSpec`] listed in
//! [`PARAMS`]. Writes go through [`set_param`], which rejects values outside
//! `[min, max]` with [`ErrorCode::ParamOutOfBounds`] and unknown names with
//! [`ErrorCode::ParamNotRegistered`]. Every accepted write — including
//! emergency writes via [`emergency_set_param`] — appends a [`ParamChange`] to a
//! bounded per-parameter ring of [`HISTORY_CAP`] entries.

use crate::errors::ErrorCode;
use soroban_sdk::{contracttype, panic_with_error, Address, Env, Symbol, Vec};

/// Maximum retained history entries per parameter (oldest evicted first).
pub const HISTORY_CAP: u32 = 32;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParamOwner {
    Admin = 0,
    Governance = 1,
}

/// Static description of one parameter. Bounds derivations live in the doc.
#[derive(Clone, Copy, Debug)]
pub struct ParamSpec {
    pub name: &'static str,
    pub unit: &'static str,
    pub min: i128,
    pub max: i128,
    pub default: i128,
    pub owner: ParamOwner,
}

/// The complete registry. `registry_is_complete` checks it against
/// [`REQUIRED_PARAMS`], which mirrors the configurable keys in `types.rs`.
pub const PARAMS: &[ParamSpec] = &[
    ParamSpec {
        name: "decimals",
        unit: "digits",
        min: 0,
        max: 18,
        default: 7,
        owner: ParamOwner::Admin,
    },
    ParamSpec {
        name: "max_age",
        unit: "seconds",
        min: 1,
        max: 86_400,
        default: 3_600,
        owner: ParamOwner::Admin,
    },
    ParamSpec {
        name: "min_sources",
        unit: "count",
        min: 1,
        max: 64,
        default: 1,
        owner: ParamOwner::Governance,
    },
    ParamSpec {
        name: "max_deviation",
        unit: "bps",
        min: 1,
        max: 10_000,
        default: 1_000,
        owner: ParamOwner::Governance,
    },
    ParamSpec {
        name: "ts_threshold",
        unit: "seconds",
        min: 1,
        max: 3_600,
        default: 300,
        owner: ParamOwner::Admin,
    },
    ParamSpec {
        name: "history_len",
        unit: "entries",
        min: 1,
        max: 1_000,
        default: 100,
        owner: ParamOwner::Admin,
    },
    ParamSpec {
        name: "quorum",
        unit: "bps",
        min: 5_001,
        max: 10_000,
        default: 6_667,
        owner: ParamOwner::Governance,
    },
    ParamSpec {
        name: "fee",
        unit: "stroops",
        min: 0,
        max: 100_000_000,
        default: 100,
        owner: ParamOwner::Governance,
    },
];

/// Names that must be present in [`PARAMS`] (completeness check).
pub const REQUIRED_PARAMS: &[&str] = &[
    "decimals",
    "max_age",
    "min_sources",
    "max_deviation",
    "ts_threshold",
    "history_len",
    "quorum",
    "fee",
];

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParamChange {
    pub actor: Address,
    pub old: i128,
    pub new: i128,
    pub ledger: u32,
    pub timestamp: u64,
    pub emergency: bool,
}

#[contracttype]
#[derive(Clone)]
enum ParamKey {
    Value(Symbol),
    History(Symbol),
}

pub fn spec(name: &str) -> Option<&'static ParamSpec> {
    PARAMS.iter().find(|p| p.name == name)
}

/// True when every required parameter is registered with sane bounds.
pub fn registry_is_complete() -> bool {
    REQUIRED_PARAMS.iter().all(|n| spec(n).is_some())
        && PARAMS
            .iter()
            .all(|p| p.min <= p.default && p.default <= p.max)
}

fn require_spec(env: &Env, name: &str) -> &'static ParamSpec {
    spec(name).unwrap_or_else(|| panic_with_error!(env, ErrorCode::ParamNotRegistered))
}

/// Validate a value against the registry without writing it.
pub fn validate(env: &Env, name: &str, value: i128) {
    let s = require_spec(env, name);
    if value < s.min || value > s.max {
        panic_with_error!(env, ErrorCode::ParamOutOfBounds);
    }
}

pub fn get_param(env: &Env, name: &str) -> i128 {
    let s = require_spec(env, name);
    env.storage()
        .persistent()
        .get(&ParamKey::Value(Symbol::new(env, name)))
        .unwrap_or(s.default)
}

fn write(env: &Env, actor: &Address, name: &str, value: i128, emergency: bool) -> i128 {
    validate(env, name, value);
    let old = get_param(env, name);
    let sym = Symbol::new(env, name);
    let store = env.storage().persistent();
    store.set(&ParamKey::Value(sym.clone()), &value);
    let hkey = ParamKey::History(sym);
    let mut hist: Vec<ParamChange> = store.get(&hkey).unwrap_or_else(|| Vec::new(env));
    if hist.len() >= HISTORY_CAP {
        hist.pop_front();
    }
    hist.push_back(ParamChange {
        actor: actor.clone(),
        old,
        new: value,
        ledger: env.ledger().sequence(),
        timestamp: env.ledger().timestamp(),
        emergency,
    });
    store.set(&hkey, &hist);
    old
}

/// Bounded, history-recording write. Caller must have authorised `actor`.
pub fn set_param(env: &Env, actor: &Address, name: &str, value: i128) -> i128 {
    actor.require_auth();
    write(env, actor, name, value, false)
}

/// Emergency write: same bounds, recorded with `emergency = true`.
pub fn emergency_set_param(env: &Env, actor: &Address, name: &str, value: i128) -> i128 {
    actor.require_auth();
    write(env, actor, name, value, true)
}

pub fn get_history(env: &Env, name: &str) -> Vec<ParamChange> {
    require_spec(env, name);
    env.storage()
        .persistent()
        .get(&ParamKey::History(Symbol::new(env, name)))
        .unwrap_or_else(|| Vec::new(env))
}
