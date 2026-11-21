# Instance-storage inventory

Audit for #798: every key written to Soroban **instance**
storage by the `price-oracle` contract, its value type, and its maximum size.

Instance storage is loaded in full on **every** invocation and shares a single
ledger entry (limited by the network's per-entry byte limit), so any value that
can grow without bound, or that is rarely read, does not belong there.

Scope: non-test code under `contracts/price-oracle/src`, found with
`grep -rn "storage().instance()"` and by following `.instance()` call chains
that rustfmt splits across lines.

## Inventory

| # | Key | Defined in | Value type | Max size | Bounded? | Written by | Read on |
|---|---|---|---|---|---|---|---|
| 1 | `DataKey::SubscriptionToken` | `subscription.rs` | `Address` | 1 `Address` (≈ 40 B XDR) | yes | `write_subscription_token` | every token-subscription call |
| 2 | `DataKey::ConsumerAccessMode` | `consumer_auth.rs` | `ConsumerAccessMode` (enum) | 4 B | yes | `write_access_mode` | every consumer-gated read (`read_access_mode`) |
| 3 | `Symbol("FEATFLAG")` | `feature_flags.rs` | `u64` bitmask | 8 B | yes (fixed width; 34 bits assigned) | `mark`, `mark_all` | `on` — hot path, called by most setters/getters |
| 4 | `DataKey::<any>` (generic) | `batch_storage.rs` | `Val` | caller-chosen | n/a — read-only probe (`has` / `get`), never writes | — | `read_instance` in the storage batch reader |
| 5 | `TreasuryKey::Balance` | `treasury.rs` | `i128` | 16 B | yes | `write_balance` | `read_balance` |
| 6 | `TreasuryKey::CategoryBudget(Symbol)` | `treasury.rs` | `i128` | 16 B × 5 categories = 80 B | yes (5 `BudgetCategory` variants) | `set_category_budget` | `read_category_budget` |
| 7 | `TreasuryKey::CategorySpent(Symbol)` | `treasury.rs` | `i128` | 16 B × 5 categories = 80 B | yes | `write_category_spent` | `read_category_spent` |
| 8 | `TreasuryKey::NextAuditId` | `treasury.rs` | `u64` | 8 B | yes | `next_audit_id` | `next_audit_id` |
| 9 | `TreasuryKey::AuditLog` | `treasury.rs` | `Vec<SpendRecord>` | **unbounded** | **no** | `append_audit` (push on every spend attempt) | `append_audit`, `audit_log` |

Keys used only by the in-file mock contracts in `reentrancy.rs` (`COUNT`,
`TARGET`, `REENTERED`, `next`) are test scaffolding that is not part of the
deployed contract and are excluded.

## Findings

1. **`TreasuryKey::AuditLog` is the only unbounded instance key.** `append_audit`
   reads the whole `Vec<SpendRecord>`, pushes one record, and writes the whole
   vector back, on every spend attempt (approved *and* rejected). Each
   `SpendRecord` holds two free-length `String`s (`proposal_ref`,
   `payment_evidence`), an `i128` and an `Address`, so a record is roughly
   100 B + the two strings. The log therefore grows monotonically, costs
   O(n) read/write per spend, and is loaded on every invocation of the contract
   whether or not treasury is touched. It should move to persistent storage as
   one entry per `id` (`TreasuryKey::AuditRecord(u64)`) with `NextAuditId` as the
   cursor; `audit_log` then becomes a paginated read.
2. **`TreasuryKey::CategoryBudget` / `CategorySpent` are rarely read** (only by
   treasury spends) but are small and bounded (160 B total). They are safe to
   leave, and are the second-best candidates to move if the instance entry ever
   approaches its limit.
3. **`FEATFLAG` is read on the hot path and is 8 B** — correct use of instance
   storage; keep it there.
4. **`SubscriptionToken` and `ConsumerAccessMode` are tiny, global,
   admin-set configuration** read on most calls — correct use of instance
   storage; keep them there.
5. **`treasury.rs` is not declared in `lib.rs`** (only `treasury_budget` is), so
   the treasury keys above are not compiled into the deployed contract today.
   The `AuditLog` finding applies as soon as the module is wired in; it is
   cheaper to fix the layout before wiring than after.

## Resolution (#878)

All treasury keys (`Balance`, `Reserve`, `CategoryBudget`, `CategorySpent`, `AuditLog`, and `NextAuditId`) have been moved to persistent storage. Feature flags are tightly packed into a single 64-bit integer (`u64`) under `FEATFLAG`.

As a result:
- Instance entry footprint has been reduced to **≤ 52 B** (strictly bounded to `SubscriptionToken`, `ConsumerAccessMode`, and `FEATFLAG`).
- Unbounded memory consumption in instance storage is eliminated.

## Summary

| Class | Keys | Total max size |
|---|---:|---|
| Bounded, hot path (kept in instance) | 3 (`SubscriptionToken`, `ConsumerAccessMode`, `FEATFLAG`) | ≈ 52 B |
| Unbounded & rarely read (moved to persistent, #878) | Treasury keys (`AuditLog`, `Balance`, etc.) | 0 B in instance |
