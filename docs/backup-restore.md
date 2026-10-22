# Backup and Point-in-Time Restore — Off-Chain Oracle State (#529)

On-chain state is durable: the network keeps it whether or not we do. The
off-chain pipeline that *feeds* it is not. Losing the submission cache, the
source configuration or the index can stop price publication while the contract
is perfectly healthy — the contract will faithfully aggregate whatever it is
given, and if nothing is given there is no aggregate.

This document covers the inventory, the automated backup, the restore
procedure, and the reconciliation that makes a restored state trustworthy
rather than merely present.

The toolchain is [`services/backup/backup.py`](../services/backup/backup.py).

---

## 1. Inventory of off-chain state

Everything below is enumerated in `STATE_INVENTORY` in the tool, which is the
single source of truth. Anything the pipeline writes that is not in this table
is state we would silently lose, so `backup.py inventory` reports unplanned
files for review before they are added.

| Component | Path | Why it matters | RPO |
|---|---|---|---|
| Source configuration | `sources.json` | Which sources are registered, their adapters and endpoints. Without it we cannot rebuild the pipeline even though the contract still lists the sources. | 1 h |
| Submission cache | `submissions.jsonl` | Every price we sent, per source and ledger. Needed to reconcile, to answer "did we submit this?", and to avoid duplicate submissions after a restart. | 1 h |
| Asset index | `index/assets.json` | Asset → decimals and display metadata used by the exporter and the SEP-40 route. Wrong decimals here are a pricing bug, not a display bug. | 4 h |
| Replay nonces | `nonces.json` | Replay-protection nonces for signed submissions. Losing these reopens a replay window; restoring stale ones rejects legitimate submissions. | 1 h |
| Metrics snapshot | `metrics.snap` | Counters that keep rates continuous across a restart, so `rate()` does not read a false zero and page the on-call. | 24 h |

RPO is the recovery *point* objective: the age of the newest state we promise to
be able to restore. RTO (how long a restore may take) is discussed in §6.

Explicitly **not** in scope: Stellar network data availability (out of scope for
this issue) and the on-chain contract state, which the network already
guarantees.

```bash
python -m services.backup.backup inventory --state-dir /var/lib/oracle/state
```

## 2. Automated backups

```bash
ORACLE_BACKUP_PASSPHRASE=… python -m services.backup.backup create \
  --state-dir /var/lib/oracle/state \
  --backup-dir /var/backups/oracle \
  --retention-days 30
```

Each run produces:

* `<backup-id>.tar.gz` — a single archive of the whole inventory. One archive,
  not a directory of files, so a restore cannot half-succeed by finding one
  component missing.
* `<backup-id>.manifest.json` — every entry with its SHA-256, size and RPO
  class, plus the archive hash.

**A backup that has not been verified is not a backup.**

```bash
python -m services.backup.backup verify /var/backups/oracle/<id>.manifest.json
```

Verification re-hashes every entry and rejects an archive containing a file the
manifest does not list, so silent corruption and unnoticed tampering both fail
loudly.

### Schedule and retention

| Policy | Value | Rationale |
|---|---|---|
| Frequency | Hourly | Matches the tightest RPO (1 h) for the cache and the nonces. |
| Retention | 30 days | Covers a month of undetected corruption, including the slow kind. |
| Keep minimum | 3 newest | A misconfigured retention must never leave zero restore points. |
| Restore-point cadence | Daily for 30 days, hourly for 7 | Hourly for the last week (incident forensics), daily for the month (disaster recovery). |

`prune` enforces retention and always preserves the newest `keep_minimum`
backups, so "we lost the state" can never become "we also lost every way back".

### Encryption and access control

Backups contain source configuration and adapter credentials metadata, so an
unencrypted backup is an incident, not a shortcut.

* Archives are encrypted with **AES-256-CBC, PBKDF2, 200 000 iterations** via
  `openssl enc`. The passphrase comes from the environment
  (`ORACLE_BACKUP_PASSPHRASE`) or a KMS-held secret, never from a file in the
  repository or the backup directory.
* The passphrase is **not** stored with the backups. A restore that needs it
  while the KMS is unavailable is a restore that does not happen, which is why
  access to the KMS is part of the on-call roster rather than one person's
  laptop.
* Backup storage is write-only for the pipeline host: it can create a backup
  but cannot read or delete an existing one. Deletion is a separate,
  audited role.
* `verify` and `restore` fail with an explicit decryption error on a wrong
  passphrase. They never fall back to reading the archive unencrypted.

The test `test_backup_is_encrypted_at_rest` asserts that submission content is
not present in the plaintext of a backup archive, so this cannot silently
regress.

## 3. Point-in-time restore

Restoring is a *decision*, not a reflex, because a restore that overwrites a
healthy running pipeline is itself an outage. The tool is built so that the
dangerous path is the one you have to ask for explicitly.

```bash
# 1. Verify first. Never restore an unverified backup.
python -m services.backup.backup verify <id>.manifest.json

# 2. Restore into a staging directory (the default; leaves the live state alone).
python -m services.backup.backup restore <id>.manifest.json --target /var/lib/oracle/restore-test

# 3. Reconcile the restored state against on-chain truth (§4).

# 4. Only then, with the pipeline stopped, swap it in.
systemctl stop oracle-pipeline
python -m services.backup.backup restore <id>.manifest.json --target /var/lib/oracle/state --in-place
systemctl start oracle-pipeline
```

Rules the tool enforces:

* **The target must be empty** unless `--in-place` is passed. A restore into a
  populated directory without the flag is refused.
* **Verification runs first.** If verification fails, nothing is written at all
  — there is no partial restore to clean up. The test
  `test_restore_does_not_run_when_verification_fails` asserts the target
  directory does not even exist afterwards.
* **A restore never touches the contract.** No on-chain call is made by the
  restore path. If reconciliation shows the chain and the restored state
  disagree, that is resolved in §4, not by restoring harder.
* **Post-restore inventory check.** The restored tree must satisfy
  `check_inventory`; a restore missing a required component is reported rather
  than started.

## 4. Reconciliation against on-chain truth

A restored state can be internally consistent and still be *stale* or *wrong*.
Reconciliation is what distinguishes "restored" from "restored correctly", and
on-chain truth always wins.

```bash
python -m services.backup.backup reconcile \
  --submissions /var/lib/oracle/restore-test/submissions.jsonl \
  --on-chain /tmp/onchain-submissions.json \
  --backup-ledger <ledger-of-the-backup>
```

| Outcome | Meaning | Action |
|---|---|---|
| `accepted` | The chain confirms this submission at this ledger and price. | Keep. |
| `replay_residue` (dropped) | The record post-dates the backup point, so it could not have been in the backup. It is residue from the failed window. | Dropped automatically. Replaying it risks a duplicate or an out-of-order submission. |
| `missing_on_chain` | At or before the backup point, but the chain has no record of it. | **Stop.** Either the restore is stale or the submission never landed. Do not resume the pipeline. |
| `price_mismatch` | The chain confirms the submission at a *different* price. | **Stop.** This is a data-integrity finding, not bookkeeping. |

Only `missing_on_chain` and `price_mismatch` make a restore unclean; the command
exits non-zero on those. Dropped post-backup records are expected in every
point-in-time restore and are reported as `replay_residue` instead, so a large
number stays visible without making every restore look broken.

Reconciliation exports counters for alerting:

```
oracle_restore_reconcile_mismatch_total{kind="dropped_future"}
oracle_restore_reconcile_mismatch_total{kind="missing_on_chain"}
oracle_restore_reconcile_mismatch_total{kind="price_mismatch"}
```

A non-zero `missing_on_chain` or `price_mismatch` is a paging event mapped to
the runbook entry for restore failure; the wiring is tracked as an action item
of the restore drill in
[`docs/incident-management/2026-08-12-restore-drill.md`](incident-management/2026-08-12-restore-drill.md).

## 5. Restore drill

The drill is run quarterly, and the automated form of it lives in the test
suite: `test_restore_drill_reproduces_known_good_state` performs
backup → destroy → restore → reconcile, asserts every byte of known-good state
returns, and reconciles the result against on-chain truth.
`test_restore_drill_rejects_replay_residue_from_the_failed_window` adds records
from the failed window and asserts they are dropped rather than replayed.

A full drill additionally runs against a live testnet deployment:

1. Announce the window; confirm the KMS-held passphrase is reachable by the
   on-call on duty, not only by the person running the drill.
2. Create a backup, verify it, and record the backup id and manifest hash.
3. Destroy the off-chain state directory on the pipeline host.
4. Confirm the expected alerts fire (the pipeline cannot submit) and that the
   runbook entry used matches the actual failure mode.
5. Restore to staging, reconcile, and attach the report to the postmortem.
6. Swap in with the pipeline stopped; confirm aggregates resume.
7. Write the postmortem using
   [`docs/incident-management/TEMPLATE.md`](incident-management/TEMPLATE.md).

The drill performed on 2026-08-12 is recorded in
[`docs/incident-management/2026-08-12-restore-drill.md`](incident-management/2026-08-12-restore-drill.md).
It found that source configuration was outside the automated backup set, which
is now fixed and asserted by `test_inventory_covers_every_planned_component`.

## 6. Recovery objectives

| Objective | Target | Source |
|---|---|---|
| RPO — how much state may be lost | 1 h (submission cache, nonces) | §1 inventory |
| RTO — how long a restore may take | 4 h P1 target (SLA §5); 48 min measured in the 2026-08-12 drill | SLA §5, drill record |
| Backup window | 1 h | §2 schedule |

The measured 48-minute restore has roughly 4× margin against the P1 target.
That margin is re-measured each drill, because restore time grows with the
submission cache and a target that is never re-measured is a wish.

## 7. Related

* [`docs/disaster-recovery.md`](disaster-recovery.md) — contract-level scenarios
* [`docs/runbook.md`](runbook.md) — what to do when a restore-related alert pages
* [`docs/incident-management/`](incident-management/README.md) — the postmortem process
