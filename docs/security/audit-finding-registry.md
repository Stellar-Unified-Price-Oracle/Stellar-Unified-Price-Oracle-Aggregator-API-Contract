# External Audit Finding Registry (#688)

## Overview

Every finding from an external security audit is tracked in one file,
[`config/audit-findings.json`](../../config/audit-findings.json), from the moment
it is reported until a reviewer accepts the fix. A finding cannot be marked
`CLOSED` unless it links both the fix and the evidence that the fix was verified.

---

## Fields

| Field | Required | Meaning |
|---|---|---|
| `id` | always | Unique finding ID, e.g. `AUD-2026-001`. |
| `title` | always | One-line description. Do not paste exploit details or secrets. |
| `audit` | always | Which audit reported it (firm and engagement). |
| `severity` | always | `CRITICAL`, `HIGH`, `MEDIUM`, `LOW` or `INFORMATIONAL`, as rated by the auditor. |
| `owner` | always | Who is responsible for the fix. |
| `status` | always | `OPEN`, `FIXED` or `CLOSED` (see below). |
| `reported_at` | always | ISO-8601 date the finding was reported. |
| `fix_ref` | `FIXED`, `CLOSED` | GitHub pull request or commit URL of the fix. |
| `fix_release` | `CLOSED` | Release tag that ships the fix, e.g. `v0.4.2`. |
| `regression_test` | `CLOSED` | `path::test_name` of the test that fails without the fix. |
| `reviewer` | `CLOSED` | Who accepted the fix. Must not be the `owner`. |
| `accepted_at` | `CLOSED` | ISO-8601 date of acceptance, not before `reported_at`. |

## Lifecycle

```
OPEN ──(fix merged, fix_ref linked)──► FIXED ──(released, test linked, reviewer accepts)──► CLOSED
```

| Status | Rule enforced by the validator |
|---|---|
| `OPEN` | Required fields only. |
| `FIXED` | `fix_ref` is a GitHub PR or commit URL. |
| `CLOSED` | `fix_ref`, `fix_release`, `regression_test`, `reviewer` and `accepted_at` are all present. The regression test file exists and defines the named test (`fn` or `def`). The reviewer is not the owner. Acceptance is not dated before the report. |

The validator also rejects unknown severities and statuses, duplicate IDs, and
any entry containing Stellar secret seeds or private keys.

## Operator workflow

1. When an audit report arrives, add one `OPEN` entry per finding.
2. When the fix merges, set `status` to `FIXED` and link `fix_ref`.
3. After the fix ships, add `fix_release` and the `regression_test`, then ask a
   reviewer other than the owner (preferably the auditor) to confirm it.
4. The reviewer sets `reviewer`, `accepted_at` and `status: CLOSED` in a PR.

## Checks

```bash
# Validate the committed registry (exit 1 on any violation)
python -m services.audit_findings.registry --report security-artifacts/audit-findings.md

# Run the test suite
python -m pytest services/audit_findings -q
```

`test_committed_registry_is_valid` validates the real registry file, so a
finding closed without evidence fails the test suite.
