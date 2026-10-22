//! Audit log for treasury spend authorization and evidence workflow.
//!
//! Non-routine treasury expenditure must carry a proposal reference, a budget
//! category, an explicit approval, and payment evidence. Every spend attempt
//! (approved or rejected) is recorded so the decision remains traceable.

use soroban_sdk::{contracttype, symbol_short, Address, Env, String, Symbol, Vec};

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
    env.storage().persistent().set(&budget_key(env, category), &budget);
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
        env.storage()
            .persistent()
            .set(&budget_key(env, request.category), &(remaining - request.amount));
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
    if request.payment_evidence.len() == 0 {
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
        let entry: AuditEntry = match env
            .storage()
            .persistent()
            .get(&DataKey::AuditEntry(i))
        {
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
    bundle.append(&Bytes::from_slice(env, source_version.to_string().as_bytes()));
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
            bundle.append(&Bytes::from_slice(env, entry.action.to_string().as_bytes()));
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
        env.storage()
            .persistent()
            .set(&DataKey::Admin, &admin);

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
