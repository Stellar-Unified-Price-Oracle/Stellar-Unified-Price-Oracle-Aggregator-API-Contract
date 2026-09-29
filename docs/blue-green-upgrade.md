# Blue-Green Upgrade & Rollback Runbook (#415)

Tooling: `scripts/blue-green-upgrade.sh`. Guards: `contracts/price-oracle/src/blue_green.rs`.
Invariant sweep: `scripts/verify-deployment.sh` (section 8).

## Flow

1. Run `verify-deployment.sh` against **blue** (current).
2. Record the blue WASM hash in `.blue-green/<CONTRACT_ID>.blue`.
3. Upload **green** and call `upgrade`.
4. Run `migrate_storage` in batches until `get_migration_state` is `null`.
   Migration is idempotent and resumable from the on-chain cursor.
5. Run the post-upgrade invariant sweep: no migration in progress, the
   expected schema version, and an unchanged admin. **A failed sweep fails
   the deployment.**

## Threats and controls

| Threat | Control |
|---|---|
| Schema desync (old code on new layout) | `check_rollback` refuses if the stored schema is newer than `max_readable_schema` of blue. The script refuses to roll back while a migration is in progress. |
| Rollback-as-weapon | Rollbacks are rate-limited: `min_interval` defaults to 17,280 ledgers (~24h). |
| Single compromised key | Quorum of distinct authorised signers (default 2-of-N). Duplicate or outsider approvals are not counted. |
| Authority confusion | The sweep asserts that the admin is unchanged across the upgrade. |
| Partial migration | `is_layout_coherent` requires no open cursor and a single schema across all assets. After an abort, re-running `upgrade` resumes the migration. |

## Forced abort drill (testnet)

```bash
ABORT_AFTER=1 ./scripts/blue-green-upgrade.sh upgrade --contract $ID --admin ops --wasm green.wasm  # exits 2
./scripts/blue-green-upgrade.sh upgrade --contract $ID --admin ops --wasm green.wasm                # resumes
```

After the abort, reads still return coherent values: data that has not been
migrated yet stays readable by the green build. The contract can still be
upgraded.

## Rollback decisions

- **Who can authorise:** the multisig signer set (`multisig.rs`), with at
  least `quorum` distinct approvals collected off-chain before the admin
  submits `rollback`.
- **When:** the green sweep fails, or green serves incoherent prices.
- **Maximum safe rollback window:** `MAX_ROLLBACK_WINDOW` = 120,960 ledgers
  (~7 days) after blue was deployed. It is also bounded by schema
  compatibility. After a schema-changing migration, roll **forward** with a
  fix instead of rolling back.
