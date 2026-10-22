# Production Admin Key Recovery Tabletop Exercise (#680)

## Overview

This procedure exercises suspected production admin key compromise, immediate signer revocation, contract pause decision-making, and the orderly restoration of trusted administration.

It bridges protocol security guarantees (`contracts/price-oracle/src/admin_compromise_tests.rs`) with production operational response, verifying that:
1. Revocation and containment meet defined response SLAs without exposing key material.
2. The contract pause decision tree isolates malicious transactions while preserving guardian recovery paths.
3. Chain-of-custody evidence is generated and cryptographically bound for post-incident review.

---

## Response Team Roles

| Role | Responsibility |
|---|---|
| **Incident Commander (IC)** | Leads triage, declares incident severity (P0), coordinates emergency pause decision. |
| **Security Lead** | Performs anomaly analysis, evaluates blast radius, validates compromised key fingerprints. |
| **Guardian Operators (Quorum: 2/3)** | Authenticate through offline hardware, sign recovery proposals, and verify candidate admin. |
| **Operations Lead** | Revokes signer access across bastions/CI/multisig, monitors network activity, logs evidence. |

---

## 5-Phase Tabletop Lifecycle

```
[Phase 1: Detection]
       │
       ▼
[Phase 2: Emergency Pause & Signer Revocation]
       │
       ▼
[Phase 3: Guardian Quorum & Recovery Proposal]
       │
       ▼
[Phase 4: Timelock & Veto Cooldown Window]
       │
       ▼
[Phase 5: Execute Recovery & State Reinstatement]
```

### Phase 1: Detection & Triage
- **Trigger**: Anomaly alert from on-chain event monitors (e.g. unannounced `set_max_price_deviation` or unauthorized asset parameter changes).
- **SLA Target**: Time-to-detect $\le$ 30 minutes (1,800s).
- **Actions**:
  1. Identify the rogue transaction hash and calling address.
  2. Confirm whether the transaction was authorized by the production key custodian.
  3. If unverified, declare P0 Security Incident.

### Phase 2: Signer Revocation & Contract Pause Decision
- **Pause Decision Tree**:
  - *Scenario A: Rogue prices or malicious configuration active* $\rightarrow$ **EXECUTE CONTRACT PAUSE IMMEDIATELY** (`pause()`).
    *Note*: `recovery_*` endpoints bypass pause checks (`check_not_paused` is omitted for guardian recovery functions), ensuring emergency recovery proceeds while paused.
  - *Scenario B: Administrative key leaked but no malicious state change yet* $\rightarrow$ Proceed directly to Guardian Quorum without halting feed consumers.
- **Revocation**:
  - Remove signer public key from all API bastion gateways, automated deploy pipelines, and multisig signers.

### Phase 3: Guardian Quorum & Recovery Proposal
- Guardians review the candidate rescue admin public address (`GA_RESCUE...`).
- A minimum threshold of 2-of-3 guardians submit on-chain approvals via `recovery_propose` / `recovery_approve`.
- The timelock countdown begins (`recovery_delay` ledgers).

### Phase 4: Timelock Validation & Adversary Neutralization
- The protocol guarantees:
  - Compromised admin **cannot** swap guardians during active recovery (`RecoveryAlreadyPending`).
  - Compromised admin **cannot** stretch delay (`MAX_RECOVERY_DELAY` bound).
  - Compromised admin gets only **one veto** per candidate.

### Phase 5: Restoration of Trusted Administration
- Once the timelock ledger height is reached, `recovery_execute` transfers `Admin` to the rescue key.
- The new admin performs configuration audit, rolls back rogue settings, and unpauses the contract (`unpause()`).
- All evidence hashes are sealed in the incident postmortem.

---

## Automated Tabletop Check

The tabletop is automated and repeatable via:

```bash
python -m services.admin_recovery.tabletop --scenario tabletop-prod-drill --assert-sla --out tabletop-evidence.json
```

Or via pytest:

```bash
python -m pytest services/admin_recovery -q
```
