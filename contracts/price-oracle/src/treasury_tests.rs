//! Treasury spend authorization and evidence workflow tests (issue #671).
//!
//! Non-routine treasury expenditure must carry a proposal reference, a budget
//! category, an approval, and payment evidence. Over-budget or unapproved
//! expenses are rejected, and every attempt (accepted or rejected) leaves a
//! traceable audit record.

use soroban_sdk::{testutils::Address as _, Address, Env, String, Symbol, Vec};

use crate::{PriceOracle, PriceOracleClient};

/// Budget categories tracked by the treasury workflow.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BudgetCategory {
    Operations,
    Security,
    Grants,
}

impl BudgetCategory {
    fn as_symbol(self) -> Symbol {
        match self {
            BudgetCategory::Operations => Symbol::new(&Env::default(), "operations"),
            BudgetCategory::Security => Symbol::new(&Env::default(), "security"),
            BudgetCategory::Grants => Symbol::new(&Env::default(), "grants"),
        }
    }
}

/// A treasury spend request. Non-routine spends require all evidence fields.
struct SpendRequest {
    proposal: String,
    category: BudgetCategory,
    amount: i128,
    approved: bool,
    evidence: String,
}

/// Outcome of a spend attempt, retained for auditability.
#[derive(Clone, PartialEq, Eq, Debug)]
enum SpendOutcome {
    Accepted,
    RejectedMissingProposal,
    RejectedMissingEvidence,
    RejectedUnapproved,
    RejectedOverBudget,
    RejectedInvalidAmount,
}

/// Traceable audit record for a single spend attempt.
struct SpendRecord {
    proposal: String,
    category: Symbol,
    amount: i128,
    outcome: SpendOutcome,
}

/// In-memory treasury workflow used to exercise the authorization rules.
struct Treasury {
    budgets: Vec<(Symbol, i128)>,
    spent: Vec<(Symbol, i128)>,
    records: Vec<SpendRecord>,
}

impl Treasury {
    fn new(env: &Env) -> Self {
        Self {
            budgets: Vec::new(env),
            spent: Vec::new(env),
            records: Vec::new(env),
        }
    }

    fn set_budget(&mut self, env: &Env, category: BudgetCategory, limit: i128) {
        let key = category.as_symbol();
        let mut updated = Vec::new(env);
        let mut found = false;
        for (i, (c, v)) in self.budgets.iter().enumerate() {
            if c == key {
                updated.push_back((c, limit));
                found = true;
            } else {
                updated.push_back((c, v));
            }
            let _ = i;
        }
        if !found {
            updated.push_back((key, limit));
        }
        self.budgets = updated;
    }

    fn budget_for(&self, category: Symbol) -> i128 {
        for (c, v) in self.budgets.iter() {
            if c == category {
                return v;
            }
        }
        0
    }

    fn spent_for(&self, category: Symbol) -> i128 {
        for (c, v) in self.spent.iter() {
            if c == category {
                return v;
            }
        }
        0
    }

    fn record_spent(&mut self, env: &Env, category: Symbol, amount: i128) {
        let mut updated = Vec::new(env);
        let mut found = false;
        for (c, v) in self.spent.iter() {
            if c == category {
                updated.push_back((c, v + amount));
                found = true;
            } else {
                updated.push_back((c, v));
            }
        }
        if !found {
            updated.push_back((category, amount));
        }
        self.spent = updated;
    }

    fn push_record(&mut self, env: &Env, req: &SpendRequest, outcome: SpendOutcome) {
        self.records.push_back(SpendRecord {
            proposal: req.proposal.clone(),
            category: req.category.as_symbol(),
            amount: req.amount,
            outcome,
        });
        let _ = env;
    }

    /// Attempt a non-routine spend, enforcing authorization and evidence rules.
    fn try_spend(&mut self, env: &Env, req: SpendRequest) -> SpendOutcome {
        let outcome = if req.amount <= 0 {
            SpendOutcome::RejectedInvalidAmount
        } else if req.proposal.len() == 0 {
            SpendOutcome::RejectedMissingProposal
        } else if req.evidence.len() == 0 {
            SpendOutcome::RejectedMissingEvidence
        } else if !req.approved {
            SpendOutcome::RejectedUnapproved
        } else {
            let category = req.category.as_symbol();
            let remaining = self.budget_for(category) - self.spent_for(category);
            if req.amount > remaining {
                SpendOutcome::RejectedOverBudget
            } else {
                self.record_spent(env, category, req.amount);
                SpendOutcome::Accepted
            }
        };

        self.push_record(env, &req, outcome.clone());
        outcome
    }
}

fn setup() -> (Env, Treasury) {
    let env = Env::default();
    let treasury = Treasury::new(&env);
    (env, treasury)
}

fn request(env: &Env, amount: i128, approved: bool) -> SpendRequest {
    SpendRequest {
        proposal: String::from_str(env, "proposal-671"),
        category: BudgetCategory::Operations,
        amount,
        approved,
        evidence: String::from_str(env, "tx-hash-abc123"),
    }
}

#[test]
fn accepts_authorized_spend_within_budget() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 1_000);

    let outcome = treasury.try_spend(&env, request(&env, 400, true));

    assert_eq!(outcome, SpendOutcome::Accepted);
    assert_eq!(treasury.spent_for(BudgetCategory::Operations.as_symbol()), 400);
    assert_eq!(treasury.records.len(), 1);
    assert_eq!(treasury.records.get(0).unwrap().outcome, SpendOutcome::Accepted);
}

#[test]
fn accepts_spend_exactly_at_budget_boundary() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 500);

    let outcome = treasury.try_spend(&env, request(&env, 500, true));

    assert_eq!(outcome, SpendOutcome::Accepted);
    assert_eq!(treasury.spent_for(BudgetCategory::Operations.as_symbol()), 500);
}

#[test]
fn rejects_over_budget_expense_and_retains_record() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 500);

    let outcome = treasury.try_spend(&env, request(&env, 501, true));

    assert_eq!(outcome, SpendOutcome::RejectedOverBudget);
    assert_eq!(treasury.spent_for(BudgetCategory::Operations.as_symbol()), 0);
    assert_eq!(treasury.records.len(), 1);
    let record = treasury.records.get(0).unwrap();
    assert_eq!(record.outcome, SpendOutcome::RejectedOverBudget);
    assert_eq!(record.amount, 501);
    assert_eq!(record.proposal, String::from_str(&env, "proposal-671"));
}

#[test]
fn rejects_unapproved_expense_and_retains_record() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 1_000);

    let outcome = treasury.try_spend(&env, request(&env, 100, false));

    assert_eq!(outcome, SpendOutcome::RejectedUnapproved);
    assert_eq!(treasury.spent_for(BudgetCategory::Operations.as_symbol()), 0);
    assert_eq!(treasury.records.len(), 1);
    assert_eq!(treasury.records.get(0).unwrap().outcome, SpendOutcome::RejectedUnapproved);
}

#[test]
fn rejects_missing_proposal_or_evidence() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 1_000);

    let mut no_proposal = request(&env, 100, true);
    no_proposal.proposal = String::from_str(&env, "");
    assert_eq!(
        treasury.try_spend(&env, no_proposal),
        SpendOutcome::RejectedMissingProposal
    );

    let mut no_evidence = request(&env, 100, true);
    no_evidence.evidence = String::from_str(&env, "");
    assert_eq!(
        treasury.try_spend(&env, no_evidence),
        SpendOutcome::RejectedMissingEvidence
    );

    assert_eq!(treasury.records.len(), 2);
}

#[test]
fn rejects_zero_and_negative_amounts() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 1_000);

    assert_eq!(
        treasury.try_spend(&env, request(&env, 0, true)),
        SpendOutcome::RejectedInvalidAmount
    );
    assert_eq!(
        treasury.try_spend(&env, request(&env, -50, true)),
        SpendOutcome::RejectedInvalidAmount
    );
    assert_eq!(treasury.spent_for(BudgetCategory::Operations.as_symbol()), 0);
    assert_eq!(treasury.records.len(), 2);
}

#[test]
fn recovery_after_rejection_allows_valid_spend() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 500);

    assert_eq!(
        treasury.try_spend(&env, request(&env, 600, true)),
        SpendOutcome::RejectedOverBudget
    );
    assert_eq!(
        treasury.try_spend(&env, request(&env, 250, true)),
        SpendOutcome::Accepted
    );

    assert_eq!(treasury.spent_for(BudgetCategory::Operations.as_symbol()), 250);
    assert_eq!(treasury.records.len(), 2);
    assert_eq!(treasury.records.get(0).unwrap().outcome, SpendOutcome::RejectedOverBudget);
    assert_eq!(treasury.records.get(1).unwrap().outcome, SpendOutcome::Accepted);
}

#[test]
fn budgets_are_tracked_per_category() {
    let (env, mut treasury) = setup();
    treasury.set_budget(&env, BudgetCategory::Operations, 100);
    treasury.set_budget(&env, BudgetCategory::Security, 1_000);

    let mut security_spend = request(&env, 900, true);
    security_spend.category = BudgetCategory::Security;

    assert_eq!(treasury.try_spend(&env, security_spend), SpendOutcome::Accepted);
    assert_eq!(treasury.spent_for(BudgetCategory::Operations.as_symbol()), 0);
    assert_eq!(treasury.spent_for(BudgetCategory::Security.as_symbol()), 900);
}

#[test]
fn oracle_contract_initializes_for_treasury_workflow() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let contract_id = env.register(PriceOracle, ());
    let client = PriceOracleClient::new(&env, &contract_id);
    client.initialize(&admin);
}
