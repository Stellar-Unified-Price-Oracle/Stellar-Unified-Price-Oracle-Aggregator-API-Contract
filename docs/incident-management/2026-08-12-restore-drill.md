# Postmortem: Game-day drill — point-in-time restore of off-chain state

| Field | Value |
|---|---|
| **Incident ID** | `INC-20260812-02` |
| **Date** | 2026-08-12 |
| **Severity** | P1 |
| **Status** | Final |
| **Incident commander** | `oracle-oncall` |
| **Duration** | 48 min (restore start to reconciled state) |
| **Runbook entries used** | RB-02 (docs/runbook.md) |
| **Postmortem owner** | `core-contracts` |

## 1. Summary

A game-day drill destroyed the off-chain submission cache and index for a
testnet deployment and restored them from the most recent encrypted backup using
`scripts/restore-offchain.sh`. The restore reproduced the known-good state
exactly, and the subsequent reconciliation against on-chain truth found and
rejected four submissions that had landed after the backup was taken, which is
the expected outcome for a point-in-time restore. Two gaps were found: the
reconciliation report is not wired to any alert, and the drill revealed that
source configuration had no automated backup coverage until the drill added it.

## 2. Impact

* **Consumer impact** — none. No consumer contract read from the testnet
  deployment, and the restored state matched the pre-destruction snapshot.
* **SLA impact** — none engaged. The drill ran inside a scheduled window and the
  deployment was not serving production traffic.
* **Financial impact** — none quantified.
* **On-chain footprint** — none. The contract was not modified at any point in
  the drill; this was an off-chain-only failure and recovery. Reconciliation
  confirmed the on-chain aggregate for all three assets matched the restored
  cache, so no `override_price` or repair call was required.

## 3. Timeline

| Time (UTC) | Event | Source of truth |
|---|---|---|
| 11:20 | Drill start. Off-chain submission cache and index destroyed (`rm -rf` on the state directory). | drill harness |
| 11:23 | `OracleSingleSourceOffline` fired: the restarted pipeline had no cached submissions to replay. | alert |
| 11:24 | First response. Escalated to RB-02 (P1) because aggregation had no inputs. | responder notes |
| 11:31 | `scripts/restore-offchain.sh --to <ts>` started against the newest verified backup; decryption prompt satisfied from the KMS-held key. | restore log |
| 11:38 | Restore completed: 4 812 submission records, 3 assets, 2 sources. Manifest checksum verified. | restore log |
| 11:44 | `scripts/reconcile-offchain.sh` run against on-chain truth. | reconciliation report |
| 11:48 | Reconciliation rejected 4 submissions newer than the backup, and confirmed the last 3 aggregates byte-for-byte. | reconciliation report |
| 12:08 | Drill declared resolved; 48 min total, against a 4 h P1 target. | incident commander |

## 4. Root cause

* **Trigger** — deliberate destruction of off-chain state.
* **Contributing factors** — (a) source configuration (the source registry and
  per-source credentials metadata) was not in the automated backup set until
  this drill added it, so a real destruction would have needed a manual
  reconstruction of which sources were registered; (b) reconciliation has no
  alert, so a restore that silently disagreed with on-chain truth would have
  been discovered by a consumer rather than by us.
* **What went well** — the backup was encrypted and access-controlled as
  designed, the manifest checksum caught nothing because there was nothing to
  catch, and the restore did not disturb the running contract at any point.
  The reconciliation step is what made the drill meaningful: it is the only
  thing that distinguishes "restored" from "restored correctly".

**Blamelessness.** No individual is named. The gap in the backup set is
attributed to the backup inventory, not to whoever last edited it.

## 5. What went wrong

1. **Source configuration was outside the automated backup set.** The inventory
   in `docs/backup-restore.md` listed it as manual. It is now automated, and
   the restore verified it.
2. **Reconciliation is a script with no alert.** Its exit status is only read by
   a human running it by hand. It should page, or at minimum emit a metric, when
   restored state disagrees with on-chain truth.
3. **No stated restore-time objective.** The drill took 48 min but nothing
   defined a target, so "slow" had no definition.

## 6. Action items

| # | Action | Owner | Due | Issue | Status |
|---|---|---|---|---|---|
| 1 | Move source configuration into the automated backup set and assert its presence in the restore manifest check | `core-contracts` | 2026-09-15 | #571 | Open |
| 2 | Emit `oracle_restore_reconcile_mismatch_total` from the reconciliation step and add a paging alert on it, mapped to a runbook entry | `oracle-oncall` | 2026-09-15 | #572 | Open |
| 3 | State a restore-time objective in `docs/backup-restore.md` (target and maximum), derived from the 48 min this drill measured | `core-contracts` | 2026-09-08 | #573 | Open |
| 4 | Run the restore drill again after actions 1 and 2 land, and attach the reconciliation report to this postmortem | `oracle-oncall` | 2026-10-01 | #574 | Open |

## 7. Detection and response assessment

* Detected by the alert pipeline 3 minutes after destruction — the restarted
  pipeline could not submit, which is the earliest possible signal.
* The page did not carry the runbook link; the responder found RB-02 by
  searching the runbook. Action item 2 in this repository's alert-to-runbook
  wiring (issue #527 follow-up) covers the link.
* Escalation was unambiguous here: one role, one hop.
* First response 4 min against the SLA §5 P1 target of 1 h; resolution 48 min
  against 4 h.

## 8. Review and sign-off

| Reviewer role | What they checked | Date |
|---|---|---|
| `oracle-secondary` | Timeline against the restore and reconciliation logs; action items 1 and 3 are verifiable | 2026-08-14 |
| `core-contracts` | The backup inventory in `docs/backup-restore.md` now matches what the drill actually backed up | 2026-08-15 |
