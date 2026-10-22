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

## Upgrade authorization is bound to the artifact (#689)

The admin approves an upgrade by authorizing `upgrade(new_wasm_hash)` for the
hash of the approved release artifact (the reproducible build of the release
tag, see [`reproducible-builds.md`](reproducible-builds.md)). Soroban binds an
authorization to the exact call arguments, so that approval cannot install any
other WASM, even a valid one that has already been uploaded.

Before signing, compare the hash in the transaction with the hash of the
approved artifact. Sign only when they match.

| Case | Result |
|---|---|
| Approval for the release hash, upgrade to the release hash | Applied. Admin and stored prices are unchanged. |
| Approval for the release hash, upgrade to a substituted artifact | Refused at `require_auth`, before any event or WASM swap. The prior implementation keeps serving reads and accepting submissions. |
| Refused substitution, then the approved upgrade | Applied. A refused attempt does not block the release. |
| Hash one byte away from the approved one | Refused. |
| Approval from an address other than the admin | Refused. |

The substituted artifact in the tests is the release WASM with an extra custom
section, so it is a valid contract with a different hash. A separate test shows
it upgrades cleanly when it is the approved hash, so the refusals come from the
authorization binding and not from a broken artifact.

```bash
make build   # the tests load target/wasm32v1-none/release/price_oracle.wasm
cargo test -p price-oracle --lib upgrade_artifact_binding_tests
```

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
