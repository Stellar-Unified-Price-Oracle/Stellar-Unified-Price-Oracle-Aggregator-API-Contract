# Release Rollback Decision and Communication Template (#730)

This document standardises two operational outputs during a degraded release of
`price-oracle`:

1. **Rollback Decision Record** — a structured go/no-go decision tied directly to
   the on-chain guards in `contracts/price-oracle/src/blue_green.rs`
   (`MAX_ROLLBACK_WINDOW = 120,960` ledgers, `DEFAULT_MIN_INTERVAL = 17,280`
   ledgers, `DEFAULT_QUORUM = 2`, `max_readable_schema = 2`) and the post-upgrade
   invariant sweep in `scripts/verify-deployment.sh`.
2. **Stakeholder Communication Templates** — pre-structured status updates for
   integrators, source operators, and public status channels so every notice
   reports verified hashes, ledger numbers, and consumer impact without leaking
   operator secrets.

Related runbooks: [`blue-green-upgrade.md`](../blue-green-upgrade.md),
[`version-matrix.md`](../version-matrix.md),
[`release-provenance.md`](../release-provenance.md),
[`privileged-transaction-review.md`](../security/privileged-transaction-review.md).

---

## 1. Rollback Decision Gate

Evaluate every check in order before authorising `./scripts/blue-green-upgrade.sh rollback`.
Any **STOP** row prohibits rolling back and requires a forward fix instead.

| # | Check | Source of Truth | Pass Condition | If Failed |
|---|---|---|---|---|
| 1 | Trigger confirmed | `scripts/verify-deployment.sh` or observed price incoherence | Reproducible failure or invariant violation recorded | Do not roll back; diagnose first |
| 2 | No schema-changing migration | `contracts/price-oracle/src/migration.rs` (`CURRENT_VERSION`) | Stored schema `<=` blue's `max_readable_schema` | **STOP** — roll forward with a fix |
| 3 | No open migration cursor | `get_migration_state()` on-chain | Returns `null` (`is_layout_coherent == true`) | **STOP** — resume `migrate_storage` first |
| 4 | Inside rollback window | `blue_green.rs` (`MAX_ROLLBACK_WINDOW`) | `current_ledger - blue_deployed_ledger <= 120,960` (~7 days) | **STOP** — prepare forward fix |
| 5 | Cooldown satisfied | `blue_green.rs` (`DEFAULT_MIN_INTERVAL`) | `current_ledger - last_rollback_ledger >= 17,280` (~24 h) | **STOP** — wait for cooldown or escalate |
| 6 | Blue artifact verified | `.blue-green/<CONTRACT_ID>.blue` & `interface/versions.json` | SHA-256 equals attested prior release binary | **STOP** — retrieve verified artifact |
| 7 | Quorum & review guard | `multisig.rs` + `services/tx_review/gate.py` | `>= 2` distinct non-self reviewers; `--payload`/`--approval` pass on `mainnet` | Collect remaining approvals before signing |

---

## 2. Rollback Decision Record Template

Copy into the incident log / release ticket before submitting the rollback transaction.

```markdown
## Rollback Decision Record

- **Incident / Ticket ID:** INC-YYYY-MM-DD-NN
- **Decision Timestamp (UTC):** YYYY-MM-DDTHH:MM:SSZ
- **Network & Contract ID:** mainnet / testnet — `C...`
- **Green Release (rolling back from):** `<tag>` | WASM SHA-256: `<64-char hex>`
- **Blue Target (rolling back to):** `<tag>` | WASM SHA-256: `<64-char hex>`
- **Stored Schema Version / Blue `max_readable_schema`:** `2 / 2`
- **Migration Cursor State (`get_migration_state`):** `null`
- **Decision:** `ROLLBACK` | `ROLL_FORWARD`
- **Trigger Summary:** <failing sweep section / incoherent price observation + ledger>
- **Quorum Approvers (min 2 distinct, non-requester):**
  - Requester: `@<handle>`
  - Reviewer 1: `@<handle>` (tx_review digest: `<payload_sha256>`)
  - Reviewer 2: `@<handle>` (tx_review digest: `<payload_sha256>`)
- **Rollback Transaction Hash & Ledger:** `<tx_hash>` @ ledger `<seq>`
- **Post-Rollback Verification (`scripts/verify-deployment.sh`):** `PASS` | `FAIL`
```

---

## 3. Communication Templates

### A. Initial Notice — Rollback Initiated (T+0)

```text
[STATUS: INVESTIGATING / ROLLBACK INITIATED] Stellar Unified Price Oracle (<network>)

At <YYYY-MM-DD HH:MM UTC> (ledger <seq>), post-upgrade verification detected <concise symptom>
on release <green_tag> (WASM <short_green_hash>).

Action in progress:
- Initiating verified blue-green rollback to <blue_tag> (WASM <short_blue_hash>).
- Quorum approvals and privileged transaction review checks are complete.
- Stored prices, registered assets, and source configurations remain preserved on-chain.

Consumer impact:
- Existing <blue_tag> SEP-40 / native read endpoints remain available.
- Any endpoints newly introduced in <green_tag> will be unavailable until a forward fix ships.

Next update: within 15 minutes or immediately upon post-rollback verification completion.
```

### B. Resolution Notice — Rollback Complete & Verified

```text
[STATUS: RESOLVED — ROLLBACK VERIFIED] Stellar Unified Price Oracle (<network>)

At <YYYY-MM-DD HH:MM UTC> (ledger <seq>), rollback to <blue_tag> completed on contract <CONTRACT_ID>.

Verification summary:
- Active WASM SHA-256: <blue_wasm_sha256>
- Rollback tx hash: <tx_hash>
- Post-rollback invariant sweep (scripts/verify-deployment.sh): PASS
- Migration state: null (coherent layout, schema v<N>, admin unchanged)

Follow-up:
- Incident review and forward-fix plan will be published under <incident_link>.
- `interface/versions.json` will be updated to reflect the active contract mapping.
```

### C. Alternative Notice — Rollback Blocked (Roll-Forward Required)

```text
[STATUS: MITIGATING — ROLL-FORWARD REQUIRED] Stellar Unified Price Oracle (<network>)

Release <green_tag> (WASM <short_green_hash>) encountered <concise symptom> at ledger <seq>.
Because <reason: e.g., schema version advanced beyond blue's max_readable_schema / rollback window elapsed>,
a downgrade rollback is blocked by contract safety guards to prevent storage desync.

Mitigation:
- Affected asset(s) / feature(s) paused or guarded via <specific action>.
- Forward fix <fix_tag> is being built, reviewed under the 2-reviewer tx_review gate, and prepared for upgrade.

Next update: within 30 minutes with target deployment ETA.
```
