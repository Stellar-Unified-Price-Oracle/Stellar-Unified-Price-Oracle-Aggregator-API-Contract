//! Treasury management for the price oracle contract.
//!
//! Handles fee collection, reserve accounting, and authorized expenditure.
//! Non-routine expenditure requires a proposal reference, a budget category,
//! an explicit approval, and payment evidence. Every spend attempt (approved
//! or rejected) is recorded in an append-only audit log so that the decision
//! is traceable.

use soroban_sdk::{contracttype, symbol_short, Address, Env, String, Symbol, Vec};

/// Storage keys for treasury state.
#[contracttype]
#[derive(Clone)]
pub enum TreasuryKey {
    Balance,
    Reserve,
    CategoryBudget(Symbol),
    CategorySpent(Symbol),
    AuditLog,
    NextAuditId,
}

/// Budget categories for treasury expenditure.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BudgetCategory {
    Operations,
    Development,
    Security,
    Marketing,
    Emergency,
}

impl BudgetCategory {
    pub fn as_symbol(&self) -> Symbol {
        match self {
            BudgetCategory::Operations => symbol_short!("ops"),
            BudgetCategory::Development => symbol_short!("dev"),
            BudgetCategory::Security => symbol_short!("sec"),
            BudgetCategory::Marketing => symbol_short!("mkt"),
            BudgetCategory::Emergency => symbol_short!("emg"),
        }
    }
}

/// A request to spend treasury funds.
#[contracttype]
#[derive(Clone)]
pub struct SpendRequest {
    /// Reference to the approved proposal authorizing this spend.
    pub proposal_ref: String,
    /// Budget category the spend draws from.
    pub category: BudgetCategory,
    /// Amount to spend (in the treasury's base unit).
    pub amount: i128,
    /// Address that must approve the spend.
    pub approver: Address,
    /// Evidence of payment (e.g. invoice hash / receipt reference).
    pub payment_evidence: String,
}

/// Outcome of a spend attempt, retained for auditability.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpendOutcome {
    Approved,
    RejectedUnapproved,
    RejectedOverBudget,
    RejectedInvalidAmount,
    RejectedMissingEvidence,
}

/// A single audit record for a spend attempt.
#[contracttype]
#[derive(Clone)]
pub struct SpendRecord {
    pub id: u64,
    pub proposal_ref: String,
    pub category: BudgetCategory,
    pub amount: i128,
    pub approver: Address,
    pub payment_evidence: String,
    pub outcome: SpendOutcome,
}

/// Error returned when a spend is rejected.
#[contracttype]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TreasuryError {
    Unapproved = 1,
    OverBudget = 2,
    InvalidAmount = 3,
    MissingEvidence = 4,
    InsufficientBalance = 5,
}

fn read_balance(env: &Env) -> i128 {
    env.storage()
        .instance()
        .get(&TreasuryKey::Balance)
        .unwrap_or(0)
}

fn write_balance(env: &Env, value: i128) {
    env.storage().instance().set(&TreasuryKey::Balance, &value);
}

fn read_category_budget(env: &Env, category: BudgetCategory) -> i128 {
    env.storage()
        .instance()
        .get(&TreasuryKey::CategoryBudget(category.as_symbol()))
        .unwrap_or(0)
}

fn read_category_spent(env: &Env, category: BudgetCategory) -> i128 {
    env.storage()
        .instance()
        .get(&TreasuryKey::CategorySpent(category.as_symbol()))
        .unwrap_or(0)
}

fn write_category_spent(env: &Env, category: BudgetCategory, value: i128) {
    env.storage()
        .instance()
        .set(&TreasuryKey::CategorySpent(category.as_symbol()), &value);
}

/// Set the budget for a category. Restricted to the treasury admin.
///
/// The caller is responsible for authenticating `admin` before invoking.
pub fn set_category_budget(env: &Env, category: BudgetCategory, budget: i128) {
    env.storage()
        .instance()
        .set(&TreasuryKey::CategoryBudget(category.as_symbol()), &budget);
}

/// Deposit funds into the treasury.
pub fn deposit(env: &Env, amount: i128) {
    let balance = read_balance(env);
    write_balance(env, balance + amount);
}

fn append_audit(env: &Env, record: SpendRecord) {
    let mut log: Vec<SpendRecord> = env
        .storage()
        .instance()
        .get(&TreasuryKey::AuditLog)
        .unwrap_or(Vec::new(env));
    log.push_back(record);
    env.storage().instance().set(&TreasuryKey::AuditLog, &log);
}

fn next_audit_id(env: &Env) -> u64 {
    let id: u64 = env
        .storage()
        .instance()
        .get(&TreasuryKey::NextAuditId)
        .unwrap_or(0);
    env.storage()
        .instance()
        .set(&TreasuryKey::NextAuditId, &(id + 1));
    id
}

/// Attempt a treasury spend.
///
/// Enforces the authorization and evidence workflow for non-routine
/// expenditure:
/// - the amount must be positive,
/// - a proposal reference and payment evidence must be present,
/// - the approver must authorize the spend,
/// - the spend must fit within the category budget and treasury balance.
///
/// Every attempt is recorded in the audit log, including rejected ones, so
/// that the decision remains traceable. Returns `Ok(())` on success or the
/// rejection reason on failure.
pub fn spend(env: &Env, request: SpendRequest) -> Result<(), TreasuryError> {
    let id = next_audit_id(env);

    let outcome = evaluate_spend(env, &request);

    append_audit(
        env,
        SpendRecord {
            id,
            proposal_ref: request.proposal_ref.clone(),
            category: request.category,
            amount: request.amount,
            approver: request.approver.clone(),
            payment_evidence: request.payment_evidence.clone(),
            outcome,
        },
    );

    match outcome {
        SpendOutcome::Approved => {
            let balance = read_balance(env);
            write_balance(env, balance - request.amount);
            let spent = read_category_spent(env, request.category);
            write_category_spent(env, request.category, spent + request.amount);
            Ok(())
        }
        SpendOutcome::RejectedUnapproved => Err(TreasuryError::Unapproved),
        SpendOutcome::RejectedOverBudget => Err(TreasuryError::OverBudget),
        SpendOutcome::RejectedInvalidAmount => Err(TreasuryError::InvalidAmount),
        SpendOutcome::RejectedMissingEvidence => Err(TreasuryError::MissingEvidence),
    }
}

fn evaluate_spend(env: &Env, request: &SpendRequest) -> SpendOutcome {
    if request.amount <= 0 {
        return SpendOutcome::RejectedInvalidAmount;
    }
    if request.proposal_ref.is_empty() || request.payment_evidence.is_empty() {
        return SpendOutcome::RejectedMissingEvidence;
    }
    if !request.approver.require_auth() {
        return SpendOutcome::RejectedUnapproved;
    }

    let budget = read_category_budget(env, request.category);
    let spent = read_category_spent(env, request.category);
    let remaining = budget - spent;
    if request.amount > remaining {
        return SpendOutcome::RejectedOverBudget;
    }

    let balance = read_balance(env);
    if request.amount > balance {
        return SpendOutcome::RejectedOverBudget;
    }

    SpendOutcome::Approved
}

/// Return the full audit log of spend attempts.
pub fn audit_log(env: &Env) -> Vec<SpendRecord> {
    env.storage()
        .instance()
        .get(&TreasuryKey::AuditLog)
        .unwrap_or(Vec::new(env))
}

/// Return the remaining budget for a category.
pub fn remaining_budget(env: &Env, category: BudgetCategory) -> i128 {
    read_category_budget(env, category) - read_category_spent(env, category)
}
