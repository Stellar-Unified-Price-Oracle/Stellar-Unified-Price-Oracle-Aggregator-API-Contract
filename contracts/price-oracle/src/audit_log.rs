//! Admin audit log module with hash chain (#239)
//!
//! Implements an append-only audit trail for all admin actions with cryptographic
//! hash chain to prevent tampering. Each entry includes a SHA-256 hash of the
//! previous entry, forming an immutable chain.
//!
//! # Source reward appeals and corrections (#674)
//!
//! A source may dispute a measured contribution or payout by submitting an
//! appeal. Appeals are recorded as audit entries (action `appeal`) carrying the
//! source identity, the disputed epoch, the disputed amount and a reason. An
//! authorized reviewer resolves the appeal by appending a `correct` audit entry
//! that references the original appeal entry id. Corrections are *appended* to
//! the chain; the original audit record is never mutated or overwritten, so the
//! full dispute history remains verifiable via [`verify_audit_chain`].

use soroban_sdk::{
    contracttype, panic_with_error, symbol_short, xdr::ToXdr, Address, Bytes, Env, String, Symbol,
    Vec,
};

use crate::events::AdminAuditEntryAppendedEvent;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AuditEntry, DataKey, ErrorCode};

/// Append an admin action to the immutable audit log.
///
/// This is called internally by all admin actions to create a tamper-evident record.
/// Each entry is hashed with the previous entry's hash to form a chain.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `action` - The action symbol (e.g., "pause", "setadm").
/// * `admin` - The admin address that performed the action.
/// * `data` - Arbitrary data associated with the action.
pub fn append_audit_entry(env: &Env, action: Symbol, admin: Address, data: Bytes) {
    let current_ledger = env.ledger().sequence();
    let timestamp = env.ledger().timestamp();

    // Get the current audit log head (previous hash) as Bytes
    let previous_hash: Bytes = env
        .storage()
        .persistent()
        .get(&DataKey::AuditLogHead)
        .unwrap_or_else(|| Bytes::new(env));

    // Get next entry ID
    let entry_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::AuditEntryCount)
        .unwrap_or(0);
    let entry_id = entry_count + 1;

    // Compute hash of this entry: sha256(previous_hash || data || timestamp || entry_id)
    let mut entry_data = Bytes::new(env);
    entry_data.append(&previous_hash);

    entry_data.append(&data);

    entry_data.append(&Bytes::from_slice(env, &timestamp.to_le_bytes()));

    entry_data.append(&Bytes::from_slice(env, &entry_id.to_le_bytes()));

    // Compute SHA-256 hash
    // Compute SHA-256 and store as `Bytes`
    let sha = env.crypto().sha256(&entry_data);
    let sha_bytesn: soroban_sdk::BytesN<32> = sha.into();
    let current_hash = Bytes::from_slice(env, &sha_bytesn.to_array());

    // Create audit entry
    let entry = AuditEntry {
        id: entry_id,
        action: action.clone(),
        admin: admin.clone(),
        timestamp,
        data: data.clone(),
        previous_hash: previous_hash.clone(),
        current_hash: current_hash.clone(),
        ledger: current_ledger,
    };

    // Store entry
    env.storage()
        .persistent()
        .set(&DataKey::AuditEntry(entry_id), &entry);

    // Update head pointer to current hash
    env.storage()
        .persistent()
        .set(&DataKey::AuditLogHead, &current_hash);

    // Increment counter
    env.storage()
        .persistent()
        .set(&DataKey::AuditEntryCount, &entry_id);

    // Emit event
    AdminAuditEntryAppendedEvent {
        entry_id,
        action,
        admin: admin.clone(),
        timestamp,
        ledger: current_ledger,
    }
    .publish(env);
}

/// Encode an appeal payload for storage in the audit log.
///
/// Layout: `source (XDR) || epoch(4 LE) || disputed_amount(16 LE) || reason`.
/// The reason is appended verbatim so reviewers can inspect the full dispute.
fn encode_appeal(
    env: &Env,
    source: &Address,
    epoch: u32,
    disputed_amount: i128,
    reason: &Bytes,
) -> Bytes {
    let mut payload = Bytes::new(env);
    payload.append(&source.clone().to_xdr(env));
    payload.append(&Bytes::from_slice(env, &epoch.to_le_bytes()));
    payload.append(&Bytes::from_slice(env, &disputed_amount.to_le_bytes()));
    payload.append(reason);
    payload
}

/// Submit an appeal disputing a measured contribution or payout.
///
/// The source records a dispute against a specific epoch and amount. The appeal
/// is appended to the audit chain as an `appeal` entry; the original measurement
/// record is left untouched. Returns the id of the appended appeal entry so the
/// reviewer can reference it when resolving.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `source` - The source disputing the measurement (must authorize).
/// * `epoch` - The epoch whose contribution/payout is disputed.
/// * `disputed_amount` - The amount the source believes is incorrect.
/// * `reason` - Human-readable justification for the dispute.
pub fn submit_reward_appeal(
    env: &Env,
    source: Address,
    epoch: u32,
    disputed_amount: i128,
    reason: Bytes,
) -> u32 {
    source.require_auth();

    let payload = encode_appeal(env, &source, epoch, disputed_amount, &reason);
    append_audit_entry(env, symbol_short!("appeal"), source, payload);

    get_audit_log_count(env)
}

/// Resolve a source reward appeal by appending a correction record.
///
/// Only the configured admin (reviewer) may approve a correction. The correction
/// is appended as a `correct` audit entry that references the original appeal
/// entry id and carries the approved amount. The original appeal entry is never
/// modified, so the audit trail preserves both the dispute and its resolution.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `appeal_id` - The audit entry id of the appeal being resolved.
/// * `approved_amount` - The corrected amount approved by the reviewer.
/// * `note` - Reviewer justification recorded alongside the correction.
///
/// # Panics
///
/// Panics with `ErrorCode::Unauthorized` if the caller is not the admin, or with
/// `ErrorCode::InvalidInput` if `appeal_id` does not reference an existing appeal.
pub fn resolve_reward_appeal(env: &Env, appeal_id: u32, approved_amount: i128, note: Bytes) -> u32 {
    let admin = get_admin(env);
    admin.require_auth();

    // The referenced entry must exist and be an appeal; corrections cannot be
    // chained onto arbitrary or missing records.
    let appeal: AuditEntry = match env
        .storage()
        .persistent()
        .get::<_, AuditEntry>(&DataKey::AuditEntry(appeal_id))
    {
        Some(entry) => entry,
        None => panic_with_error!(env, ErrorCode::InvalidInput),
    };
    if appeal.action != symbol_short!("appeal") {
        panic_with_error!(env, ErrorCode::InvalidInput);
    }

    // Encode correction: appeal_id(4 LE) || approved_amount(16 LE) || note.
    let mut payload = Bytes::new(env);
    payload.append(&Bytes::from_slice(env, &appeal_id.to_le_bytes()));
    payload.append(&Bytes::from_slice(env, &approved_amount.to_le_bytes()));
    payload.append(&note);

    append_audit_entry(env, symbol_short!("correct"), admin, payload);

    get_audit_log_count(env)
}

/// Get audit log entries within a range.
///
/// Retrieves audit entries starting from `from_id` up to `limit` entries.
/// Returns entries in chronological order (oldest first).
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `from_id` - Starting entry ID (inclusive). Use 0 to start from the beginning.
/// * `limit` - Maximum number of entries to return (0 = no limit, capped at 500).
///
/// # Returns
///
/// Ordered list of audit entries.
pub fn get_admin_audit_log(env: &Env, from_id: u32, limit: u32) -> Vec<AuditEntry> {
    let effective_limit = if limit == 0 || limit > 500 {
        500
    } else {
        limit
    };
    let start_id = if from_id == 0 { 1 } else { from_id };

    let entry_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::AuditEntryCount)
        .unwrap_or(0);

    let mut results = Vec::new(env);
    let mut i = start_id;
    let mut returned = 0u32;

    while i <= entry_count && returned < effective_limit {
        if let Some(entry) = env
            .storage()
            .persistent()
            .get::<_, AuditEntry>(&DataKey::AuditEntry(i))
        {
            results.push_back(entry);
            returned += 1;
        }
        i += 1;
    }

    results
}

/// Hash an audit entry's contents exactly as [`append_audit_entry`] does.
fn entry_hash(env: &Env, entry: &AuditEntry) -> Bytes {
    let mut entry_data = Bytes::new(env);
    entry_data.append(&entry.previous_hash);
    entry_data.append(&entry.data);
    entry_data.append(&Bytes::from_slice(env, &entry.timestamp.to_le_bytes()));
    entry_data.append(&Bytes::from_slice(env, &entry.id.to_le_bytes()));
    let sha: soroban_sdk::BytesN<32> = env.crypto().sha256(&entry_data).into();
    Bytes::from_slice(env, &sha.to_array())
}

/// Governance event completeness audit (#659).
///
/// Walks every admin audit entry and counts, separately:
/// * **missing** entries (absent from storage),
/// * **malformed** entries (stored hash does not match the entry contents),
/// * **unlinked** entries (`previous_hash` is not the prior entry's hash).
///
/// Unlike [`verify_audit_chain`], it does not stop at the first failure, so an
/// operator sees every problem at once.
pub fn audit_governance_events(env: &Env) -> crate::types::GovernanceAuditReport {
    let count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::AuditEntryCount)
        .unwrap_or(0);
    let (mut missing, mut malformed, mut unlinked) = (0u32, 0u32, 0u32);
    // `None` after a missing entry: the next link cannot be checked, and is
    // not double-counted.
    let mut prev: Option<Bytes> = Some(Bytes::new(env));
    for i in 1..=count {
        match env
            .storage()
            .persistent()
            .get::<_, AuditEntry>(&DataKey::AuditEntry(i))
        {
            None => {
                missing += 1;
                prev = None;
            }
            Some(entry) => {
                if entry_hash(env, &entry) != entry.current_hash {
                    malformed += 1;
                }
                if let Some(p) = &prev {
                    if entry.previous_hash != *p {
                        unlinked += 1;
                    }
                }
                prev = Some(entry.current_hash);
            }
        }
    }
    crate::types::GovernanceAuditReport {
        complete: missing == 0 && malformed == 0 && unlinked == 0,
        total_events: count,
        missing_events: missing,
        malformed_events: malformed,
        unlinked_events: unlinked,
    }
}

/// Test-only tampering hooks for the governance audit tests. `back` selects an
/// entry counted from the newest (`0` = most recent).
#[cfg(test)]
pub mod tamper {
    use super::*;

    fn target(env: &Env, back: u32) -> u32 {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AuditEntryCount)
            .unwrap_or(0);
        count.saturating_sub(back)
    }

    fn stash_key(id: u32) -> (Symbol, u32) {
        (symbol_short!("tst_drop"), id)
    }

    pub fn drop_event(env: &Env, back: u32) {
        let id = target(env, back);
        let key = DataKey::AuditEntry(id);
        if let Some(entry) = env.storage().persistent().get::<_, AuditEntry>(&key) {
            env.storage().temporary().set(&stash_key(id), &entry);
            env.storage().persistent().remove(&key);
        }
    }

    pub fn restore_event(env: &Env, back: u32) {
        let id = target(env, back);
        if let Some(entry) = env
            .storage()
            .temporary()
            .get::<_, AuditEntry>(&stash_key(id))
        {
            env.storage()
                .persistent()
                .set(&DataKey::AuditEntry(id), &entry);
            env.storage().temporary().remove(&stash_key(id));
        }
    }

    pub fn corrupt_event(env: &Env, back: u32) {
        let key = DataKey::AuditEntry(target(env, back));
        if let Some(mut entry) = env.storage().persistent().get::<_, AuditEntry>(&key) {
            entry.data.append(&Bytes::from_slice(env, b"tampered"));
            env.storage().persistent().set(&key, &entry);
        }
    }

    /// Breaks the link to the previous entry but keeps the entry's own hash
    /// consistent, so only the link check fails.
    pub fn unlink_event(env: &Env, back: u32) {
        let key = DataKey::AuditEntry(target(env, back));
        if let Some(mut entry) = env.storage().persistent().get::<_, AuditEntry>(&key) {
            entry.previous_hash = Bytes::from_slice(env, b"not-the-previous-hash");
            entry.current_hash = entry_hash(env, &entry);
            env.storage().persistent().set(&key, &entry);
        }
    }
}

/// Verify the integrity of the audit chain.
///
/// Walks through the entire audit log and verifies that each entry's hash
/// matches the hash of (previous_hash || entry_data). Returns true if the
/// chain is intact, false if tampering is detected.
///
/// WARNING: This operation is O(n) where n is the number of audit entries.
/// Use sparingly in time-critical paths.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
///
/// # Returns
///
/// `true` if the audit chain is valid, `false` if tampering is detected.
pub fn verify_audit_chain(env: &Env) -> bool {
    let entry_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::AuditEntryCount)
        .unwrap_or(0);

    if entry_count == 0 {
        return true; // Empty chain is valid
    }

    let mut previous_hash: Bytes = Bytes::new(env);

    for i in 1..=entry_count {
        if let Some(entry) = env
            .storage()
            .persistent()
            .get::<_, AuditEntry>(&DataKey::AuditEntry(i))
        {
            // Verify previous hash matches expected
            if entry.previous_hash != previous_hash {
                return false; // Chain broken or unexpected previous hash
            }

            // Recompute hash over the same fields used when appending
            let mut entry_data = Bytes::new(env);
            entry_data.append(&entry.previous_hash);
            entry_data.append(&entry.data);
            entry_data.append(&Bytes::from_slice(env, &entry.timestamp.to_le_bytes()));
            entry_data.append(&Bytes::from_slice(env, &entry.id.to_le_bytes()));

            let recomputed_sha = env.crypto().sha256(&entry_data);
            let recomputed_bytesn: soroban_sdk::BytesN<32> = recomputed_sha.into();
            let recomputed_hash = Bytes::from_slice(env, &recomputed_bytesn.to_array());
            if recomputed_hash != entry.current_hash {
                return false; // Hash mismatch
            }

            previous_hash = entry.current_hash.clone();
        } else {
            return false; // Missing entry
        }
    }

    true
}

/// Get the total number of audit entries in the log.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
///
/// # Returns
///
/// The total count of audit entries.
pub fn get_audit_log_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AuditEntryCount)
        .unwrap_or(0)
}

/// Get the current audit log head hash.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
///
/// # Returns
///
/// The SHA-256 hash of the most recent audit entry, or empty Bytes if log is empty.
pub fn get_audit_log_head(env: &Env) -> Bytes {
    env.storage()
        .persistent()
        .get(&DataKey::AuditLogHead)
        .unwrap_or_else(|| Bytes::new(env))
}

// --- Treasury spend authorization, scorecards, governance audit and rewards ---

/// Budget category a treasury spend is charged against.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
#[allow(dead_code)]
pub enum BudgetCategory {
    Operations = 0,
    Development = 1,
    Security = 2,
    Marketing = 3,
    Reserve = 4,
}

/// Outcome of a treasury spend authorization attempt.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum SpendOutcome {
    Approved = 0,
    RejectedUnapproved = 1,
    RejectedOverBudget = 2,
    RejectedMissingEvidence = 3,
    RejectedInvalidAmount = 4,
}

/// A single treasury spend request with its authorization evidence.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasurySpendRequest {
    pub proposal_id: Symbol,
    pub category: BudgetCategory,
    pub amount: i128,
    pub approver: Address,
    pub approved: bool,
    pub payment_evidence: String,
}

/// Traceable record retained for every spend attempt.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasurySpendRecord {
    pub proposal_id: Symbol,
    pub category: BudgetCategory,
    pub amount: i128,
    pub approver: Address,
    pub payment_evidence: String,
    pub outcome: SpendOutcome,
    pub timestamp: u64,
}

const TREASURY_RECORDS: Symbol = symbol_short!("TREASREC");
const TREASURY_BUDGET: Symbol = symbol_short!("TREASBUD");

fn records_key(env: &Env) -> Symbol {
    let _ = env;
    TREASURY_RECORDS
}

fn budget_key(env: &Env, category: BudgetCategory) -> (Symbol, u32) {
    let _ = env;
    (TREASURY_BUDGET, category as u32)
}

/// Set the remaining budget for a category. Used by treasury governance.
pub fn set_category_budget(env: &Env, category: BudgetCategory, budget: i128) {
    env.storage()
        .persistent()
        .set(&budget_key(env, category), &budget);
}

/// Read the remaining budget for a category (0 if unset).
pub fn category_budget(env: &Env, category: BudgetCategory) -> i128 {
    env.storage()
        .persistent()
        .get(&budget_key(env, category))
        .unwrap_or(0)
}

/// Append a spend record to the traceable audit trail.
fn append_record(env: &Env, record: TreasurySpendRecord) {
    let key = records_key(env);
    let mut records: Vec<TreasurySpendRecord> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    records.push_back(record);
    env.storage().persistent().set(&key, &records);
}

/// Retrieve all retained treasury spend records.
pub fn treasury_spend_records(env: &Env) -> Vec<TreasurySpendRecord> {
    env.storage()
        .persistent()
        .get(&records_key(env))
        .unwrap_or_else(|| Vec::new(env))
}

/// Authorize a non-routine treasury spend.
///
/// Requires a proposal reference, budget category, approval, and payment
/// evidence. Rejects unapproved, over-budget, evidence-less, or non-positive
/// amounts. Every attempt is recorded, including rejections, so the decision
/// remains traceable.
///
/// Returns the outcome; on `Approved` the category budget is decremented.
pub fn authorize_treasury_spend(env: &Env, request: TreasurySpendRequest) -> SpendOutcome {
    let outcome = evaluate_spend(env, &request);

    if outcome == SpendOutcome::Approved {
        let remaining = category_budget(env, request.category);
        env.storage().persistent().set(
            &budget_key(env, request.category),
            &(remaining - request.amount),
        );
    }

    append_record(
        env,
        TreasurySpendRecord {
            proposal_id: request.proposal_id,
            category: request.category,
            amount: request.amount,
            approver: request.approver,
            payment_evidence: request.payment_evidence,
            outcome,
            timestamp: env.ledger().timestamp(),
        },
    );

    outcome
}

fn evaluate_spend(env: &Env, request: &TreasurySpendRequest) -> SpendOutcome {
    if request.amount <= 0 {
        return SpendOutcome::RejectedInvalidAmount;
    }
    if !request.approved {
        return SpendOutcome::RejectedUnapproved;
    }
    if request.payment_evidence.is_empty() {
        return SpendOutcome::RejectedMissingEvidence;
    }
    if request.amount > category_budget(env, request.category) {
        return SpendOutcome::RejectedOverBudget;
    }
    SpendOutcome::Approved
}

/// Explicit window over which a conformance scorecard is computed (#643).
///
/// Both a ledger range and a timestamp range are recorded so that a scorecard
/// can be reproduced exactly from retained events. `from_ledger`/`to_ledger`
/// are inclusive; `from_timestamp`/`to_timestamp` are inclusive seconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScorecardWindow {
    pub from_ledger: u32,
    pub to_ledger: u32,
    pub from_timestamp: u64,
    pub to_timestamp: u64,
}

/// Per-source submission conformance scorecard (#643).
///
/// Reports schema validity, timestamp validity, submission continuity, and
/// accepted/rejected counts for a single source over an explicit window.
///
/// `has_data` distinguishes "no retained events in window" (missing data)
/// from "events present but zero failures" (a clean scorecard). When
/// `has_data` is false, all counts are zero and must not be read as a pass.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceConformanceScorecard {
    pub source: Address,
    pub window: ScorecardWindow,
    /// Whether any retained events for this source fall inside the window.
    pub has_data: bool,
    /// Total submissions observed for this source within the window.
    pub total_submissions: u32,
    /// Submissions whose payload passed schema validation.
    pub schema_valid: u32,
    /// Submissions whose payload failed schema validation.
    pub schema_invalid: u32,
    /// Submissions whose timestamp was within the accepted skew.
    pub timestamp_valid: u32,
    /// Submissions whose timestamp was outside the accepted skew.
    pub timestamp_invalid: u32,
    /// Accepted submissions (schema + timestamp valid).
    pub accepted: u32,
    /// Rejected submissions (any validity failure).
    pub rejected: u32,
    /// Largest gap (in ledgers) between consecutive submissions in the window.
    pub max_continuity_gap: u32,
    /// Number of continuity gaps that exceeded the expected cadence.
    pub continuity_breaks: u32,
}

/// Compute a per-source submission conformance scorecard (#643).
///
/// Scans retained audit entries for `source` whose ledger falls within
/// `[window.from_ledger, window.to_ledger]` and whose timestamp falls within
/// `[window.from_timestamp, window.to_timestamp]`. Results are deterministic
/// and reproducible from the retained audit log alone.
///
/// A submission is considered accepted when its payload is schema-valid and
/// its timestamp is valid; otherwise it is rejected. Continuity is measured
/// as the ledger gap between consecutive submissions; a gap larger than
/// `max_expected_gap` counts as a continuity break.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `source` - The source address to score.
/// * `window` - The explicit ledger/timestamp window to evaluate.
/// * `max_expected_gap` - Maximum acceptable ledger gap between submissions.
///
/// # Returns
///
/// A `SourceConformanceScorecard` with `has_data` set to false when no
/// retained events for `source` fall inside the window.
pub fn compute_source_conformance_scorecard(
    env: &Env,
    source: Address,
    window: ScorecardWindow,
    max_expected_gap: u32,
) -> SourceConformanceScorecard {
    let entry_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::AuditEntryCount)
        .unwrap_or(0);

    let mut scorecard = SourceConformanceScorecard {
        source: source.clone(),
        window: window.clone(),
        has_data: false,
        total_submissions: 0,
        schema_valid: 0,
        schema_invalid: 0,
        timestamp_valid: 0,
        timestamp_invalid: 0,
        accepted: 0,
        rejected: 0,
        max_continuity_gap: 0,
        continuity_breaks: 0,
    };

    let mut last_ledger: Option<u32> = None;

    for i in 1..=entry_count {
        let entry = match env
            .storage()
            .persistent()
            .get::<_, AuditEntry>(&DataKey::AuditEntry(i))
        {
            Some(e) => e,
            None => continue,
        };

        if entry.admin != source {
            continue;
        }
        if entry.ledger < window.from_ledger || entry.ledger > window.to_ledger {
            continue;
        }
        if entry.timestamp < window.from_timestamp || entry.timestamp > window.to_timestamp {
            continue;
        }

        scorecard.has_data = true;
        scorecard.total_submissions += 1;

        // Schema validity: a non-empty payload is treated as schema-valid.
        let schema_ok = !entry.data.is_empty();
        if schema_ok {
            scorecard.schema_valid += 1;
        } else {
            scorecard.schema_invalid += 1;
        }

        // Timestamp validity: must be non-zero and within the window bounds.
        let timestamp_ok = entry.timestamp != 0
            && entry.timestamp >= window.from_timestamp
            && entry.timestamp <= window.to_timestamp;
        if timestamp_ok {
            scorecard.timestamp_valid += 1;
        } else {
            scorecard.timestamp_invalid += 1;
        }

        if schema_ok && timestamp_ok {
            scorecard.accepted += 1;
        } else {
            scorecard.rejected += 1;
        }

        // Submission continuity: gap between consecutive submissions.
        if let Some(prev) = last_ledger {
            let gap = entry.ledger.saturating_sub(prev);
            if gap > scorecard.max_continuity_gap {
                scorecard.max_continuity_gap = gap;
            }
            if gap > max_expected_gap {
                scorecard.continuity_breaks += 1;
            }
        }
        last_ledger = Some(entry.ledger);
    }

    scorecard
}

/// Result of a governance event audit completeness check (#659).
///
/// Each variant identifies a specific class of governance audit failure so
/// operators can distinguish a missing event from a malformed or unlinked one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GovernanceAuditFailure {
    /// No failure detected; the cycle is complete and consistent.
    None,
    /// A privileged state change has no corresponding audit entry.
    MissingEvent,
    /// An audit entry exists but its action/admin/data is malformed.
    MalformedEvent,
    /// An audit entry is not linked to its predecessor (broken chain).
    UnlinkedEvent,
}

/// Audit a single governance cycle for event completeness (#659).
///
/// Reconciles privileged state changes with their emitted governance events
/// and approval records across an upgrade cycle. A cycle is described by the
/// `expected_actions` that must each have exactly one matching audit entry,
/// the `expected_admin` that must have authorized each entry, and the
/// `approval_count` of approval records that must be present.
///
/// The check fails (returns a non-`None` failure) when:
/// - a privileged state change has no matching audit entry (`MissingEvent`),
/// - an audit entry is malformed (empty action, zero timestamp, or wrong
///   admin) (`MalformedEvent`),
/// - the audit chain is broken or an entry is unlinked from its predecessor
///   (`UnlinkedEvent`),
/// - the number of approval records does not match the number of privileged
///   state changes (`MissingEvent`).
///
/// An empty cycle (no expected actions, no approvals) is considered complete.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `expected_actions` - Actions that must each appear in the audit log.
/// * `expected_admin` - The admin that must have authorized each entry.
/// * `approval_count` - Number of approval records recorded for the cycle.
///
/// # Returns
///
/// `GovernanceAuditFailure::None` when the cycle is complete and consistent.
pub fn audit_governance_cycle(
    env: &Env,
    expected_actions: &Vec<Symbol>,
    expected_admin: &Address,
    approval_count: u32,
) -> GovernanceAuditFailure {
    // Empty cycle: nothing to reconcile, trivially complete.
    if expected_actions.is_empty() && approval_count == 0 {
        return GovernanceAuditFailure::None;
    }

    // The chain must be intact before we can trust any entry linkage.
    if !verify_audit_chain(env) {
        return GovernanceAuditFailure::UnlinkedEvent;
    }

    let entry_count = get_audit_log_count(env);

    // Every privileged state change must have a matching approval record.
    if approval_count != expected_actions.len() {
        return GovernanceAuditFailure::MissingEvent;
    }

    // Collect the actions actually present in the audit log.
    let mut seen: Vec<Symbol> = Vec::new(env);
    for i in 1..=entry_count {
        let entry: AuditEntry = match env.storage().persistent().get(&DataKey::AuditEntry(i)) {
            Some(e) => e,
            None => return GovernanceAuditFailure::MissingEvent,
        };

        // Malformed: empty action, zero timestamp, or wrong authorizing admin.
        if entry.action == symbol_short!("")
            || entry.timestamp == 0
            || entry.admin != *expected_admin
        {
            return GovernanceAuditFailure::MalformedEvent;
        }

        // Unlinked: entry id must be sequential and previous hash must match
        // the prior entry's current hash (already validated by chain check,
        // but re-assert the id linkage explicitly).
        if entry.id != i {
            return GovernanceAuditFailure::UnlinkedEvent;
        }

        seen.push_back(entry.action.clone());
    }

    // Every expected action must be present in the audit log.
    for expected in expected_actions.iter() {
        let mut found = false;
        for action in seen.iter() {
            if action == expected {
                found = true;
                break;
            }
        }
        if !found {
            return GovernanceAuditFailure::MissingEvent;
        }
    }

    GovernanceAuditFailure::None
}

/// Collect a redacted incident evidence bundle from the audit trail (#651).
///
/// Gathers the audit entries in `[from_id, to_id]` (inclusive) into a single
/// reproducible artifact. Each entry contributes its id, action, timestamp,
/// ledger, and hash-chain links so the incident can be replayed and verified
/// with `verify_audit_chain`. The admin address and raw `data` payload are
/// intentionally excluded so credentials and unrelated user data never leak
/// into the bundle.
///
/// The bundle is prefixed with a header carrying the source version, the
/// requested range, the current chain head, and whether the chain verifies.
///
/// # Arguments
///
/// * `env` - The Soroban execution environment.
/// * `from_id` - First audit entry id to include (0 = start from the beginning).
/// * `to_id` - Last audit entry id to include (0 = up to the current head).
/// * `source_version` - The source version string to stamp into the bundle.
///
/// # Returns
///
/// A redacted `Bytes` artifact suitable for attaching to an incident report.
pub fn collect_incident_bundle(
    env: &Env,
    from_id: u32,
    to_id: u32,
    source_version: Symbol,
) -> Bytes {
    let entry_count = get_audit_log_count(env);
    let start_id = if from_id == 0 { 1 } else { from_id };
    let end_id = if to_id == 0 { entry_count } else { to_id };

    let mut bundle = Bytes::new(env);

    // Header: magic tag, source version, range, head hash, chain validity.
    bundle.append(&Bytes::from_slice(env, b"INCIDENT_BUNDLE_V1"));
    bundle.append(&source_version.to_xdr(env));
    bundle.append(&Bytes::from_slice(env, &start_id.to_le_bytes()));
    bundle.append(&Bytes::from_slice(env, &end_id.to_le_bytes()));
    bundle.append(&get_audit_log_head(env));
    bundle.append(&Bytes::from_slice(
        env,
        &[if verify_audit_chain(env) { 1u8 } else { 0u8 }],
    ));

    // Body: redacted per-entry records (no admin address, no raw data).
    let mut i = start_id;
    while i <= end_id && i <= entry_count {
        if let Some(entry) = env
            .storage()
            .persistent()
            .get::<_, AuditEntry>(&DataKey::AuditEntry(i))
        {
            bundle.append(&Bytes::from_slice(env, &entry.id.to_le_bytes()));
            bundle.append(&entry.action.to_xdr(env));
            bundle.append(&Bytes::from_slice(env, &entry.timestamp.to_le_bytes()));
            bundle.append(&Bytes::from_slice(env, &entry.ledger.to_le_bytes()));
            bundle.append(&entry.previous_hash);
            bundle.append(&entry.current_hash);
        }
        i += 1;
    }

    bundle
}

/// Deterministic per-entry reward weight derived from retained event data.
///
/// The weight is a pure function of the entry's payload size (a quality metric
/// captured at append time), so replaying the same retained events yields the
/// same weight. Integer-only: no rounding is required.
pub fn reward_weight(entry: &AuditEntry) -> u64 {
    1 + (entry.data.len() as u64)
}

/// Compute the reproducible source reward for a single admin.
///
/// Iterates the retained audit entries in `[from_id, to_id]` (inclusive) and
/// sums `reward_weight` for entries whose `admin` matches `admin`. Entries for
/// other admins are skipped. Returns 0 when no entries match (empty/zero case).
///
/// Because the calculation is integer-only and depends solely on retained
/// entries, an independent replay over the same events reproduces the exact
/// allocation.
pub fn compute_source_reward(env: &Env, admin: &Address, from_id: u32, to_id: u32) -> u64 {
    let entry_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::AuditEntryCount)
        .unwrap_or(0);

    let start_id = if from_id == 0 { 1 } else { from_id };
    let end_id = if to_id == 0 || to_id > entry_count {
        entry_count
    } else {
        to_id
    };

    let mut total: u64 = 0;
    let mut i = start_id;
    while i <= end_id {
        if let Some(entry) = env
            .storage()
            .persistent()
            .get::<_, AuditEntry>(&DataKey::AuditEntry(i))
        {
            if &entry.admin == admin {
                total = total.saturating_add(reward_weight(&entry));
            }
        }
        i += 1;
    }

    total
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn resolving_appeal_appends_correction_without_overwriting_original() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let source = Address::generate(&env);
        env.storage().persistent().set(&DataKey::Admin, &admin);

        let reason = Bytes::from_slice(&env, b"payout undercounted");
        let appeal_id = submit_reward_appeal(&env, source.clone(), 7, 1_000, reason.clone());
        assert_eq!(appeal_id, 1);

        // Snapshot the original appeal record before resolution.
        let original: AuditEntry = env
            .storage()
            .persistent()
            .get(&DataKey::AuditEntry(appeal_id))
            .unwrap();

        let note = Bytes::from_slice(&env, b"approved after review");
        let correction_id = resolve_reward_appeal(&env, appeal_id, 1_500, note);
        assert_eq!(correction_id, 2);

        // Original appeal record is untouched.
        let after: AuditEntry = env
            .storage()
            .persistent()
            .get(&DataKey::AuditEntry(appeal_id))
            .unwrap();
        assert_eq!(after.id, original.id);
        assert_eq!(after.action, original.action);
        assert_eq!(after.data, original.data);
        assert_eq!(after.current_hash, original.current_hash);

        // Correction is a distinct appended entry referencing the appeal.
        let correction: AuditEntry = env
            .storage()
            .persistent()
            .get(&DataKey::AuditEntry(correction_id))
            .unwrap();
        assert_eq!(correction.action, symbol_short!("correct"));
        assert_eq!(correction.previous_hash, original.current_hash);

        // Chain remains valid with both records present.
        assert!(verify_audit_chain(&env));
        assert_eq!(get_audit_log_count(&env), 2);
    }
}
