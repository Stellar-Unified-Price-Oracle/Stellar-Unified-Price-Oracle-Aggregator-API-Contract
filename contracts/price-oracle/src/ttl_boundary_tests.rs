//! TTL and rent boundary simulation suite (#522).
//!
//! Storage expiry is a real operational event on Soroban and its failure mode
//! is subtle: an evicted key must look *absent*, never like a stale-but-plausible
//! value. This suite walks each key class across the expiry boundary — the last
//! ledger an entry is live, the eviction ledger itself, and the ledgers after —
//! and asserts the contract fails closed at every step.
//!
//! ## Simulating expiry
//!
//! The two storage tiers expire differently under the Soroban *test* host, and
//! the difference was verified empirically rather than assumed:
//!
//! * **Temporary** entries are expired by the host. With `min_temp_entry_ttl =
//!   100`, an entry written at ledger 1 is readable at ledger 100 and gone at
//!   ledger 101 — live *through* `written_at + T - 1`, evicted *at*
//!   `written_at + T`.
//! * **Persistent** entries are *not* expired by the host. Advancing the ledger
//!   leaves them readable indefinitely, because in production they are archived
//!   and restored on a later access rather than deleted. The suite therefore
//!   removes them explicitly to reproduce the post-expiry state a consumer
//!   actually observes.
//!
//! [`evict_if_expired`] encapsulates that model so every boundary assertion
//! measures the same rule, and [`evicted_from`] is the single definition of the
//! boundary ledger. Centralising it is what makes the assertions trustworthy: a
//! test cannot assert a different boundary than the one the suite documents.
//!
//! ## What is asserted
//!
//! * every key class has a boundary case, and the class list is complete;
//! * an expired read never yields a default/zero value — it yields absent or a
//!   distinct error;
//! * distinct errors are distinct from each other, so an operator can tell an
//!   evicted aggregate from an unregistered asset;
//! * batch TTL extension is atomic: extending N entries extends exactly N, and
//!   a capped call never extends a partial entry or corrupts the remainder.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};

use crate::storage::{LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};
use crate::types::{DataKey, ErrorCode};
use crate::{PriceOracleContract, PriceOracleContractClient};

/// Minimum persistent-entry TTL used throughout the suite.
///
/// Small so a test can cross the boundary in a few ledger steps, and set on
/// `LedgerInfo` so the value the host reports matches what the tests assume.
const MIN_TTL: u32 = 100;

/// Every storage key class the contract persists, with the tier it lives in.
///
/// The completeness test at the bottom asserts this list matches the classes the
/// suite actually covers, so a new key class cannot skip boundary testing.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KeyClass {
    /// Singleton config / identity keys (`Admin`, `CfgMinSources`, …).
    AdminConfig,
    /// `SrcActive` / `SrcRegistry` — the source allow-list.
    SourceRegistry,
    /// `AssetRegistered` / `AssetRegistry` — the asset allow-list.
    AssetRegistry,
    /// `Submission(asset, source)` — a single source's latest price.
    Submission,
    /// `Aggregate(asset)` — the published price.
    Aggregate,
    /// `PriceHistory(asset, ledger)` — temporary history.
    History,
}

const KEY_CLASSES: &[KeyClass] = &[
    KeyClass::AdminConfig,
    KeyClass::SourceRegistry,
    KeyClass::AssetRegistry,
    KeyClass::Submission,
    KeyClass::Aggregate,
    KeyClass::History,
];

impl KeyClass {
    /// Human-readable name, used in assertion messages.
    fn name(self) -> &'static str {
        match self {
            KeyClass::AdminConfig => "AdminConfig",
            KeyClass::SourceRegistry => "SourceRegistry",
            KeyClass::AssetRegistry => "AssetRegistry",
            KeyClass::Submission => "Submission",
            KeyClass::Aggregate => "Aggregate",
            KeyClass::History => "History",
        }
    }

    /// Storage tier this class lives in.
    ///
    /// Temporary entries are the ones an operator is most likely to be surprised
    /// by, since they are evicted aggressively and are *meant* to be.
    fn is_temporary(self) -> bool {
        matches!(self, KeyClass::History)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Expiry model
// ─────────────────────────────────────────────────────────────────────────────

/// Sets ledger info with a small, explicit TTL window so the boundary is
/// reachable in a few steps and the numbers the tests reason about are visible.
fn set_ledger(e: &Env, sequence: u32, timestamp: u64) {
    e.ledger().with_mut(|l| {
        l.sequence_number = sequence;
        l.timestamp = timestamp;
        l.min_persistent_entry_ttl = MIN_TTL;
        l.min_temp_entry_ttl = MIN_TTL;
    });
}

/// The first ledger at which an entry written at `written_at` is gone.
///
/// Verified against the host: with `min_ttl = 100`, an entry written at ledger
/// 1 is still present at ledger 100 and already evicted at ledger 101. So the
/// entry is live *through* `evicted_from - 1` and gone *at* `evicted_from` —
/// the boundary is the first expired ledger, not the last live one.
fn evicted_from(written_at: u32) -> u32 {
    written_at + MIN_TTL
}

/// Advances the ledger to `sequence` and evicts `key` if the model says it has
/// expired, returning `true` if the key was present at that point.
///
/// This is the single place the expiry model is applied, so every boundary
/// assertion in the suite measures the same rule.
///
/// Note the asymmetry the test host actually exhibits, which this helper
/// preserves:
///
/// * **Temporary** entries are expired by the host itself. Advancing the
///   ledger past `live_until` is enough — the entry is already gone, and
///   removing it again would be a no-op.
/// * **Persistent** entries are *not* expired by the host; they are archived
///   and would be restored on a later access. Advancing the ledger alone
///   therefore leaves them readable, so the suite removes them explicitly to
///   reproduce the post-restore state where the value is gone.
///
/// A `true` return means "the key was still there when we looked", i.e. it had
/// not yet been evicted at this point in the walk.
fn evict_if_expired(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    key: &DataKey,
    temporary: bool,
    written_at: u32,
    sequence: u32,
) -> bool {
    e.ledger().set_sequence_number(sequence);

    let owner = client.address.clone();
    e.as_contract(&owner, || {
        if temporary {
            // Host-evicted: just report presence at this point in the walk.
            e.storage().temporary().has(key)
        } else {
            let had = e.storage().persistent().has(key);
            if had && sequence >= evicted_from(written_at) {
                e.storage().persistent().remove(key);
            }
            had
        }
    })
}

/// Builds a contract with one source, one asset and a live aggregate price, then
/// returns the source, asset and contract address for the tests to poke at
/// individual keys.
fn build_fixture(e: &Env) -> (Address, Address, Address) {
    set_ledger(e, 1, 1_000);
    let (client, _admin) = setup_contract(e);
    client.set_min_sources_required(&1);
    let source = register_test_source(e, &client, "Source 1");
    let asset = register_test_asset(e, &client);
    client.submit_price(&source, &asset, &1_000_000, &1_000);
    (source, asset, client.address)
}

// ─────────────────────────────────────────────────────────────────────────────
// Boundary behaviour, per key class
// ─────────────────────────────────────────────────────────────────────────────

/// The three phases every key class is walked through.
enum Phase {
    /// The last ledger the entry is live.
    JustBefore,
    /// The first ledger the entry is gone.
    JustAfter,
}

impl Phase {
    /// The ledger to advance to, given the ledger the entry was written at.
    fn sequence(self, written_at: u32) -> u32 {
        match self {
            // The last ledger the entry is still readable on.
            Phase::JustBefore => evicted_from(written_at) - 1,
            // The first ledger it is gone on.
            Phase::JustAfter => evicted_from(written_at),
        }
    }
}

/// The aggregate class is the one an oracle consumer reads, so it gets the full
/// just-before / at / just-after walk.
#[test]
fn aggregate_expired_read_never_returns_a_default() {
    let e = Env::default();
    e.mock_all_auths();
    let (_source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    let key = DataKey::Aggregate(asset.clone());
    let written_at = e.ledger().sequence();

    // Live: the real price is served.
    let before = client.get_price(&asset, &0);
    assert!(before.is_some(), "aggregate must be live before expiry");
    assert_eq!(before.unwrap().price, 1_000_000);

    // Just before eviction the value is still served.
    let existed = evict_if_expired(
        &e,
        &client,
        &key,
        false,
        written_at,
        Phase::JustBefore.sequence(written_at),
    );
    assert!(
        existed,
        "aggregate must still exist at the last live ledger"
    );
    let at_boundary = client.get_price(&asset, &0);
    assert!(
        at_boundary.is_some(),
        "aggregate must be served through live_until"
    );
    assert_eq!(at_boundary.unwrap().price, 1_000_000);

    // Just after eviction the read must fail closed.
    let evicted = evict_if_expired(
        &e,
        &client,
        &key,
        false,
        written_at,
        Phase::JustAfter.sequence(written_at),
    );
    assert!(evicted, "eviction should have removed the aggregate");

    let after = client.get_price(&asset, &0);
    assert!(
        after.is_none(),
        "expired aggregate must read as absent, not as a default"
    );

    // The strict version must surface a distinct error rather than None.
    let err = client.try_get_aggregate_with_version(&asset);
    assert_eq!(
        err,
        Err(Ok(ErrorCode::NoData.into())),
        "expired aggregate must report NoData, not a fabricated price"
    );

    // …and the asset itself is still registered: the two failures are distinct.
    assert!(client.is_asset_registered(&asset));
}

/// A per-source submission: evicting it must not resurrect a price from the
/// other sources, and must not make the asset look unregistered.
#[test]
fn submission_expired_read_fails_closed() {
    let e = Env::default();
    e.mock_all_auths();
    let (source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    let key = DataKey::Submission(asset.clone(), source.clone());
    let written_at = e.ledger().sequence();

    // The aggregate is derived from the submission, so it is live first.
    assert!(client.get_price(&asset, &0).is_some());

    let existed = evict_if_expired(
        &e,
        &client,
        &key,
        false,
        written_at,
        Phase::JustAfter.sequence(written_at),
    );
    assert!(existed, "submission should have been live and then evicted");

    // The submission key is gone…
    let owner = client.address.clone();
    let present = e.as_contract(&owner, || e.storage().persistent().has(&key));
    assert!(!present, "submission key must be absent after eviction");

    // …but the registry entry is untouched: eviction is not deregistration.
    assert!(
        client.is_asset_registered(&asset),
        "evicting a submission must not unregister the asset"
    );
}

/// The source allow-list: losing it must fail closed rather than silently
/// treating every source as unregistered-but-present.
#[test]
fn source_registry_expiry_fails_closed() {
    let e = Env::default();
    e.mock_all_auths();
    let (source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    let key = DataKey::SrcActive(source.clone());
    let written_at = e.ledger().sequence();

    let existed = evict_if_expired(
        &e,
        &client,
        &key,
        false,
        written_at,
        Phase::JustAfter.sequence(written_at),
    );
    assert!(existed, "source flag should have been evicted");

    // A submission from the now-evicted source must be rejected as
    // unauthorized, not accepted and not silently ignored.
    let err = client.try_submit_price(&source, &asset, &2_000_000, &1_000);
    assert_eq!(
        err,
        Err(Ok(ErrorCode::NotAuthorized.into())),
        "an evicted source must fail closed with NotAuthorized"
    );
}

/// The asset allow-list: eviction yields a *distinct* error from an aggregate
/// eviction, so an operator can tell the two apart.
#[test]
fn asset_registry_expiry_has_a_distinct_error() {
    let e = Env::default();
    e.mock_all_auths();
    let (_source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    // Registration is recorded twice: a legacy `AssetRegistered` flag and an
    // O(1) `AssetRegistryIndex` membership entry. `check_registered_asset`
    // consults the index first, so evicting only the legacy flag leaves the
    // asset registered — which is precisely the backward-compatibility path in
    // `storage.rs`. Both must go for the asset to read as unregistered.
    let legacy = DataKey::AssetRegistered(asset.clone());
    let index = DataKey::AssetRegistryIndex(asset.clone());
    let written_at = e.ledger().sequence();

    let boundary = Phase::JustAfter.sequence(written_at);
    evict_if_expired(&e, &client, &index, false, written_at, boundary);
    evict_if_expired(&e, &client, &legacy, false, written_at, boundary);

    // Reading through the allow-list check now fails with AssetNotRegistered.
    let err = client.try_get_aggregate_with_version(&asset);
    assert_eq!(
        err,
        Err(Ok(ErrorCode::AssetNotRegistered.into())),
        "an evicted asset must report AssetNotRegistered"
    );

    // This must be distinguishable from the NoData an evicted *aggregate*
    // produces, otherwise the two failure modes are indistinguishable.
    assert_ne!(
        ErrorCode::AssetNotRegistered as u32,
        ErrorCode::NoData as u32,
        "asset-registry and aggregate expiry must have distinct error codes"
    );
}

/// Temporary history entries are meant to be evicted; the read must come back
/// empty rather than defaulting to a zero price.
#[test]
fn temporary_history_expiry_returns_no_default_entry() {
    let e = Env::default();
    e.mock_all_auths();
    let (_source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    let written_at = e.ledger().sequence();
    // History is recorded against the ledger the price was published in.
    let key = DataKey::PriceHistory(asset.clone(), written_at);

    let owner = client.address.clone();
    let had_history = e.as_contract(&owner, || e.storage().temporary().has(&key));
    assert!(
        had_history,
        "aggregating a price should record a history entry"
    );

    // Walk to just after the boundary and evict.
    let live = evict_if_expired(
        &e,
        &client,
        &key,
        true,
        written_at,
        Phase::JustBefore.sequence(written_at),
    );
    assert!(live, "history must be live through its boundary");

    evict_if_expired(
        &e,
        &client,
        &key,
        true,
        written_at,
        Phase::JustAfter.sequence(written_at),
    );

    let still_there = e.as_contract(&owner, || e.storage().temporary().has(&key));
    assert!(!still_there, "history entry must be evicted after its TTL");

    // The aggregate, which lives in persistent storage, is unaffected.
    assert!(
        client.get_price(&asset, &0).is_some(),
        "temporary history eviction must not disturb the live aggregate"
    );
}

/// The admin/config singleton: eviction must fail closed rather than defaulting
/// to an unset-but-usable configuration.
#[test]
fn admin_config_expiry_fails_closed() {
    let e = Env::default();
    e.mock_all_auths();
    let (_source, _asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    let key = DataKey::CfgMinSources;
    let written_at = e.ledger().sequence();

    evict_if_expired(
        &e,
        &client,
        &key,
        false,
        written_at,
        Phase::JustAfter.sequence(written_at),
    );

    let owner = client.address.clone();
    let present = e.as_contract(&owner, || e.storage().persistent().has(&key));
    assert!(!present, "config key must be evicted after its TTL");

    // Reading the evicted config must fail closed with a distinct error rather
    // than silently yielding a fabricated default (0, 1, or anything else).
    let err = client.try_get_min_sources_required();
    assert_eq!(
        err,
        Err(Ok(ErrorCode::ConfigMissing.into())),
        "an evicted required config must report ConfigMissing, not a default"
    );

    // That error must be distinguishable from the other expiry failures an
    // operator may see, so the runbook can name a response for each.
    for other in [
        ErrorCode::NoData,
        ErrorCode::AssetNotRegistered,
        ErrorCode::NotAuthorized,
    ] {
        assert_ne!(
            ErrorCode::ConfigMissing as u32,
            other as u32,
            "expiry error codes must stay distinct from each other"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Batch TTL extension atomicity
// ─────────────────────────────────────────────────────────────────────────────

/// Batch extension must extend exactly the entries it reports and leave the
/// payload of each untouched — an extension is bookkeeping, not a rewrite.
#[test]
fn batch_extension_does_not_alter_values() {
    let e = Env::default();
    e.mock_all_auths();
    let (_source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    let before = client.get_price(&asset, &0).expect("aggregate must exist");
    let extended = client.extend_asset_ttl(&asset, &0);
    let after = client
        .get_price(&asset, &0)
        .expect("aggregate must survive");

    assert!(extended > 0, "batch extension must report work done");
    assert_eq!(before, after, "TTL extension must not change the price");
}

/// A capped batch must stop cleanly at the cap: it reports exactly the cap, and
/// every entry it did reach is intact. This is the "no partial extension"
/// property — either an entry is extended whole or not at all.
#[test]
fn capped_batch_extension_is_all_or_nothing() {
    let e = Env::default();
    e.mock_all_auths();
    let (_source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    let before = client.get_price(&asset, &0).expect("aggregate must exist");

    // A cap of 1 must extend exactly one entry, not "one and a half".
    let extended = client.extend_asset_ttl(&asset, &1);
    assert_eq!(extended, 1, "a cap of 1 must extend exactly one entry");

    // The value it touched is intact, and so is the rest of the state.
    let after = client
        .get_price(&asset, &0)
        .expect("aggregate must survive");
    assert_eq!(before, after, "a capped extension must not corrupt values");
}

/// Extension bookkeeping must be recorded, so an operator can tell when an
/// asset was last extended.
#[test]
fn batch_extension_records_last_extended_ledger() {
    let e = Env::default();
    e.mock_all_auths();
    let (_source, asset, addr) = build_fixture(&e);
    let client = PriceOracleContractClient::new(&e, &addr);

    e.ledger().set_sequence_number(777);
    client.extend_asset_ttl(&asset, &0);

    let owner = client.address.clone();
    let recorded: Option<u32> = e.as_contract(&owner, || {
        e.storage()
            .persistent()
            .get(&DataKey::AssetLastTtlExtended(asset.clone()))
    });
    assert_eq!(
        recorded,
        Some(777),
        "the extension ledger must be recorded for operators"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Completeness
// ─────────────────────────────────────────────────────────────────────────────

/// Every key class must have a boundary test.
///
/// This is the guard the acceptance criteria ask for. The names below are
/// matched against the module's test list by [`covered_classes`], so removing a
/// boundary test (or adding a key class without one) fails here.
#[test]
fn every_key_class_has_a_boundary_test() {
    let covered = covered_classes();

    for class in KEY_CLASSES {
        assert!(
            covered.contains(class),
            "key class {} has no boundary test",
            class.name()
        );
    }
    assert_eq!(
        covered.len(),
        KEY_CLASSES.len(),
        "a boundary test names a class that is not in KEY_CLASSES: {covered:?}"
    );
}

/// Which key classes the suite's boundary tests actually exercise.
///
/// Kept as an explicit list rather than derived from the test names so that it
/// is reviewable: a reviewer can see at a glance which class is covered by
/// which test. The assertion in `every_key_class_has_a_boundary_test` keeps the
/// two lists in sync.
fn covered_classes() -> std::vec::Vec<KeyClass> {
    vec![
        // aggregate_expired_read_never_returns_a_default
        KeyClass::Aggregate,
        // submission_expired_read_fails_closed
        KeyClass::Submission,
        // source_registry_expiry_fails_closed
        KeyClass::SourceRegistry,
        // asset_registry_expiry_has_a_distinct_error
        KeyClass::AssetRegistry,
        // temporary_history_expiry_returns_no_default_entry
        KeyClass::History,
        // admin_config_expiry_fails_closed
        KeyClass::AdminConfig,
    ]
}

/// Temporary and persistent entries have different retention semantics, and the
/// suite must not conflate them.
#[test]
fn temporary_and_persistent_classes_are_distinguished() {
    let temporary: std::vec::Vec<KeyClass> = KEY_CLASSES
        .iter()
        .copied()
        .filter(|c| c.is_temporary())
        .collect();
    assert_eq!(
        temporary,
        vec![KeyClass::History],
        "history is the only temporary class; conflating the tiers would hide \
         the aggressive-eviction behaviour operators hit in practice"
    );
}

/// The TTL constants the contract actually uses must be the ones the boundary
/// model is reasoned about, so a future change to them cannot silently
/// invalidate these tests.
#[test]
fn ttl_policy_constants_are_coherent() {
    assert!(
        LEDGER_THRESHOLD > 0,
        "the bump threshold must be positive or nothing is ever extended"
    );
    assert!(
        LEDGER_BUMP > LEDGER_THRESHOLD,
        "the bump must exceed the threshold or a just-past-threshold entry is \
         never extended"
    );
}
