# Release compatibility and rollback notes

Operator-facing notes for #721. They answer two questions for any release cut from
this repository: **what does it stay compatible with**, and **when and how can it be
rolled back**. Every number below is read from the code or the existing runbooks;
the source of each is named so the notes can be re-checked rather than trusted.

Related runbooks: [`blue-green-upgrade.md`](../blue-green-upgrade.md),
[`version-matrix.md`](../version-matrix.md),
[`interface-versioning.md`](../interface-versioning.md),
[`migrations/README.md`](../migrations/README.md),
[`release-provenance.md`](../release-provenance.md).

## 1. Compatibility surface of this release line

| Surface | Current value | Source | Compatible with |
|---|---|---|---|
| Published interface package | `@stellar-unified-price-oracle/interface` **0.2.0** | `interface/interface.json` | consumers built against 0.2.0; 0.1.0 consumers need [`migrations/v0.2.0.md`](../migrations/v0.2.0.md) |
| Public endpoints in the interface | 640 | `interface/interface.json` | additive within a minor line |
| Contract crate version | `0.1.0` | `contracts/price-oracle/Cargo.toml` | independent of the interface version; the interface-to-WASM mapping is recorded in `interface/versions.json` |
| Storage schema written by this build | `CURRENT_VERSION = 2` | `contracts/price-oracle/src/migration.rs` | upgrades from schema 1 by `migrate_storage` |
| Highest schema this build can read back | `max_readable_schema = 2` | `contracts/price-oracle/src/blue_green.rs` | a rollback target must have `max_readable_schema` ≥ the stored schema |
| Upgrade matrix | `N-2 → N`, `N → N-2`, and aborted migrations | [`version-matrix.md`](../version-matrix.md) | every incompatible pair is refused by the rollback guard |

**Semver rules for the interface** (enforced in CI by `scripts/devx/gen_interface.py --check`):

| Change | Required bump on 0.x |
|---|---|
| Function removed, signature changed, or error code removed/renumbered | **minor** |
| Function added | patch |
| Docs only | none |

A breaking change also requires a migration guide registered in
[`migrations/index.json`](../migrations/index.json), and a function must be
deprecated for at least 90 days before it is removed.

## 2. Per-release compatibility statement (fill in for each tag)

Copy this block into the release notes for every `vX.Y.Z` / `interface-vX.Y.Z` tag.

```text
Release:                    <tag>
Interface version:          <x.y.z>   (previous: <x.y.z>)
Breaking interface change:  yes | no   -> migration guide: docs/migrations/<vX.Y.Z>.md
Contract WASM hash:         <sha256>   (reproducible build of the tag, see release-provenance.md)
Storage schema written:     <CURRENT_VERSION>   (migration required from: <versions>)
Rollback target:            <previous WASM hash>   max_readable_schema: <n>
Rollback allowed:           yes | no   reason: <schema-changing migration | none>
Rollback window ends:       ledger <deployed_ledger + 120,960>
Deprecations introduced:    <functions, removal date>
Error codes added/removed:  <list> (see docs/errors/REGISTRY.md)
Consumer action required:   <none | steps>
```

## 3. Can this release be rolled back?

Decision, in order. Stop at the first row that applies.

| # | Condition | Decision | Why (control) |
|---|---|---|---|
| 1 | The release ran a **schema-changing migration** (`CURRENT_VERSION` increased) | **Do not roll back. Roll forward with a fix.** | Old code on a newer layout is the downgrade-corruption path; `migrate_storage` run by an older build would rewind the recorded version (documented in `version-matrix.md`). |
| 2 | Stored schema `>` the rollback target's `max_readable_schema` | **Refused** by `check_rollback` | prevents old code reading a layout it does not understand |
| 3 | A migration is in progress (`get_migration_state` is not `null`) | **Refused** by the script; finish or resume the migration first | `is_layout_coherent` requires no open cursor and a single schema across all assets |
| 4 | More than `MAX_ROLLBACK_WINDOW` = **120,960 ledgers (~7 days)** since blue was deployed | **Refused** | bounds how stale the rollback target may be |
| 5 | Less than `min_interval` = **17,280 ledgers (~24 h)** since the previous rollback | **Refused** | stops rollback being used as a weapon |
| 6 | Fewer than `quorum` = **2** distinct authorised signers approved | **Refused** | duplicate or outsider approvals are not counted |
| 7 | None of the above | **Allowed** | quorum collected off-chain, then the admin submits the rollback |

Constants are `MAX_ROLLBACK_WINDOW`, `DEFAULT_MIN_INTERVAL` and `DEFAULT_QUORUM` in
`contracts/price-oracle/src/blue_green.rs`.

## 4. When to roll back

Roll back (rows 2–7 permitting) when the **post-upgrade invariant sweep fails** or the
green build serves incoherent prices. Do not roll back for a defect that only needs a
configuration change, and do not roll back after a schema-changing migration (row 1).

## 5. Rollback procedure

1. **Freeze and confirm the trigger.** Record the failing check from
   `scripts/verify-deployment.sh` (or the observed price incoherence) in the incident.
2. **Check the decision table** in §3. If row 1 applies, stop and prepare a forward fix.
3. **Locate blue.** The blue WASM hash is in `.blue-green/<CONTRACT_ID>.blue`, written
   before every upgrade. Verify it equals the hash recorded in the release's
   compatibility statement (§2).
4. **Collect the quorum** (default 2 distinct signers) off-chain.
5. **Authorize for the exact hash.** The admin authorizes `upgrade(blue_hash)`; Soroban
   binds the authorization to the arguments, so it cannot install any other WASM.
   Compare the hash in the transaction with the approved blue hash before signing.
6. **Run the rollback:**

   ```bash
   ./scripts/blue-green-upgrade.sh rollback --contract <CONTRACT_ID> --admin <IDENTITY> \
       --blue-hash <BLUE_HASH> --network <network>
   ```

   On `mainnet` the script requires the privileged-transaction review payload and
   approval (`--payload` / `--approval`).
7. **Verify.** Re-run `scripts/verify-deployment.sh`: no migration in progress, the
   expected schema version, and the admin unchanged. A failing sweep fails the rollback.
8. **Record** the rollback ledger (it starts the `min_interval` clock), the hashes, the
   approvers, and update `interface/versions.json` in the next release.

## 6. What a consumer should expect

| Event | Consumer-visible effect |
|---|---|
| Additive release (new endpoints) | none; existing calls keep working |
| Breaking release | follow the migration guide; deprecated functions stay for ≥ 90 days |
| Upgrade with migration in flight | reads return pre-migration data and stay coherent; `is_layout_coherent` is false until the migration finishes |
| Rollback | prices and sources are preserved; only the executing WASM changes, so the callable interface reverts to that of the rollback target (functions added by the rolled-back release disappear) |
| Refused or aborted upgrade/rollback | the prior implementation keeps serving reads and accepting submissions |
