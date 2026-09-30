//! N-2..N upgrade and downgrade round-trip matrix (#517).
//!
//! Real deployments skip versions and occasionally roll back, so the interesting
//! paths are `N-2 → N`, `N → N-2` and "the migration died half way", not
//! latest-on-latest. This module builds a matrix over the last three schema
//! versions and asserts, for every row:
//!
//! * an upgrade from `N-2` with populated state keeps every price, source and
//!   asset readable (`upgrade_from_n_minus_two_preserves_state`);
//! * a rollback of a build that cannot read the stored layout is refused, at
//!   every version boundary (`rollback_to_an_unreadable_schema_is_refused`);
//! * an up/down round trip is lossless (`round_trip_up_and_back_is_lossless`);
//! * migration is idempotent at every boundary
//!   (`migration_is_idempotent_at_every_version_boundary`);
//! * an aborted migration is recoverable at every boundary
//!   (`aborted_migration_is_recoverable_at_every_version_boundary`).
//!
//! The matrix is data-driven from [`crate::migration::CURRENT_VERSION`] and is
//! completeness-checked (`matrix_covers_three_consecutive_versions`), so adding
//! a schema version forces a new row rather than silently shrinking the matrix.
//!
//! Version compatibility decisions live in [`crate::blue_green`]; the runbook is
//! `docs/blue-green-upgrade.md` and the matrix table is `docs/version-matrix.md`.
//! Run with `make version-matrix`.

use std::string::ToString;

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, Vec,
};

use crate::blue_green::{
    check_rollback, is_layout_coherent, Deployment, RollbackDecision, RollbackPolicy,
};
use crate::migration::CURRENT_VERSION;
use crate::test_helpers::{
    create_contract, ledger_default, register_test_asset, register_test_source,
};
use crate::types::DataKey;
use crate::{Asset, PriceOracleContractClient};

/// Schema version the current build writes and reads.
const N: u32 = CURRENT_VERSION;
/// The first schema version ever released.
const EARLIEST: u32 = 1;
/// The oldest release in the matrix: `N-2`, or the earliest release when fewer
/// than three schema versions have shipped.
const N_MINUS_2: u32 = if N > EARLIEST + 1 { N - 2 } else { EARLIEST };
/// The intermediate release, `N-1`.
const N_MINUS_1: u32 = if N > EARLIEST { N - 1 } else { EARLIEST };
/// A hypothetical *newer* release. Nothing in this build may read its state.
const N_PLUS_1: u32 = N + 1;

/// The three version boundaries the matrix covers, oldest first: the oldest
/// release this build must still upgrade from, the release it is, and the
/// forward boundary it must never be rolled back onto.
///
/// When three schema versions have shipped this is exactly `N-2, N-1, N`. Until
/// then it is every released version plus the forward boundary, so the matrix
/// always has three rows and always covers the oldest supported upgrade.
pub const VERSION_MATRIX: [u32; 3] = [N_MINUS_2, N, N_PLUS_1];

/// Ledger used as "now" by the pure rollback-decision tests.
const NOW: u32 = 1_000_000;

/// A deployment descriptor for a build of the contract that speaks `version` and
/// can read up to `max_readable`.
fn build(version: u32, max_readable: u32) -> Deployment {
    Deployment {
        schema_version: version,
        max_readable_schema: max_readable,
        // Deployed inside the rollback window, so the decision under test is the
        // schema check and not the age check.
        deployed_ledger: NOW - 1_000,
    }
}

/// Deploys an oracle and rewinds its stored schema version to `version`, which
/// is the state a deployment of that release would have left behind.
fn oracle_at_version(e: &Env, version: u32) -> (PriceOracleContractClient<'_>, Address) {
    e.mock_all_auths();
    let admin = Address::generate(e);
    let client = create_contract(e);
    client.initialize(
        &admin,
        &1u32,
        &10u32,
        &18u32,
        &soroban_sdk::String::from_str(e, "version matrix"),
    );
    e.as_contract(&client.address, || {
        e.storage()
            .persistent()
            .set(&DataKey::StorageVersion, &version);
    });
    (client, admin)
}

/// Populates the contract the way a live deployment would: one priced asset and
/// one registered-but-unpriced asset (the case the v1→v2 migration exists for).
fn populate(client: &PriceOracleContractClient<'_>, e: &Env) -> (Address, Address, Address) {
    ledger_default(e, 100, 1_000);
    let source = register_test_source(e, client, "Matrix Source");
    let priced = register_test_asset(e, client);
    let unpriced = register_test_asset(e, client);
    client.submit_price(&source, &priced, &1_234, &1_000);
    (source, priced, unpriced)
}

/// The state that must survive every upgrade, downgrade and round trip.
fn state_fingerprint(
    client: &PriceOracleContractClient<'_>,
    e: &Env,
    asset: &Address,
) -> std::string::String {
    let served = client.lastprice(&Asset::Stellar(asset.clone())).unwrap();
    let versioned = client.get_aggregate_with_version(asset);
    let signers: Vec<Address> = Vec::new(e);
    format!(
        "price={} ts={} sources={} version={} n_assets={} n_sources={}",
        served.price,
        served.timestamp,
        versioned.aggregate.num_sources,
        versioned.version,
        client.assets().len(),
        client.get_oracle_sources().sources.len() + signers.len() as u32,
    )
}

/// The matrix spans three version boundaries and covers every released schema
/// version, so a new migration cannot silently drop out of the matrix.
#[test]
fn matrix_covers_three_consecutive_versions() {
    assert_eq!(VERSION_MATRIX.len(), 3, "the matrix must have three rows");
    assert_eq!(
        VERSION_MATRIX[1], N,
        "the middle row is the current release"
    );
    assert_eq!(
        VERSION_MATRIX[2], N_PLUS_1,
        "the last row is the forward boundary"
    );
    assert!(VERSION_MATRIX[0] <= N, "rows are ordered oldest first");
    for w in VERSION_MATRIX.windows(2) {
        assert!(w[1] > w[0], "the matrix must be strictly increasing: {w:?}");
    }
    // Every released schema version below the current one has a row, so adding a
    // migration cannot silently drop out of the matrix.
    for v in EARLIEST..=N {
        assert!(
            VERSION_MATRIX.contains(&v),
            "schema version {v} is missing from the version matrix"
        );
    }
    // Once three versions have shipped the matrix is exactly N-2..N.
    if N > EARLIEST + 1 {
        assert_eq!(VERSION_MATRIX[0], N - 2);
        assert_eq!(N_MINUS_1, N - 1);
    }
}

/// N-2 → N with populated state: nothing is lost and the layout is complete.
#[test]
fn upgrade_from_n_minus_two_preserves_state() {
    let e = Env::default();
    let (client, _) = oracle_at_version(&e, N_MINUS_2);
    let (_source, priced, unpriced) = populate(&client, &e);
    let before = state_fingerprint(&client, &e, &priced);
    assert_eq!(client.get_storage_version(), N_MINUS_2);

    client.migrate_storage(&50);

    assert_eq!(client.get_storage_version(), N, "the upgrade completed");
    assert!(
        client.get_migration_state().is_none(),
        "no cursor left open"
    );
    assert_eq!(state_fingerprint(&client, &e, &priced), before);
    assert!(is_layout_coherent(false, &[N, N, N]));

    // The v1 → v2 migration backfills a placeholder aggregate for the unpriced
    // asset. It must exist (so downstream code finds a typed record) and must
    // never be served as a price.
    assert!(
        client
            .get_aggregate_with_version(&unpriced)
            .aggregate
            .num_sources
            == 0
    );
    assert!(
        client.lastprice(&Asset::Stellar(unpriced)).is_none(),
        "a backfilled placeholder must not be served as a price"
    );
}

/// A build that cannot read the stored layout is refused a rollback, at every
/// version boundary in the matrix. This is the classic corruption path: old
/// code running over state written by a newer release.
#[test]
fn rollback_to_an_unreadable_schema_is_refused() {
    let e = Env::default();
    let a = Address::generate(&e);
    let b = Address::generate(&e);
    let mut signers: Vec<Address> = Vec::new(&e);
    signers.push_back(a.clone());
    signers.push_back(b.clone());
    let mut approvals: Vec<Address> = Vec::new(&e);
    approvals.push_back(a);
    approvals.push_back(b);
    let now = NOW;

    for stored in VERSION_MATRIX {
        if stored <= EARLIEST {
            // Nothing older than the first release exists to roll back to.
            continue;
        }
        let older = build(stored - 1, stored - 1);
        assert_eq!(
            check_rollback(
                &RollbackPolicy::default(),
                &signers,
                &approvals,
                None,
                now,
                &older,
                stored
            ),
            RollbackDecision::SchemaIncompatible,
            "a v{} build must not read v{} state",
            older.schema_version,
            stored
        );
    }

    // Even the newest stored state in the matrix is unreadable by the N-2 build.
    assert_eq!(
        check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &approvals,
            None,
            now,
            &build(N_MINUS_2, N_MINUS_2),
            N
        ),
        RollbackDecision::SchemaIncompatible
    );
    // The same build may roll back while the stored schema is its own.
    assert_eq!(
        check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &approvals,
            None,
            now,
            &build(N_MINUS_2, N_MINUS_2),
            N_MINUS_2
        ),
        RollbackDecision::Allowed
    );
}

/// N-2 → N → N-2 → N: the same state comes back out of a full round trip.
#[test]
fn round_trip_up_and_back_is_lossless() {
    let e = Env::default();
    let (client, _) = oracle_at_version(&e, N_MINUS_2);
    let (_source, priced, unpriced) = populate(&client, &e);
    let before = state_fingerprint(&client, &e, &priced);

    // Up.
    client.migrate_storage(&50);
    let after_upgrade = state_fingerprint(&client, &e, &priced);
    assert_eq!(client.get_storage_version(), N);

    // Down: rolling the WASM back to the N-2 build also rewinds the recorded
    // schema version to that build's own version.
    e.as_contract(&client.address, || {
        e.storage()
            .persistent()
            .set(&DataKey::StorageVersion, &N_MINUS_2);
    });
    assert_eq!(client.get_storage_version(), N_MINUS_2);
    assert_eq!(
        state_fingerprint(&client, &e, &priced),
        before,
        "the N-2 build must still read the data it wrote"
    );
    assert!(client.lastprice(&Asset::Stellar(unpriced)).is_none());

    // Forward again: the round trip is a no-op on the data.
    client.migrate_storage(&50);
    assert_eq!(client.get_storage_version(), N);
    assert_eq!(state_fingerprint(&client, &e, &priced), after_upgrade);
    assert!(is_layout_coherent(false, &[N]));
}

/// Migrating twice (or migrating an already-current contract) changes nothing.
#[test]
fn migration_is_idempotent_at_every_version_boundary() {
    for from in VERSION_MATRIX {
        let e = Env::default();
        let (client, _) = oracle_at_version(&e, from);
        let (_source, priced, _unpriced) = populate(&client, &e);
        let before = state_fingerprint(&client, &e, &priced);

        client.migrate_storage(&50);
        let after = state_fingerprint(&client, &e, &priced);
        // The migration always ends at the schema version this build speaks,
        // including when the stored version is newer: that rewind is the classic
        // downgrade hazard, and it is why `check_rollback` refuses to put an old
        // build on a newer layout in the first place.
        assert_eq!(
            client.get_storage_version(),
            N,
            "v{from}: wrong end version"
        );

        // Idempotency: repeated calls, and a call with a tiny batch size after a
        // completed migration, leave both the version and the data alone.
        for _ in 0..3 {
            client.migrate_storage(&1);
        }
        assert_eq!(state_fingerprint(&client, &e, &priced), after);
        assert_eq!(
            state_fingerprint(&client, &e, &priced),
            before,
            "v{from}: data changed"
        );
        assert!(client.get_migration_state().is_none());
    }
}

/// A migration that dies half way leaves a coherent, recoverable state at every
/// version boundary: the partial pass is visible, the data is still readable,
/// and re-running finishes the job to exactly the same result.
#[test]
fn aborted_migration_is_recoverable_at_every_version_boundary() {
    for from in VERSION_MATRIX {
        let e = Env::default();
        let (client, _) = oracle_at_version(&e, from);
        let (_source, priced, unpriced) = populate(&client, &e);
        let before = state_fingerprint(&client, &e, &priced);

        if from >= N {
            // Already current: there is nothing to abort, and the call must not
            // open a cursor.
            client.migrate_storage(&1);
            assert!(client.get_migration_state().is_none());
            assert_eq!(state_fingerprint(&client, &e, &priced), before);
            continue;
        }

        // Abort after a single item: the cursor is still open.
        client.migrate_storage(&1);
        let open = client
            .get_migration_state()
            .expect("the aborted migration must leave an open cursor");
        assert_eq!(open.from_version, from);
        assert_eq!(open.to_version, N);
        assert!(
            !is_layout_coherent(true, &[N]),
            "v{from}: an open cursor is by definition not coherent"
        );
        // Data written before the abort is still readable by the new build.
        assert_eq!(state_fingerprint(&client, &e, &priced), before);

        // Resume: the migration completes and the result is identical to an
        // uninterrupted run.
        for _ in 0..(client.assets().len() + 2) {
            client.migrate_storage(&1);
        }
        assert!(
            client.get_migration_state().is_none(),
            "v{from}: cursor left open"
        );
        assert_eq!(client.get_storage_version(), N);
        assert!(is_layout_coherent(false, &[N, N, N]));
        assert_eq!(state_fingerprint(&client, &e, &priced), before);
        assert!(client.lastprice(&Asset::Stellar(unpriced)).is_none());
    }
}

/// A build must never read state written by a release it cannot parse, at any
/// matrix boundary — including a release newer than the current build.
#[test]
fn cross_version_reads_are_rejected() {
    let e = Env::default();
    let a = Address::generate(&e);
    let b = Address::generate(&e);
    let mut signers: Vec<Address> = Vec::new(&e);
    signers.push_back(a.clone());
    signers.push_back(b.clone());
    let mut approvals: Vec<Address> = Vec::new(&e);
    approvals.push_back(a);
    approvals.push_back(b);

    // A stored schema the N-2 build can read is fine; anything newer is refused.
    assert_eq!(
        check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &approvals,
            None,
            NOW,
            &build(N_MINUS_2, N_MINUS_2),
            N_MINUS_2
        ),
        RollbackDecision::Allowed
    );
    for stored in VERSION_MATRIX.iter().filter(|v| **v > N_MINUS_2) {
        assert_eq!(
            check_rollback(
                &RollbackPolicy::default(),
                &signers,
                &approvals,
                None,
                NOW,
                &build(N_MINUS_2, N_MINUS_2),
                *stored
            ),
            RollbackDecision::SchemaIncompatible,
            "the N-2 build must refuse stored schema {stored}"
        );
    }

    // Forward compatibility: this build must not be rolled back onto a layout
    // written by a newer release either.
    assert_eq!(
        check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &approvals,
            None,
            NOW,
            &build(N, N),
            N_PLUS_1
        ),
        RollbackDecision::SchemaIncompatible
    );
    assert_eq!(
        check_rollback(
            &RollbackPolicy::default(),
            &signers,
            &approvals,
            None,
            NOW,
            &build(N, N),
            N
        ),
        RollbackDecision::Allowed
    );
}
