# Secret Scanning History & Exposure Response Procedure (#683)

## Overview

This procedure defines the operational response when credential material (e.g. Stellar secret seeds, AWS access keys, GitHub tokens, or generic API secrets) is discovered in git working trees, staged changes, or historical commits.

The core principle is **Revoke first, investigate second, and ensure zero secret values ever leak into public records, incident tickets, or chat logs.**

---

## Response Objectives & Core Invariants

1. **Immediate Revocation (< 15 min SLA)**: The moment an exposed secret is detected, it is presumed compromised and immediately invalidated at the upstream provider or on-chain signer registry before performing deep forensic analysis.
2. **Zero-Secret Public Invariant**: Raw secret values are strictly forbidden from appearing in incident logs, git commit messages, markdown post-mortems, security advisories, or chat threads. Only redacted fingerprints (`SABC...XYZ` or `AKIA...WXYZ`) or cryptographic hashes may be cited.
3. **Rotation Over History Rewriting**: Rewriting git history (`git filter-repo` / force-push) does not un-leak a credential because existing clones, forks, and provider mirrors retain the material. Remediation is achieved via upstream revocation and rotation.

---

## Operational Roles & Escalation

| Role | Primary Responsibility |
|---|---|
| **Security On-Call / First Responder** | Ingests finding, immediately issues credential revocation or coordinates with custodian. |
| **Incident Commander (IC)** | Declares incident tier (P0 for signer/deployer keys, P1 for CI/API tokens), coordinates team. |
| **Infrastructure / Custodian Lead** | Inspects provider audit logs (AWS CloudTrail, GitHub Audit, Stellar Horizon/Soroban RPC). |
| **Communications Lead** | Formats and dispatches redacted security advisories and partner notifications. |

---

## 5-Phase Secret Response Lifecycle

```
[Phase 1: Detection & Ingest]
       │
       ▼
[Phase 2: Immediate Credential Revocation (SLA < 15m)]
       │
       ▼
[Phase 3: Git History & Blast Radius Assessment]
       │
       ▼
[Phase 4: Tree Sanitization & Regression Guard]
       │
       ▼
[Phase 5: Redacted Notification & Post-Incident Review]
```

### Phase 1: Detection & Ingest
- **Detection Vectors**:
  - Pre-commit hook (`scanner.py --staged`)
  - CI test failure (`scanner.py --history`)
  - External vulnerability report / bug bounty
- **Actions**:
  1. Record the rule triggered (`SECRET-SOROBAN-SECRET-SEED`, `SECRET-AWS-ACCESS-KEY-ID`, etc.), file path, line number, and commit hash.
  2. Compute the redacted fingerprint using `redact_secret_value()`: retain $\le 4$ prefix characters and $\le 4$ suffix characters, replacing the middle with asterisks.
  3. Never paste or commit the raw value.

### Phase 2: Immediate Credential Revocation (SLA < 15m)
Execute provider-specific invalidation immediately:
- **Stellar Secret Seed / Signer Account**:
  - If used as an oracle admin or contract signer: initiate an emergency multisig transaction to set the signer weight to 0 or rotate the account master key via `set_options`.
  - Bump account sequence numbers to invalidate pending transaction envelopes.
- **AWS IAM Key (`AKIA...`)**:
  - Invalidate via AWS CLI: `aws iam update-access-key --access-key-id <KEY_ID> --status Inactive`.
  - Delete after validating that replacement service accounts are active.
- **GitHub PAT / OAuth Token (`ghp_...`, `github_pat_...`)**:
  - Revoke token in organization/user settings or via GitHub REST API: `DELETE /authorizations/{id}`.
- **Generic Service API Keys**:
  - Invalidate upstream key in third-party developer portal and flush environment variables.
- Obtain and record a **Revocation Receipt** (provider timestamp, confirmation ID, and audit tracking reference).

### Phase 3: Git History & Blast Radius Assessment
1. **Exposure Window Analysis**:
   - Determine introducing commit: `git log --all --find-object=<blob-sha> --format=%h --reverse`.
   - Calculate elapsed time between commit introduction and credential revocation.
2. **Blast Radius Evaluation**:
   - **Critical (Tier 1)**: Stellar admin keys, deployment keys. Audit on-chain transactions submitted by the key during the exposure window. Verify oracle aggregator parameters (`max_price_deviation`, asset quorums, fee recipient).
   - **High (Tier 2)**: AWS / CI / GitHub tokens. Audit CloudTrail and GitHub audit logs for unauthorized runner starts, artifact downloads, or repository setting changes.
   - **Medium (Tier 3)**: Read-only telemetry feeds, sandbox API tokens. Verify no rate-limiting or quota anomalies occurred.

### Phase 4: Tree Sanitization & Regression Guard
1. Purge the secret from the working tree in a new commit.
2. Add an automated scanner regression check.
3. If the material was in a public repository commit, evaluate git history cleanup (`git filter-repo`) solely to prevent subsequent clones from downloading stale blobs, while treating the key as permanently invalid.

### Phase 5: Redacted Notification & Post-Mortem
1. Dispatch internal operations alert with receipt ID and blast radius tier.
2. Generate public security advisory using `PublicIncidentRecord.to_markdown()`.
3. File formal post-mortem under `docs/incident-management/YYYY-MM-DD-secret-exposure.md` linking the zero-secret verified record.

---

## Automated Incident Response Engine

The repository provides an automated engine in `services/secret_scan/incident_response.py`:

```python
from services.secret_scan.incident_response import (
    SecretExposureFinding,
    execute_incident_response,
)

finding = SecretExposureFinding(
    finding_id="INC-2026-001",
    rule_id="SECRET-SOROBAN-SECRET-SEED",
    raw_secret=compromised_key_in_memory,
    file_path="config/staging_keys.json",
    line_number=14,
    detected_at=discovery_time,
    commit_hash="9c4c8c1",
)

report = execute_incident_response(finding)
assert report.zero_secret_verified is True
print(report.public_record.to_markdown())
```

### Verification & Testing
Execute the test suite to verify the end-to-end response procedure and zero-secret leak invariant:

```bash
python -m pytest services/secret_scan/test/test_incident_response.py -v
```
