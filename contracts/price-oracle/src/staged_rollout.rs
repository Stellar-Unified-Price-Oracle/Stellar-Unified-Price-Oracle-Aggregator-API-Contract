//! # Staged percentage config rollout with automatic rollback (#531)
//!
//! A configuration change is first applied to a deterministic subset of
//! assets (`percent` of the asset space), health is sampled while that
//! subset runs on the candidate, and the rollout either advances, completes
//! or is rolled back automatically when the failure rate crosses the gate.
//!
//! ```text
//! (baseline) ──start(p%)──> Rolling ──advance(q%)──> Rolling ──complete──> (candidate is baseline)
//!                              │
//!                              └── failure_bp > gate (after min_samples) ──> RolledBack (baseline)
//! ```
//!
//! ## Mixed-configuration semantics
//!
//! While a rollout is active each asset sees exactly one configuration:
//! `bucket(asset) < percent` → candidate, otherwise baseline. The bucket is
//! `sha256(xdr(asset))[0..4] % 100`, so it is stable across ledgers and a
//! widened rollout never moves an asset back to baseline. Consumers must
//! read [`effective_config`] per asset and never assume a global value.
//!
//! ## Atomic rollback
//!
//! The baseline is never mutated during a rollout; the candidate lives only
//! in the single [`RolloutKey::Active`] entry. Rollback is one storage
//! removal, so after it every asset resolves to the baseline again with no
//! partially reverted state. Health samples use `min_samples` plus a
//! failure-rate threshold (not single failures) to avoid flapping.

use soroban_sdk::{contractevent, contracttype, panic_with_error, xdr::ToXdr, Address, Env};

use crate::errors::ErrorCode;
use crate::storage::get_admin;

/// The configuration governed by staged rollout.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedConfig {
    pub max_deviation_bp: u32,
    pub min_sources: u32,
}

/// Health gate deciding automatic rollback.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HealthGate {
    /// Roll back when failures / samples exceeds this (basis points).
    pub max_failure_bp: u32,
    /// Samples required before the gate may trip (anti-flapping).
    pub min_samples: u32,
}

/// An in-flight rollout.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rollout {
    pub candidate: StagedConfig,
    pub percent: u32,
    pub gate: HealthGate,
    pub samples: u32,
    pub failures: u32,
    pub started_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum RolloutKey {
    Baseline,
    Active,
}

/// Rollout outcome reported by [`RolloutEvent`].
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RolloutPhase {
    Started,
    Advanced,
    Completed,
    RolledBack,
}

/// Emitted on every rollout transition.
///
/// Topics: `phase`
#[contractevent]
#[derive(Clone)]
pub struct RolloutEvent {
    #[topic]
    pub phase: RolloutPhase,
    pub percent: u32,
    pub samples: u32,
    pub failures: u32,
}

const DEFAULT_BASELINE: StagedConfig = StagedConfig {
    max_deviation_bp: 500,
    min_sources: 1,
};

fn require_admin(env: &Env) {
    get_admin(env).require_auth();
}

fn emit(env: &Env, phase: RolloutPhase, r: &Rollout) {
    RolloutEvent {
        phase,
        percent: r.percent,
        samples: r.samples,
        failures: r.failures,
    }
    .publish(env);
}

pub fn baseline(env: &Env) -> StagedConfig {
    env.storage()
        .persistent()
        .get(&RolloutKey::Baseline)
        .unwrap_or(DEFAULT_BASELINE)
}

pub fn active(env: &Env) -> Option<Rollout> {
    env.storage().persistent().get(&RolloutKey::Active)
}

/// Stable bucket in `0..100` for `asset`.
pub fn bucket(env: &Env, asset: &Address) -> u32 {
    let h = env.crypto().sha256(&asset.clone().to_xdr(env)).to_array();
    u32::from_be_bytes([h[0], h[1], h[2], h[3]]) % 100
}

/// Configuration in force for `asset` right now.
pub fn effective_config(env: &Env, asset: &Address) -> StagedConfig {
    match active(env) {
        Some(r) if bucket(env, asset) < r.percent => r.candidate,
        _ => baseline(env),
    }
}

fn validate(env: &Env, percent: u32, gate: &HealthGate) {
    if percent == 0 || percent > 100 || gate.max_failure_bp > 10_000 || gate.min_samples == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
}

pub fn start(env: &Env, candidate: StagedConfig, percent: u32, gate: HealthGate) {
    require_admin(env);
    validate(env, percent, &gate);
    if active(env).is_some() {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let r = Rollout {
        candidate,
        percent,
        gate,
        samples: 0,
        failures: 0,
        started_at: env.ledger().timestamp(),
    };
    env.storage().persistent().set(&RolloutKey::Active, &r);
    emit(env, RolloutPhase::Started, &r);
}

fn load(env: &Env) -> Rollout {
    active(env).unwrap_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
}

/// Widens the rollout; narrowing is rejected so no asset flips back.
pub fn advance(env: &Env, percent: u32) {
    require_admin(env);
    let mut r = load(env);
    validate(env, percent, &r.gate);
    if percent < r.percent {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    r.percent = percent;
    env.storage().persistent().set(&RolloutKey::Active, &r);
    emit(env, RolloutPhase::Advanced, &r);
}

/// Promotes the candidate to baseline for every asset.
pub fn complete(env: &Env) {
    require_admin(env);
    let r = load(env);
    env.storage().persistent().set(&RolloutKey::Baseline, &r.candidate);
    env.storage().persistent().remove(&RolloutKey::Active);
    emit(env, RolloutPhase::Completed, &r);
}

fn rollback_inner(env: &Env, r: &Rollout) {
    env.storage().persistent().remove(&RolloutKey::Active);
    emit(env, RolloutPhase::RolledBack, r);
}

/// Manual rollback.
pub fn rollback(env: &Env) {
    require_admin(env);
    let r = load(env);
    rollback_inner(env, &r);
}

/// Records a health sample from the canary subset. Returns `true` when the
/// sample tripped the gate and the rollout was rolled back automatically.
pub fn report_health(env: &Env, healthy: bool) -> bool {
    require_admin(env);
    let mut r = load(env);
    r.samples = r.samples.saturating_add(1);
    if !healthy {
        r.failures = r.failures.saturating_add(1);
    }
    let failure_bp = (r.failures as u64 * 10_000) / r.samples as u64;
    if r.samples >= r.gate.min_samples && failure_bp > r.gate.max_failure_bp as u64 {
        rollback_inner(env, &r);
        return true;
    }
    env.storage().persistent().set(&RolloutKey::Active, &r);
    false
}

#[cfg(test)]
mod test {
    use super::*;
    use crate::types::DataKey;
    use crate::PriceOracleContract;
    use soroban_sdk::testutils::{Address as _, Events as _};

    fn setup() -> (Env, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(PriceOracleContract, ());
        let admin = Address::generate(&env);
        env.as_contract(&id, || {
            env.storage().persistent().set(&DataKey::Admin, &admin);
        });
        (env, id)
    }

    fn cand() -> StagedConfig {
        StagedConfig {
            max_deviation_bp: 100,
            min_sources: 3,
        }
    }

    fn gate() -> HealthGate {
        HealthGate {
            max_failure_bp: 2_000,
            min_samples: 5,
        }
    }

    #[test]
    fn applies_to_subset_only() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            start(&env, cand(), 50, gate());
            let (mut on, mut off) = (0, 0);
            for _ in 0..200 {
                let a = Address::generate(&env);
                if effective_config(&env, &a) == cand() {
                    on += 1;
                } else {
                    assert_eq!(effective_config(&env, &a), DEFAULT_BASELINE);
                    off += 1;
                }
            }
            assert!(on > 0 && off > 0);
        });
    }

    #[test]
    fn degradation_rolls_back_atomically() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            start(&env, cand(), 100, gate());
            let a = Address::generate(&env);
            assert_eq!(effective_config(&env, &a), cand());
            // Below min_samples the gate cannot trip (anti-flapping).
            for _ in 0..4 {
                assert!(!report_health(&env, false));
            }
            assert!(report_health(&env, false));
            assert!(active(&env).is_none());
            assert_eq!(effective_config(&env, &a), DEFAULT_BASELINE);
            assert!(!env.events().all().events().is_empty());
        });
    }

    #[test]
    fn healthy_rollout_completes() {
        let (env, id) = setup();
        env.as_contract(&id, || {
            start(&env, cand(), 10, gate());
            for _ in 0..10 {
                assert!(!report_health(&env, true));
            }
            advance(&env, 100);
            complete(&env);
            assert_eq!(baseline(&env), cand());
            assert!(active(&env).is_none());
        });
    }
}
