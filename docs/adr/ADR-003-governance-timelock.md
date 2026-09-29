# ADR-003: Governance and Timelock Design

## Status

Accepted

## Context

Sensitive parameter changes should not take effect immediately. A delay protects consumers
from abrupt configuration shifts and gives operators time to react.

## Decision

- Timelock pending operations stored under `TlPendingOp(id)`.
- Configurable `TimelockDuration` in ledgers (default ~24 h).
- Priority tiers: `Urgent`, `Normal`, `LongTerm` with separate delay constants.
- Admin-only proposal; anyone can execute once the delay elapses.

## Consequences

- Admin operations are two-phase, increasing UX complexity.
- Emergency pause remains instant and bypasses timelock.
- Batch operations reuse the same pending-op queue for atomicity.
