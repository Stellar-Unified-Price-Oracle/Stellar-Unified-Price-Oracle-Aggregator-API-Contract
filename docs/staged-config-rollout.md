# Staged config rollout with automatic rollback (#531)

Module: `contracts/price-oracle/src/staged_rollout.rs`.

| Endpoint | Auth | Effect |
|----------|------|--------|
| `start_config_rollout(candidate, percent, gate)` | admin | Candidate applies to `percent`% of assets |
| `advance_config_rollout(percent)` | admin | Widen only (narrowing rejected) |
| `report_rollout_health(healthy)` | admin | Records a sample; auto-rolls back when the gate trips |
| `rollback_config_rollout()` | admin | Manual rollback |
| `complete_config_rollout()` | admin | Candidate becomes the baseline |
| `get_effective_config(asset)` / `get_config_rollout()` | none | Read |

## Health gate

Rollback fires when `samples >= min_samples` **and**
`failures * 10000 / samples > max_failure_bp`. `min_samples` prevents a
single bad sample from flapping the rollout.

## Mixed-configuration semantics

During a rollout an asset is on the candidate iff
`sha256(xdr(asset))[0..4] % 100 < percent`. The bucket is stable, and
`advance` can only widen, so an asset never flips back mid-rollout.
Consumers must read `get_effective_config(asset)` per asset rather than
assuming one global value.

## Atomicity

The baseline is never touched during a rollout; the candidate lives in one
storage entry. Rollback removes that entry in a single write, so all assets
revert together.

## Events

`RolloutEvent { phase: Started | Advanced | Completed | RolledBack, percent, samples, failures }`.
