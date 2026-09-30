//! # Treasury budget versus actual spend reconciliation (#546)
//!
//! See `docs/treasury-budget.md`.
//!
//! Budgets are set per [`BudgetCategory`] per period. Every spend is recorded
//! with [`record_spend`] (and emits a public `tr_spend` event so the report is
//! reproducible from chain data). [`reconcile`] covers every category and
//! reports overruns, remainders and alerts; [`close_period`] applies the
//! explicit rollover policy: unspent funds carry into the next period's budget
//! for the same category, capped at [`ROLLOVER_CAP_BPS`] of its allocation.

use crate::errors::ErrorCode;
use soroban_sdk::{contracttype, panic_with_error, symbol_short, Env, Vec};

/// Deviation (bps of allocation) above which a category is alerted.
pub const ALERT_DEVIATION_BPS: i128 = 1_000;
/// Maximum rollover as bps of the category's own allocation.
pub const ROLLOVER_CAP_BPS: i128 = 5_000;

#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BudgetCategory {
    SourceRewards = 0,
    Infrastructure = 1,
    Security = 2,
    Grants = 3,
    Operations = 4,
}

pub const ALL_CATEGORIES: [BudgetCategory; 5] = [
    BudgetCategory::SourceRewards,
    BudgetCategory::Infrastructure,
    BudgetCategory::Security,
    BudgetCategory::Grants,
    BudgetCategory::Operations,
];

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CategoryLine {
    pub category: BudgetCategory,
    pub allocated: i128,
    pub rollover_in: i128,
    pub spent: i128,
    pub remainder: i128,
    pub overrun: i128,
    pub alert: bool,
}

#[contracttype]
#[derive(Clone)]
enum TreasuryKey {
    Budget(u32, BudgetCategory),
    Rollover(u32, BudgetCategory),
    Spent(u32, BudgetCategory),
}

fn get(env: &Env, k: &TreasuryKey) -> i128 {
    env.storage().persistent().get(k).unwrap_or(0)
}

pub fn set_budget(env: &Env, period: u32, category: BudgetCategory, amount: i128) {
    if amount < 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&TreasuryKey::Budget(period, category), &amount);
}

pub fn record_spend(env: &Env, period: u32, category: BudgetCategory, amount: i128) {
    if amount <= 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    let k = TreasuryKey::Spent(period, category);
    let total = get(env, &k) + amount;
    env.storage().persistent().set(&k, &total);
    env.events()
        .publish((symbol_short!("tr_spend"), period, category as u32), amount);
}

pub fn line(env: &Env, period: u32, category: BudgetCategory) -> CategoryLine {
    let allocated = get(env, &TreasuryKey::Budget(period, category));
    let rollover_in = get(env, &TreasuryKey::Rollover(period, category));
    let spent = get(env, &TreasuryKey::Spent(period, category));
    let available = allocated + rollover_in;
    let diff = available - spent;
    let (remainder, overrun) = if diff >= 0 { (diff, 0) } else { (0, -diff) };
    // Material deviation: any spend with no budget, or an overrun above
    // ALERT_DEVIATION_BPS of the available amount.
    let alert = if available == 0 {
        spent > 0
    } else {
        overrun * 10_000 > ALERT_DEVIATION_BPS * available
    };
    CategoryLine {
        category,
        allocated,
        rollover_in,
        spent,
        remainder,
        overrun,
        alert,
    }
}

/// Reconciliation over every category (completeness by construction).
pub fn reconcile(env: &Env, period: u32) -> Vec<CategoryLine> {
    let mut out = Vec::new(env);
    for c in ALL_CATEGORIES.iter() {
        let l = line(env, period, *c);
        if l.alert {
            env.events().publish(
                (symbol_short!("tr_alert"), period, *c as u32),
                (l.spent, l.overrun),
            );
        }
        out.push_back(l);
    }
    out
}

/// Close `period`: roll capped remainders into `period + 1`. Returns lines.
pub fn close_period(env: &Env, period: u32) -> Vec<CategoryLine> {
    let lines = reconcile(env, period);
    for l in lines.iter() {
        let carry = l.remainder.min(l.allocated * ROLLOVER_CAP_BPS / 10_000);
        env.storage()
            .persistent()
            .set(&TreasuryKey::Rollover(period + 1, l.category), &carry);
    }
    lines
}
