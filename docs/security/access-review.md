# Production Access Review and Least-Privilege Evidence (#691)

## Overview

Each quarter, a Core Maintainer inventories every human and automation identity
with access to the four production systems, removes anything beyond what its
role needs, and records evidence that the removed access no longer works.

| System | Covers |
|---|---|
| `DEPLOYMENT` | Deploy environments, the OIDC deploy roles from [secretless CI](../secretless-ci.md), upgrade signing |
| `MONITORING` | Dashboards, alerting, metrics push |
| `SOURCE` | The GitHub repository |
| `TREASURY` | Treasury multisig signing (`multisig.rs` governors) |

Levels, lowest first: `NONE`, `READ`, `TRIAGE`, `WRITE`, `ADMIN`.

---

## Role ceilings

A grant above the ceiling of the principal's roles is removed. The human roles
follow the [contributor ladder](../../CONTRIBUTING.md#least-privilege-permission-map).

| Role | Kind | DEPLOYMENT | MONITORING | SOURCE | TREASURY |
|---|---|---|---|---|---|
| `contributor` | human | — | — | READ | — |
| `triager` | human | — | — | TRIAGE | — |
| `maintainer` | human | — | READ | WRITE | — |
| `core-maintainer` | human | WRITE | READ | ADMIN | — |
| `treasury-signer` | human | — | — | — | WRITE |
| `on-call` | human | — | WRITE | — | — |
| `deploy-workflow` | automation | WRITE | — | — | — |
| `metrics-exporter` | automation | — | WRITE | — | — |
| `ci` | automation | — | — | READ | — |

Treasury signing is never implied by another role. A human inactive for more
than 90 days is reviewed one rung lower on the ladder, as in
CONTRIBUTING's inactivity expiry. Automation identities hold only automation
roles, and humans never hold them.

## Quarterly review

1. **Inventory.** Export who has what from each system (GitHub org and
   repository settings, deploy environment and cloud role bindings, monitoring
   users and tokens, multisig governor list). Record principals, roles, owners,
   last activity and grants in an inventory file. Record levels only, never
   credentials.
2. **Review.** Run the review. It prints the approved access list and the
   removals:
   ```bash
   python -m services.access_review.review --inventory inventory.json \
     --report security-artifacts/access-review.md --out security-artifacts/access-review.json
   ```
3. **Remove.** Remove each listed grant in the source system.
4. **Verify.** For every removal, try the removed access (for example, push to
   a protected branch, assume the deploy role, open a monitoring admin page) and
   record the result as a `revocation_checks` entry.
5. **Approve.** Each principal's owner, a human who is not the principal,
   approves its line of the access list in `approvals`.
6. **Re-run** the review. It passes only when every removal has a `DENIED`
   probe and every principal is approved, all dated on or after `started_at`.
   File the markdown report in an issue with the `access-change` label. The
   next review is due 92 days after `started_at`.

### Inventory format

```json
{
  "review_id": "2026-Q4",
  "started_at": "2026-10-01",
  "principals": [
    {"id": "<handle>", "kind": "human", "roles": ["maintainer"], "owner": "<core maintainer>",
     "last_active": "2026-09-15", "grants": {"SOURCE": "WRITE", "DEPLOYMENT": "WRITE"}}
  ],
  "approvals": [
    {"principal": "<handle>", "approved_by": "<core maintainer>", "approved_at": "2026-10-03"}
  ],
  "revocation_checks": [
    {"principal": "<handle>", "system": "DEPLOYMENT", "result": "DENIED", "checked_at": "2026-10-04"}
  ]
}
```

## What fails a review

- A removed grant without a probe dated in this review, or with a probe that
  was not `DENIED`.
- A principal whose access list is missing an approval, was approved by someone
  other than its owner, or was approved before the review started.
- A principal whose owner is missing, itself, or not a human in the inventory.
- Unknown roles, systems or levels, automation and human roles mixed, duplicate
  principals, or key material anywhere in the inventory.

## Drill and tests

```bash
# Review the seeded synthetic inventory (one excess grant of each kind)
python -m services.access_review.review --seed --report security-artifacts/access-review.md

python -m pytest services/access_review -q
```
