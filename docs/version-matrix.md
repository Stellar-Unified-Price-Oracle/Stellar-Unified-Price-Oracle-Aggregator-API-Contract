# N-2..N Upgrade and Downgrade Matrix (#517)

Suite: `contracts/price-oracle/src/version_matrix_tests.rs`.
Run with `make version-matrix` (`cargo test -p price-oracle --lib version_matrix`).
CI runs it on every pull request (`.github/workflows/ci.yml`, job `conformance`).
Runbook: [`docs/blue-green-upgrade.md`](blue-green-upgrade.md). Decision logic:
`contracts/price-oracle/src/blue_green.rs`. Migration driver:
`contracts/price-oracle/src/migration.rs`.

Real deployments skip versions and occasionally roll back. The interesting paths
are `N-2 → N`, `N → N-2` and "the migration died half way" — not
latest-on-latest — so the suite is a matrix over version boundaries rather than a
single happy path.

## The matrix

`VERSION_MATRIX` has three rows: the oldest release this build must still
upgrade from, the release this build is, and the forward boundary.

| Row | Meaning | `CURRENT_VERSION = 2` (today) | `CURRENT_VERSION = 5` |
|---|---|---|---|
| 0 | `N-2`, or the earliest release when fewer than three have shipped | 1 | 3 |
| 1 | `N`, the release this build speaks | 2 | 5 |
| 2 | `N+1`, the forward boundary: a release whose state this build must never be rolled back onto | 3 | 6 |

`matrix_covers_three_consecutive_versions` is the completeness gate: it fails if
the matrix stops covering a released schema version (so adding a migration in
`migration.rs` cannot silently drop out of the matrix), if the rows stop being
strictly increasing, or if the middle row stops being the current release. Once
three schema versions have shipped, row 0 is exactly `N-2`.

## What each row is exercised with

State is populated the way a live deployment is: one priced asset, one registered
but unpriced asset, one source, an admin, and a storage version rewound to the
row's release.

| Test | Property |
|---|---|
| `upgrade_from_n_minus_two_preserves_state` | `N-2 → N` keeps every price, source, asset and version; the layout ends coherent; the backfilled placeholder aggregate for the unpriced asset is **not** served as a price (#453) |
| `round_trip_up_and_back_is_lossless` | `N-2 → N → N-2 → N` returns byte-identical state fingerprints; the `N-2` build still reads what it wrote |
| `rollback_to_an_unreadable_schema_is_refused` | a build whose `max_readable_schema` is below the stored schema is refused at **every** row of the matrix, with quorum and a fresh deployment |
| `cross_version_reads_are_rejected` | the `N-2` build refuses every newer stored schema, and this build is refused as a rollback target for `N+1` state |
| `migration_is_idempotent_at_every_version_boundary` | migrating repeatedly, and migrating an already-current contract, changes neither the data nor the recorded version |
| `aborted_migration_is_recoverable_at_every_version_boundary` | a migration stopped after one item leaves an open cursor and incoherent layout flag, the data stays readable, and re-running finishes to the same result as an uninterrupted run |

## Documented incompatibilities and hazards

| Finding | Affected versions | Behaviour |
|---|---|---|
| `migrate_storage` always ends at `CURRENT_VERSION` | any stored version `> N` | Calling the migration from a build that is *older* than the stored layout rewinds the recorded version instead of refusing. The data is untouched, but the version key is now wrong for the newer build. This is the classic downgrade corruption path, and it is why `check_rollback` must refuse the downgrade **before** the old code runs (`rollback_to_an_unreadable_schema_is_refused`, `cross_version_reads_are_rejected`). |
| The v1 → v2 migration backfills a zero aggregate for unpriced assets | `1 → 2` | Harmless only because the read path refuses to serve a zero aggregate (#453). A consumer reading storage directly must apply the same rule. |
| A migration that dies mid-flight leaves `MigrationState` open | all | Reads still return pre-migration data, and `is_layout_coherent` reports the layout as not coherent until the migration resumes. Operators must re-run `migrate_storage`; the sweep in `scripts/verify-deployment.sh` fails a deployment that leaves a cursor open. |

There is no version pair in the matrix today for which state is silently
misread: every incompatible pair is rejected by the rollback guard, and every
compatible pair round-trips losslessly.

## Build artifacts for the last N versions

The matrix runs against the storage versions of the last N releases, not
against N WASM blobs, because the storage schema is what a rollback actually
risks: the WASM of a previous release is the same contract with an older
`CURRENT_VERSION` and an older `run_migration_step`. Each row reproduces the
on-chain state that release would have written by setting `DataKey::StorageVersion`
before migrating, which is the same lever the deployment tooling uses. When a new
schema version is added, add its row to `VERSION_MATRIX` and, if its migration is
not a no-op, a `(from, to)` branch in `run_migration_step` plus a round-trip case
here.
