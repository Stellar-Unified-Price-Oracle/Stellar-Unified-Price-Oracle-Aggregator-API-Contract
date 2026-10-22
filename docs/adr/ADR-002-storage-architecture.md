# ADR-002: Storage Architecture

## Status

Accepted

## Context

Contract state must remain accessible under Soroban’s persistent/temporary storage model
while minimizing TTL bump traffic and keeping read paths O(1).

## Decision

- Persistent storage for admin config, asset/source registries, and aggregate prices.
- Temporary storage for per-ledger price history snapshots.
- `LEDGER_THRESHOLD = 10_000` and `LEDGER_BUMP = 40_000` for hot entries.
- O(1) asset-membership index (`AssetRegistryIndex`) alongside ordered `AssetRegistry`.

## Consequences

- History entries auto-expire; query paths prune old entries explicitly.
- Registry lookups remain constant-time after lazy index migration.
- New keys follow `NamespaceVariant` naming in `DataKey` to avoid collisions.
