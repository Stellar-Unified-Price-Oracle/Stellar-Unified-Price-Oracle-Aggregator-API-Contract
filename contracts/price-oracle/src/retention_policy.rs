//! # Data Retention and Correction Trace Policy (#714)
//!
//! This module provides a configurable, on-chain retention policy for all
//! categories of oracle data. It answers: *"How long is each kind of record
//! guaranteed to be available?"*
//!
//! ## Categories and default retention
//!
//! | Category | Storage key | Default (ledgers) | Rationale |
//! |---|---|---|---|
//! | Price history | `PriceHistory(asset, ledger)` | `max_history_length` × cadence | Consumer time-series queries |
//! | Rejected submissions (audit entries) | `AuditEntry(id)` | `DEFAULT_AUDIT_RETENTION` | Dispute resolution, source quality review |
//! | Price corrections | `PriceRevisions(asset)` | `DEFAULT_CORRECTION_RETENTION` | Mandatory audit trace |
//! | Admin audit log | `AuditEntry(id)` | `DEFAULT_AUDIT_RETENTION` | Governance and accountability |
//!
//! ## Key invariant: corrections survive normal pruning
//!
//! `correction_ledgers >= history_ledgers` is enforced on write. A correction
//! record documents why and when an already-published price was revised. It
//! MUST outlive the underlying history entry it replaced so a consumer who
//! fetched the old price can verify what happened.
//!
//! ## TTL bump strategy
//!
//! On every write within a retention window, the relevant storage entry's TTL
//! is bumped to at least `retention_ledgers` from the current ledger. This is
//! a best-effort guarantee: Soroban may evict entries if the network's minimum
//! TTL is shorter than the configured window. Operators SHOULD set
//! `audit_ledgers` and `correction_ledgers` to values within the network's
//! archival TTL budget.
//!
//! ## Limits
//!
//! - Retention cannot be set below the network-enforced minimum TTL; entries
//!   will still be subject to Soroban's storage lifecycle.
//! - This policy does not retrieve deleted entries; it only extends TTLs going
//!   forward. Off-chain backup (`docs/backup-restore.md`) is required for
//!   long-term archival beyond on-chain storage limits.

use soroban_sdk::{panic_with_error, Address, Env};

use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, DataRetentionPolicy, ErrorCode};

// ─────────────────────────────────────────────────────────────────────────────
// Default retention constants
// ─────────────────────────────────────────────────────────────────────────────

/// Default minimum ledgers price-history snapshots are retained.
///
/// At ~5 s per ledger ≈ 17,280 ledgers/day × 90 days ≈ 1,555,200 ledgers.
/// We default to 60 days.
pub const DEFAULT_HISTORY_RETENTION: u32 = 1_036_800; // ~60 days

/// Default minimum ledgers audit log entries (rejected submissions, admin
/// actions) are retained.
///
/// Regulatory and dispute windows typically require 90–180 days.
/// Default: 180 days.
pub const DEFAULT_AUDIT_RETENTION: u32 = 3_110_400; // ~180 days

/// Default minimum ledgers price-correction revision chains are retained.
///
/// Corrections must survive at least as long as the history they revised.
/// Default: same as audit retention (180 days).
pub const DEFAULT_CORRECTION_RETENTION: u32 = 3_110_400; // ~180 days

/// Hard minimum for any retention window (must be at least 1 ledger).
pub const MIN_RETENTION_LEDGERS: u32 = 1;

// ─────────────────────────────────────────────────────────────────────────────
// Policy read/write
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the current retention policy, or the default policy if none has
/// been explicitly set by an admin.
pub fn get_retention_policy(env: &Env) -> DataRetentionPolicy {
    let key = DataKey::RetentionPolicy;
    if let Some(policy) = env.storage().persistent().get::<_, DataRetentionPolicy>(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        return policy;
    }
    default_policy(env)
}

/// Sets the data retention policy.
///
/// # Validation
///
/// - All windows must be >= `MIN_RETENTION_LEDGERS`.
/// - `audit_ledgers >= history_ledgers` — audit records must outlive history.
/// - `correction_ledgers >= history_ledgers` — corrections must outlive history.
///
/// # Authorization
///
/// The caller MUST have already verified admin authorization before calling
/// this function.
///
/// # Panics
///
/// [`ErrorCode::InvalidConfiguration`] when any validation rule is violated.
pub fn set_retention_policy(env: &Env, policy: DataRetentionPolicy) {
    validate_policy(env, &policy);
    let key = DataKey::RetentionPolicy;
    env.storage().persistent().set(&key, &policy);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

/// Returns the effective minimum retention in ledgers for price-history entries.
///
/// Used by `prices.rs` to decide how long to bump a history entry's TTL.
pub fn history_retention_ledgers(env: &Env) -> u32 {
    get_retention_policy(env).history_ledgers
}

/// Returns the effective minimum retention in ledgers for audit log entries
/// (including rejected-submission records).
///
/// Used by `audit_log.rs` to extend entry TTLs on write and on bump.
pub fn audit_retention_ledgers(env: &Env) -> u32 {
    get_retention_policy(env).audit_ledgers
}

/// Returns the effective minimum retention in ledgers for price-correction
/// revision chains.
///
/// Used by `corrections.rs` to bump revision chain TTLs after each correction.
pub fn correction_retention_ledgers(env: &Env) -> u32 {
    get_retention_policy(env).correction_ledgers
}

// ─────────────────────────────────────────────────────────────────────────────
// TTL enforcement helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Bumps the TTL of a price-history entry to at least `history_retention_ledgers`
/// from the current ledger.
///
/// Call this after writing a new history entry or after any operation that
/// reads a history entry within its retention window.
pub fn bump_history_entry(env: &Env, asset: &Address, ledger_seq: u32) {
    let key = DataKey::PriceHistory(asset.clone(), ledger_seq);
    if env.storage().temporary().has(&key) {
        let retention = history_retention_ledgers(env);
        let bump_to = retention.max(LEDGER_BUMP);
        env.storage()
            .temporary()
            .extend_ttl(&key, LEDGER_THRESHOLD, bump_to);
    }
}

/// Bumps the TTL of an audit entry to at least `audit_retention_ledgers`.
///
/// Call this after writing a new audit entry or when inspecting an existing one.
pub fn bump_audit_entry(env: &Env, entry_id: u32) {
    let key = DataKey::AuditEntry(entry_id);
    if env.storage().persistent().has(&key) {
        let retention = audit_retention_ledgers(env);
        let bump_to = retention.max(LEDGER_BUMP);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, bump_to);
    }
}

/// Bumps the TTL of the price-revision chain for `asset` to at least
/// `correction_retention_ledgers`.
///
/// Call this after writing a new correction to the revision chain.
pub fn bump_correction_chain(env: &Env, asset: &Address) {
    let key = DataKey::PriceRevisions(asset.clone());
    if env.storage().persistent().has(&key) {
        let retention = correction_retention_ledgers(env);
        let bump_to = retention.max(LEDGER_BUMP);
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, bump_to);
    }
    // Also bump the correction count key.
    let count_key = DataKey::PriceCorrectionCount(asset.clone());
    if env.storage().persistent().has(&count_key) {
        let retention = correction_retention_ledgers(env);
        let bump_to = retention.max(LEDGER_BUMP);
        env.storage()
            .persistent()
            .extend_ttl(&count_key, LEDGER_THRESHOLD, bump_to);
    }
}

/// Applies the retention policy to all audit entries currently stored,
/// extending their TTLs to satisfy the minimum window.
///
/// This is an admin-callable sweep operation that enforces the policy
/// retroactively. It is bounded by `max_entries` to avoid exceeding the
/// invocation budget.
///
/// Returns the number of entries whose TTLs were bumped.
pub fn apply_retention_to_audit_log(env: &Env, max_entries: u32) -> u32 {
    let count_key = DataKey::AuditEntryCount;
    let total: u32 = env
        .storage()
        .persistent()
        .get(&count_key)
        .unwrap_or(0);

    let retention = audit_retention_ledgers(env);
    let bump_to = retention.max(LEDGER_BUMP);

    let start = total.saturating_sub(max_entries);
    let mut bumped: u32 = 0;

    for id in start..total {
        let key = DataKey::AuditEntry(id);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, bump_to);
            bumped += 1;
        }
    }

    bumped
}

// ─────────────────────────────────────────────────────────────────────────────
// Internal helpers
// ─────────────────────────────────────────────────────────────────────────────

fn default_policy(env: &Env) -> DataRetentionPolicy {
    DataRetentionPolicy {
        history_ledgers: DEFAULT_HISTORY_RETENTION,
        audit_ledgers: DEFAULT_AUDIT_RETENTION,
        correction_ledgers: DEFAULT_CORRECTION_RETENTION,
        set_ledger: env.ledger().sequence(),
        active: true,
    }
}

fn validate_policy(env: &Env, policy: &DataRetentionPolicy) {
    if policy.history_ledgers < MIN_RETENTION_LEDGERS
        || policy.audit_ledgers < MIN_RETENTION_LEDGERS
        || policy.correction_ledgers < MIN_RETENTION_LEDGERS
    {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    // Corrections and audit records must outlive the history they reference.
    if policy.audit_ledgers < policy.history_ledgers {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    if policy.correction_ledgers < policy.history_ledgers {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
}
