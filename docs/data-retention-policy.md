# Data Retention and Correction Trace Policy (#714)

Module: `contracts/price-oracle/src/retention_policy.rs`

## Purpose

Provides an explicit, on-chain, operator-configurable retention policy for all
categories of oracle data. Before this feature, retention was implicitly
determined by Soroban's storage TTL mechanism and the `max_history_length`
setting, with no documented guarantee about how long audit or correction records
would survive.

This feature makes retention periods explicit, reviewable, and enforced.

## API

| Endpoint | Description |
|---|---|
| `get_retention_policy() → DataRetentionPolicy` | Returns the active policy (or default if none set). |
| `set_retention_policy(policy)` | Sets the policy. Admin-only. |
| `apply_retention_policy(max_entries) → u32` | Sweeps audit log and extends TTLs. Admin-only. Returns count bumped. |

### `DataRetentionPolicy` struct

```rust
pub struct DataRetentionPolicy {
    /// Minimum ledgers price-history snapshots are retained.
    pub history_ledgers: u32,
    /// Minimum ledgers audit log entries (including rejected submissions) are retained.
    /// Must be >= history_ledgers.
    pub audit_ledgers: u32,
    /// Minimum ledgers price-correction revision chains are retained.
    /// Must be >= history_ledgers.
    pub correction_ledgers: u32,
    /// Ledger when this policy was last set.
    pub set_ledger: u32,
    /// Whether this policy is active.
    pub active: bool,
}
```

## Retention categories and rationale

| Category | Storage key | Default (ledgers) | ~Days at 5s/ledger | Rationale |
|---|---|---|---|---|
| Price history | `PriceHistory(asset, ledger)` | 1,036,800 | 60 days | Consumer time-series queries |
| Audit log / rejected submissions | `AuditEntry(id)` | 3,110,400 | 180 days | Dispute resolution, source quality review, governance |
| Price-correction traces | `PriceRevisions(asset)` | 3,110,400 | 180 days | Mandatory correction audit trail; must outlive history |

## Key invariant: corrections and audit records survive normal pruning

The policy enforces two ordering constraints on write:

```
audit_ledgers     >= history_ledgers
correction_ledgers >= history_ledgers
```

This means:

1. **A correction entry always outlives the history entry it replaced.** A
   consumer who fetched a historical price before a correction was applied can
   always look up the correction chain to see what changed and why.

2. **Rejected-submission audit records outlive the accepted entries from the same
   period.** An auditor reviewing a disputed aggregation can retrieve both the
   accepted sources and the rejected-source records for the same time window.

`set_retention_policy` enforces these constraints with
`ErrorCode::InvalidConfiguration` on violation.

## TTL enforcement

On every relevant write (new history entry, new audit entry, new correction),
the storage entry's TTL is extended to `max(retention_ledgers, LEDGER_BUMP)`
from the current ledger.

`apply_retention_policy(max_entries)` retroactively sweeps existing audit log
entries and bumps their TTLs. This is useful after:
- A policy change that increased `audit_ledgers`
- A storage migration
- An operator audit requiring confirmed availability

## Validation rules

```
history_ledgers     >= 1
audit_ledgers       >= 1
correction_ledgers  >= 1
audit_ledgers       >= history_ledgers
correction_ledgers  >= history_ledgers
```

Any violation panics with `ErrorCode::InvalidConfiguration`.

## Limits and caveats

- **Network TTL cap.** Soroban enforces a maximum persistent-entry TTL at the
  protocol level. Setting `audit_ledgers` above this cap does not extend entries
  beyond the network maximum. Operators SHOULD verify that their configured
  windows fall within the network's archival TTL budget.
- **No retroactive retrieval.** If an entry was already evicted before the
  policy was set, it cannot be recovered on-chain. Use `docs/backup-restore.md`
  for long-term off-chain archival.
- **`apply_retention_policy` is bounded.** Each call processes at most
  `max_entries` entries. For large audit logs, call it repeatedly.

## Security notes

The correction trace is append-only and hash-chained (`PriceRevision` entries
with `actor`, `reason`, `corrected = true`). No correction can be silently
undone; every change is permanently visible in the chain. The retention policy
guarantees this chain stays accessible for the configured window.

For the full correction model see `docs/price-revision-metrics.md`.
