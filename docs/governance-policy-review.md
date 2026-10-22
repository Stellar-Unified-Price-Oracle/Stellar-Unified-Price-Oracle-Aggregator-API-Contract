# Governance Policy Review Cadence

This document establishes the review cadence for critical governance policies in the price oracle. Each policy has a named owner, a review date, a change history, and a supersession record. The generated registry (see [Registry](#registry)) makes overdue reviews visible and prevents an overdue policy from being reported as current.

## Policy register

| Policy ID | Policy | Owner | Review date | Status | Supersedes |
|-----------|--------|-------|-------------|--------|------------|
| GOV-001 | Oracle price feed authorization | oracle-maintainers | 2025-06-30 | current | — |
| GOV-002 | Admin key rotation and custody | security-team | 2025-06-30 | current | — |
| GOV-003 | Price deviation and circuit-breaker thresholds | oracle-maintainers | 2025-06-30 | current | — |
| GOV-004 | Alerting and incident escalation | oncall-owners | 2025-06-30 | current | — |
| GOV-005 | Audit log retention and access | security-team | 2025-06-30 | current | — |

Review dates are the last day a policy may be considered current. A policy whose review date has passed is **overdue** and must not be reported as current.

## Change history

Every change to a critical policy must record the date, the author, and a short description. Append entries; do not rewrite history.

### GOV-001 — Oracle price feed authorization
- 2025-01-15 — Initial policy established (owner: oracle-maintainers).

### GOV-002 — Admin key rotation and custody
- 2025-01-15 — Initial policy established (owner: security-team).

### GOV-003 — Price deviation and circuit-breaker thresholds
- 2025-01-15 — Initial policy established (owner: oracle-maintainers).

### GOV-004 — Alerting and incident escalation
- 2025-01-15 — Initial policy established (owner: oncall-owners).

### GOV-005 — Audit log retention and access
- 2025-01-15 — Initial policy established (owner: security-team).

## Supersession

When a policy is replaced, mark the old entry `superseded` and record the replacement in the `Supersedes` column of the new policy. A superseded policy is never reported as current, regardless of its review date.

| Superseded policy | Superseded on | Replaced by |
|-------------------|---------------|-------------|
| _(none yet)_ | — | — |

## Registry

The registry below is generated from the policy register. It is the authoritative view of which policies are current. A policy is reported as `current` only when it is not superseded **and** its review date has not passed; otherwise it is reported as `overdue`.

```
GOV-001  current   owner=oracle-maintainers  review=2025-06-30
GOV-002  current   owner=security-team        review=2025-06-30
GOV-003  current   owner=oracle-maintainers  review=2025-06-30
GOV-004  current   owner=oncall-owners        review=2025-06-30
GOV-005  current   owner=security-team        review=2025-06-30
```

### Overdue handling

- Any policy past its review date is emitted as `overdue` in the registry.
- An overdue policy cannot be reported as `current`; the generator must fail the check if it would otherwise do so.
- Overdue policies are surfaced in the registry output so reviewers can act before the next cadence window.

## Repeatable check

A repeatable check validates this document on every change:

1. Every critical policy has an owner and a review date.
2. Every policy has at least one change-history entry.
3. Superseded policies are marked and never reported as current.
4. No policy past its review date is reported as current in the registry.

Run the check as part of CI; a failing check blocks merge until the policy is reviewed and the registry regenerated.

## Review procedure

1. The owner reviews the policy before the review date.
2. The owner appends a change-history entry describing the outcome.
3. The owner updates the review date to the next cadence window.
4. The registry is regenerated and the repeatable check passes.

No secrets or production credentials are recorded in this document or its history.
