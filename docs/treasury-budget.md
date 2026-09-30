# Treasury Budget vs Actual Reconciliation (#546)

Module: `contracts/price-oracle/src/treasury_budget.rs`

## Categories
`SourceRewards`, `Infrastructure`, `Security`, `Grants`, `Operations` (`ALL_CATEGORIES`).
`reconcile(period)` always returns one line per category, so coverage is complete by
construction.

## Flow
1. `set_budget(period, category, amount)` — planned allocation.
2. `record_spend(period, category, amount)` — accumulates actual spend and emits a
   public `tr_spend` event.
3. `reconcile(period)` — per category: `available = allocated + rollover_in`,
   `remainder`, `overrun`, and `alert`. Alerts emit `tr_alert`.
4. `close_period(period)` — applies rollover.

## Alert rule
Alert when spend occurs with zero available budget, or when
`overrun > ALERT_DEVIATION_BPS (10%) × available`.

## Rollover policy
Unspent remainder rolls into the **same category** of `period + 1`, capped at
`ROLLOVER_CAP_BPS` (50%) of that category's allocation. Anything above the cap returns to
the general treasury — it never silently vanishes; the cap is explicit here.

## Reproducibility
Budgets are in contract storage and spends are public `tr_spend` events, so anyone can
rebuild the reconciliation from chain data.
