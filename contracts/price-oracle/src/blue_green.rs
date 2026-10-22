//! # Blue-green upgrade and rollback guards (#415)
//!
//! Pure decision logic used by the blue-green upgrade tooling
//! (`scripts/blue-green-upgrade.sh`). It answers three questions an upgrade
//! driver must never get wrong:
//!
//! 1. **May we roll back?** Only with a quorum of distinct authorised
//!    approvers, and no sooner than `min_interval` ledgers after the previous
//!    rollback, so a single compromised key cannot pin a vulnerable version.
//! 2. **Is the rollback target schema-compatible?** A rollback must never run
//!    old code against a newer storage layout (schema desync).
//! 3. **Is state coherent after an abort?** A migration must be either not
//!    started or fully complete before any read or re-upgrade is served.
//!
//! See `docs/blue-green-upgrade.md` for the runbook.

use soroban_sdk::{symbol_short, Address, Env, Vec};

/// Documented defaults: 2-of-N approvers, 17,280 ledgers (~24h) between rollbacks.
pub const DEFAULT_QUORUM: u32 = 2;
pub const DEFAULT_MIN_INTERVAL: u32 = 17_280;
/// Maximum age, in ledgers, of a blue deployment that may still be rolled back to (~7 days).
pub const MAX_ROLLBACK_WINDOW: u32 = 120_960;

/// Rollback authorisation policy. Owned by the contract admin set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RollbackPolicy {
    pub quorum: u32,
    pub min_interval: u32,
}

impl Default for RollbackPolicy {
    fn default() -> Self {
        Self {
            quorum: DEFAULT_QUORUM,
            min_interval: DEFAULT_MIN_INTERVAL,
        }
    }
}

/// A deployed contract version: the WASM it runs and the schema it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deployment {
    pub schema_version: u32,
    /// Highest schema version this build can read (forward-compat window).
    pub max_readable_schema: u32,
    pub deployed_ledger: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RollbackDecision {
    Allowed,
    InsufficientQuorum,
    RateLimited,
    SchemaIncompatible,
    OutsideWindow,
}

/// Counts approvals that are both distinct and members of `signers`.
fn valid_approvals(signers: &Vec<Address>, approvals: &Vec<Address>) -> u32 {
    let mut seen: Vec<Address> = Vec::new(approvals.env());
    for a in approvals.iter() {
        if signers.contains(&a) && !seen.contains(&a) {
            seen.push_back(a);
        }
    }
    seen.len()
}

/// Decides whether rolling back from `green` to `blue` is permitted.
///
/// `stored_schema` is the schema version currently persisted on-chain, which
/// may be newer than `green.schema_version` after a partial migration.
#[allow(clippy::too_many_arguments)]
pub fn check_rollback(
    policy: &RollbackPolicy,
    signers: &Vec<Address>,
    approvals: &Vec<Address>,
    last_rollback_ledger: Option<u32>,
    current_ledger: u32,
    blue: &Deployment,
    stored_schema: u32,
) -> RollbackDecision {
    if valid_approvals(signers, approvals) < policy.quorum.max(1) {
        return RollbackDecision::InsufficientQuorum;
    }
    if let Some(last) = last_rollback_ledger {
        if current_ledger.saturating_sub(last) < policy.min_interval {
            return RollbackDecision::RateLimited;
        }
    }
    if current_ledger.saturating_sub(blue.deployed_ledger) > MAX_ROLLBACK_WINDOW {
        return RollbackDecision::OutsideWindow;
    }
    if stored_schema > blue.max_readable_schema {
        return RollbackDecision::SchemaIncompatible;
    }
    RollbackDecision::Allowed
}

/// Returns `true` when storage is in a single, coherent layout: no migration
/// cursor is outstanding and every asset reports the same schema version.
pub fn is_layout_coherent(migration_in_progress: bool, asset_schema_versions: &[u32]) -> bool {
    if migration_in_progress {
        return false;
    }
    match asset_schema_versions.first() {
        None => true,
        Some(v) => asset_schema_versions.iter().all(|x| x == v),
    }
}

/// Emits `("rollback", decision_code)` with the approver count for audit.
pub fn emit_rollback_decision(env: &Env, decision: RollbackDecision, approvals: u32) {
    env.events()
        .publish((symbol_short!("rollback"), decision as u32), approvals);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, vec};

    const NOW: u32 = 1_000_000;

    fn blue() -> Deployment {
        Deployment {
            schema_version: 2,
            max_readable_schema: 2,
            deployed_ledger: NOW - 1_000,
        }
    }

    fn setup(env: &Env) -> (Vec<Address>, Address, Address) {
        let a = Address::generate(env);
        let b = Address::generate(env);
        let c = Address::generate(env);
        (vec![env, a.clone(), b.clone(), c], a, b)
    }

    #[test]
    fn quorum_allows_rollback() {
        let env = Env::default();
        let (signers, a, b) = setup(&env);
        let d = check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &vec![&env, a, b],
            None,
            NOW,
            &blue(),
            2,
        );
        assert_eq!(d, RollbackDecision::Allowed);
    }

    #[test]
    fn single_compromised_key_cannot_roll_back() {
        let env = Env::default();
        let (signers, a, _) = setup(&env);
        // Same key submitted twice, plus an outsider, still counts as one.
        let outsider = Address::generate(&env);
        let approvals = vec![&env, a.clone(), a, outsider];
        let d = check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &approvals,
            None,
            NOW,
            &blue(),
            2,
        );
        assert_eq!(d, RollbackDecision::InsufficientQuorum);
    }

    #[test]
    fn rollback_is_rate_limited() {
        let env = Env::default();
        let (signers, a, b) = setup(&env);
        let d = check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &vec![&env, a, b],
            Some(NOW - 10),
            NOW,
            &blue(),
            2,
        );
        assert_eq!(d, RollbackDecision::RateLimited);
    }

    #[test]
    fn rollback_to_old_code_on_new_schema_is_refused() {
        let env = Env::default();
        let (signers, a, b) = setup(&env);
        let d = check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &vec![&env, a, b],
            None,
            NOW,
            &blue(),
            3,
        );
        assert_eq!(d, RollbackDecision::SchemaIncompatible);
    }

    #[test]
    fn rollback_outside_window_is_refused() {
        let env = Env::default();
        let (signers, a, b) = setup(&env);
        let old = Deployment {
            deployed_ledger: NOW - MAX_ROLLBACK_WINDOW - 1,
            ..blue()
        };
        let d = check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &vec![&env, a, b],
            None,
            NOW,
            &old,
            2,
        );
        assert_eq!(d, RollbackDecision::OutsideWindow);
    }

    #[test]
    fn mid_migration_abort_is_detected_as_incoherent() {
        assert!(!is_layout_coherent(true, &[2, 2]));
        assert!(!is_layout_coherent(false, &[1, 2, 2]));
        assert!(is_layout_coherent(false, &[2, 2, 2]));
        assert!(is_layout_coherent(false, &[]));
    }
}
