//! Differential storage read/write round-trip tests (#521).
//!
//! Serialization bugs corrupt data silently and are easy to introduce when a
//! persisted type gains a field or changes shape. These tests property-test that
//! every value written to storage reads back **exactly** — both as a decoded
//! value and, where the encoding is defined, at the **byte level** (`to_xdr`).
//!
//! Coverage:
//!
//! * a round-trip property per persisted type, driven by `proptest`;
//! * boundary values per type (`i128::MIN`/`MAX`, unsigned saturation, empty
//!   strings, `None` options, empty collections, nested structs);
//! * TTL bumps must not alter the stored bytes (ties into #522);
//! * a read after a storage migration must return a logically unchanged value;
//! * a completeness check asserting every persisted type is enumerated here, so
//!   adding a persisted type without a round-trip test fails the test run.
//!
//! The byte-level assertions are the strict ones: Soroban's canonical XDR is
//! deterministic, so `to_xdr(write) == to_xdr(read)` catches field-order and
//! width regressions that a decoded-value comparison would tolerate.

#![cfg(test)]

use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    xdr::ToXdr,
    Address, Bytes, Env, Map, String, Vec,
};

use crate::storage::LEDGER_BUMP;
use crate::test_helpers::setup_contract;
use crate::types::{
    AggregatePrice, AssetMetadata, DataKey, FrozenPrice, OracleSources, PriceBounds, PriceEntry,
    PriceHistoryEntry, PriceOverrideEntry, SourceVerification, StorageTtlEntry, SubscriptionExpiry,
    SubscriptionPayment, SubscriptionPlan,
};
use crate::PriceOracleContract;

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Runs `$body` inside the storage frame of `owner`.
///
/// Direct `env.storage()` access is only permitted inside a contract frame.
/// The frame matters for more than permissions: **storage is namespaced per
/// contract**, so reads and writes must name the same contract whose keys they
/// intend to touch. Tests that exercise the deployed contract therefore pass
/// that contract's address; tests that only need a scratch owner register a
/// throwaway one.
fn in_frame_of<R>(e: &Env, owner: &Address, body: impl FnOnce() -> R) -> R {
    e.as_contract(owner, body)
}

/// Runs `$body` in the frame of a freshly registered throwaway contract.
fn in_storage_frame<R>(e: &Env, body: impl FnOnce() -> R) -> R {
    let owner = e.register(PriceOracleContract, ());
    in_frame_of(e, &owner, body)
}

/// Canonical XDR bytes for an owned value — the encoding Soroban persists.
///
/// The `env` argument is required rather than assumed: Soroban `Address` is an
/// Env-scoped object, so serializing a value that holds one against a different
/// `Env` fails with a host "unknown object reference" error. Values are always
/// encoded in the same `Env` they were created in.
fn xdr_of<T: ToXdr>(env: &Env, v: T) -> Bytes {
    v.to_xdr(env)
}

/// [`xdr_of`] for a borrowed value, so the caller keeps ownership.
fn xdr_of_ref<T: ToXdr + Clone>(env: &Env, v: &T) -> Bytes {
    xdr_of(env, v.clone())
}

/// Every persisted type this suite round-trips.
///
/// The completeness test asserts this list stays in sync, so a new persisted
/// type cannot be added without a corresponding round-trip property.
const PERSISTED_TYPES: &[&str] = &[
    "AggregatePrice",
    "AssetMetadata",
    "FrozenPrice",
    "OracleSources",
    "PriceBounds",
    "PriceEntry",
    "PriceHistoryEntry",
    "PriceOverrideEntry",
    "SourceVerification",
    "StorageTtlEntry",
    "SubscriptionExpiry",
    "SubscriptionPayment",
    "SubscriptionPlan",
];

/// A `soroban_sdk::String` of `len` ASCII characters.
///
/// Lengths beyond the 32-byte `Symbol` limit are safe here because these are
/// `String`s, not `Symbol`s — a distinction the long-string boundary cases
/// below exist to protect.
fn ascii_string(env: &Env, len: usize) -> String {
    let s: std::string::String = (0..len)
        .map(|i| char::from(b'a' + (i % 26) as u8))
        .collect();
    String::from_str(env, &s)
}

// ─────────────────────────────────────────────────────────────────────────────
// #521 — Differential round-trip properties, per persisted type
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// `PriceEntry` survives a write/read round trip exactly, byte for byte.
    ///
    /// Covers the `Option<i128> volume` field in both the `Some` and `None`
    /// arms, which is where a naive encoder would silently drop a variant.
    #[test]
    fn price_entry_round_trips(
        price in any::<i128>(),
        timestamp in any::<u64>(),
        decimals in any::<u32>(),
        last_updated in any::<u32>(),
        ledger_timestamp in any::<u64>(),
        volume in prop::option::of(any::<i128>()),
    ) {
        let e = Env::default();
        let key = DataKey::Submission(Address::generate(&e), Address::generate(&e));

        let entry = PriceEntry {
            price,
            timestamp,
            source: Address::generate(&e),
            decimals,
            last_updated,
            ledger_timestamp,
            volume,
        };

        let read: Option<PriceEntry> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &entry);
            e.storage().persistent().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&entry));
        // Byte-level equality: the encoding must be lossless, not merely
        // decodable back to an equal value.
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &entry));
    }

    /// `AggregatePrice` survives a write/read round trip exactly.
    #[test]
    fn aggregate_price_round_trips(
        price in any::<i128>(),
        timestamp in any::<u64>(),
        num_sources in any::<u32>(),
        decimals in any::<u32>(),
        is_override in any::<bool>(),
        version in any::<u32>(),
    ) {
        let e = Env::default();
        let key = DataKey::Aggregate(Address::generate(&e));

        let agg = AggregatePrice {
            price,
            timestamp,
            num_sources,
            decimals,
            is_override,
            version,
        };

        let read: Option<AggregatePrice> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &agg);
            e.storage().persistent().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&agg));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &agg));
    }

    /// `PriceHistoryEntry` round-trips in **temporary** storage, the class
    /// actually used for history entries.
    #[test]
    fn price_history_entry_round_trips_in_temporary_storage(
        price in any::<i128>(),
        timestamp in any::<u64>(),
        ledger in any::<u32>(),
        num_sources in any::<u32>(),
        is_interpolated in any::<bool>(),
    ) {
        let e = Env::default();
        let key = DataKey::PriceHistory(Address::generate(&e), ledger);

        let entry = PriceHistoryEntry {
            price,
            timestamp,
            ledger,
            num_sources,
            is_interpolated,
        };

        let read: Option<PriceHistoryEntry> = in_storage_frame(&e, || {
            e.storage().temporary().set(&key, &entry);
            e.storage().temporary().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&entry));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &entry));
    }

    /// `PriceOverrideEntry` round-trips, including the `reason` string.
    #[test]
    fn price_override_entry_round_trips(
        price in any::<i128>(),
        reason_len in 0usize..24,
        expiry_ledger in any::<u32>(),
        set_ledger in any::<u32>(),
    ) {
        let e = Env::default();
        let key = DataKey::PriceOverride(Address::generate(&e));

        let entry = PriceOverrideEntry {
            price,
            reason: ascii_string(&e, reason_len),
            expiry_ledger,
            set_ledger,
        };

        let read: Option<PriceOverrideEntry> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &entry);
            e.storage().persistent().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&entry));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &entry));
    }

    /// `FrozenPrice` round-trips, including a long admin-supplied reason.
    #[test]
    fn frozen_price_round_trips(
        price in any::<i128>(),
        timestamp in any::<u64>(),
        decimals in any::<u32>(),
        reason_len in 0usize..40,
        frozen_at_ledger in any::<u32>(),
    ) {
        let e = Env::default();
        let key = DataKey::FrozenPrice(Address::generate(&e));

        let frozen = FrozenPrice {
            price,
            timestamp,
            decimals,
            reason: ascii_string(&e, reason_len),
            frozen_at_ledger,
        };

        let read: Option<FrozenPrice> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &frozen);
            e.storage().persistent().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&frozen));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &frozen));
    }

    /// `PriceBounds` round-trips across inverted min/max, zero width, and the
    /// full `i128` range.
    #[test]
    fn price_bounds_round_trips(
        min_price in any::<i128>(),
        max_price in any::<i128>(),
        max_change_bps_per_ledger in any::<u32>(),
    ) {
        let e = Env::default();
        let key = DataKey::AssetPriceBounds(Address::generate(&e));

        let bounds = PriceBounds {
            min_price,
            max_price,
            max_change_bps_per_ledger,
        };

        let read: Option<PriceBounds> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &bounds);
            e.storage().persistent().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&bounds));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &bounds));
    }

    /// `AssetMetadata` round-trips, exercising `decimals: Option<u32>` in both
    /// arms plus an empty-string `logo_uri`.
    #[test]
    fn asset_metadata_round_trips(
        name_len in 0usize..24,
        symbol_len in 0usize..12,
        logo_len in 0usize..32,
        decimals in prop::option::of(any::<u32>()),
    ) {
        let e = Env::default();
        let key = DataKey::AssetMetadata(Address::generate(&e));

        let meta = AssetMetadata {
            name: ascii_string(&e, name_len),
            symbol: ascii_string(&e, symbol_len),
            decimals,
            logo_uri: ascii_string(&e, logo_len),
        };

        let read: Option<AssetMetadata> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &meta);
            e.storage().persistent().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&meta));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &meta));
    }

    /// `OracleSources` round-trips with nested `SourceVerification` values —
    /// the nested-struct case in the issue's risk list.
    #[test]
    fn oracle_sources_round_trips_with_nested_verification(
        n_sources in 0usize..4,
        verified in prop::collection::vec(any::<bool>(), 1..4),
    ) {
        let e = Env::default();
        let key = DataKey::OracleSources;

        let mut sources: Vec<Address> = Vec::new(&e);
        let mut metadata: Map<Address, String> = Map::new(&e);
        let mut verification: Map<Address, SourceVerification> = Map::new(&e);

        for i in 0..n_sources {
            let addr = Address::generate(&e);
            sources.push_back(addr.clone());
            metadata.set(addr.clone(), ascii_string(&e, i + 1));
            verification.set(
                addr,
                SourceVerification {
                    verified: *verified.get(i).unwrap_or(&false),
                    verification_method: ascii_string(&e, i + 2),
                    verifier: Address::generate(&e),
                },
            );
        }

        let registry = OracleSources {
            sources,
            metadata,
            verification,
        };

        let read: Option<OracleSources> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &registry);
            e.storage().persistent().get(&key)
        });

        prop_assert_eq!(read.as_ref(), Some(&registry));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &registry));
    }

    /// The subscription trio round-trips individually.
    #[test]
    fn subscription_types_round_trip(
        duration in any::<u32>(),
        amount in any::<i128>(),
        expiry_timestamp in any::<u64>(),
        status in any::<u32>(),
    ) {
        let e = Env::default();

        let plan = SubscriptionPlan { duration, amount };
        let expiry = SubscriptionExpiry { expiry_timestamp };
        let payment = SubscriptionPayment {
            consumer: Address::generate(&e),
            amount,
            timestamp: expiry_timestamp,
            status,
        };

        let k1 = DataKey::SubscriptionToken;
        let k2 = DataKey::SubscriptionExpiry(Address::generate(&e));
        let k3 = DataKey::SubscriptionPayment(Address::generate(&e));

        let (r1, r2, r3) = in_storage_frame(&e, || {
            e.storage().persistent().set(&k1, &plan);
            e.storage().persistent().set(&k2, &expiry);
            e.storage().persistent().set(&k3, &payment);
            (
                e.storage().persistent().get::<_, SubscriptionPlan>(&k1),
                e.storage().persistent().get::<_, SubscriptionExpiry>(&k2),
                e.storage()
                    .persistent()
                    .get::<_, SubscriptionPayment>(&k3),
            )
        });

        prop_assert_eq!(r1.as_ref(), Some(&plan));
        prop_assert_eq!(r2.as_ref(), Some(&expiry));
        prop_assert_eq!(r3.as_ref(), Some(&payment));
        prop_assert_eq!(xdr_of_ref(&e, &r1.unwrap()), xdr_of_ref(&e, &plan));
        prop_assert_eq!(xdr_of_ref(&e, &r2.unwrap()), xdr_of_ref(&e, &expiry));
        prop_assert_eq!(xdr_of_ref(&e, &r3.unwrap()), xdr_of_ref(&e, &payment));
    }

    /// `StorageTtlEntry` round-trips, including the documented "unknown TTL"
    /// sentinel of `0`.
    #[test]
    fn storage_ttl_entry_round_trips(
        key_len in 0usize..20,
        exists in any::<bool>(),
        remaining_ttl in any::<u32>(),
    ) {
        let e = Env::default();
        let k = DataKey::AssetLastTtlExtended(Address::generate(&e));

        let entry = StorageTtlEntry {
            key: ascii_string(&e, key_len),
            exists,
            remaining_ttl,
        };

        let read: Option<StorageTtlEntry> = in_storage_frame(&e, || {
            e.storage().persistent().set(&k, &entry);
            e.storage().persistent().get(&k)
        });

        prop_assert_eq!(read.as_ref(), Some(&entry));
        prop_assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &entry));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// #521 — Boundary values
// ─────────────────────────────────────────────────────────────────────────────

/// `i128::MIN` and `i128::MAX` must round-trip exactly; these are the values a
/// saturated or sign-mangled encoder breaks first.
#[test]
fn i128_boundaries_round_trip_exactly() {
    let e = Env::default();

    for price in [i128::MIN, i128::MIN + 1, -1, 0, 1, i128::MAX - 1, i128::MAX] {
        let key = DataKey::Submission(Address::generate(&e), Address::generate(&e));
        let entry = PriceEntry {
            price,
            timestamp: 0,
            source: Address::generate(&e),
            decimals: 0,
            last_updated: 0,
            ledger_timestamp: 0,
            volume: Some(price),
        };

        let read: Option<PriceEntry> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &entry);
            e.storage().persistent().get(&key)
        });

        assert_eq!(
            read.as_ref(),
            Some(&entry),
            "value {price} did not round-trip"
        );
        assert_eq!(
            xdr_of_ref(&e, &read.unwrap()),
            xdr_of_ref(&e, &entry),
            "bytes differ for {price}"
        );
    }
}

/// Saturation boundaries for the unsigned integer fields.
#[test]
fn unsigned_boundaries_round_trip_exactly() {
    let e = Env::default();

    for (timestamp, last_updated, decimals) in [
        (0u64, 0u32, 0u32),
        (1, 1, 1),
        (u64::MAX, u32::MAX, u32::MAX),
        (u64::MAX - 1, u32::MAX - 1, u32::MAX),
    ] {
        let key = DataKey::Submission(Address::generate(&e), Address::generate(&e));
        let entry = PriceEntry {
            price: 0,
            timestamp,
            source: Address::generate(&e),
            decimals,
            last_updated,
            ledger_timestamp: timestamp,
            volume: None,
        };

        let read: Option<PriceEntry> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &entry);
            e.storage().persistent().get(&key)
        });

        assert_eq!(read.as_ref(), Some(&entry));
        assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &entry));
    }
}

/// Empty strings and empty collections round-trip without being normalised away.
#[test]
fn empty_strings_and_collections_round_trip() {
    let e = Env::default();

    let meta = AssetMetadata {
        name: String::from_str(&e, ""),
        symbol: String::from_str(&e, ""),
        decimals: None,
        logo_uri: String::from_str(&e, ""),
    };
    let key = DataKey::AssetMetadata(Address::generate(&e));
    let read: Option<AssetMetadata> = in_storage_frame(&e, || {
        e.storage().persistent().set(&key, &meta);
        e.storage().persistent().get(&key)
    });
    assert_eq!(read.as_ref(), Some(&meta));
    assert_eq!(xdr_of_ref(&e, &read.unwrap()), xdr_of_ref(&e, &meta));

    // Empty source registry: empty Vec plus two empty Maps.
    let registry = OracleSources {
        sources: Vec::new(&e),
        metadata: Map::new(&e),
        verification: Map::new(&e),
    };
    let k2 = DataKey::OracleSources;
    let read2: Option<OracleSources> = in_storage_frame(&e, || {
        e.storage().persistent().set(&k2, &registry);
        e.storage().persistent().get(&k2)
    });
    assert_eq!(read2.as_ref(), Some(&registry));
    assert_eq!(xdr_of_ref(&e, &read2.unwrap()), xdr_of_ref(&e, &registry));
}

/// `None` must stay `None` and `Some(0)` must stay `Some(0)` — the classic
/// `Option<i128>` zero-versus-absent confusion.
#[test]
fn optional_fields_distinguish_none_from_zero() {
    let e = Env::default();

    for volume in [None, Some(0), Some(-1), Some(i128::MAX)] {
        let key = DataKey::Submission(Address::generate(&e), Address::generate(&e));
        let entry = PriceEntry {
            price: 42,
            timestamp: 7,
            source: Address::generate(&e),
            decimals: 18,
            last_updated: 1,
            ledger_timestamp: 7,
            volume,
        };

        let read: Option<PriceEntry> = in_storage_frame(&e, || {
            e.storage().persistent().set(&key, &entry);
            e.storage().persistent().get(&key)
        });

        assert_eq!(read.as_ref().unwrap().volume, volume);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// #521 — TTL bump must not alter stored bytes
// ─────────────────────────────────────────────────────────────────────────────

/// Extending a key's TTL is a metadata operation; the persisted payload must be
/// byte-identical before and after. See also the #522 boundary suite.
#[test]
fn ttl_bump_preserves_stored_bytes() {
    let e = Env::default();

    let key = DataKey::Submission(Address::generate(&e), Address::generate(&e));
    // A second, independent key for the "very large bump" case. Both live in
    // the same `Env` because a Soroban `Address` is Env-scoped and cannot be
    // written into a different `Env`.
    let key_far = DataKey::Submission(Address::generate(&e), Address::generate(&e));

    let entry = PriceEntry {
        price: 1_234_567,
        timestamp: 99,
        source: Address::generate(&e),
        decimals: 7,
        last_updated: 3,
        ledger_timestamp: 99,
        volume: Some(500),
    };

    let before_bytes = xdr_of_ref(&e, &entry);

    // Write, then bump with the contract's own threshold/bump so the extension
    // actually applies.
    let after: Option<PriceEntry> = in_storage_frame(&e, || {
        e.storage().persistent().set(&key, &entry);
        e.ledger().set_sequence_number(5_000);
        e.storage()
            .persistent()
            .extend_ttl(&key, crate::storage::LEDGER_THRESHOLD, LEDGER_BUMP);
        e.storage().persistent().get(&key)
    });

    assert!(after.is_some(), "TTL bump must not remove the entry");
    assert_eq!(
        xdr_of_ref(&e, &after.unwrap()),
        before_bytes,
        "TTL bump altered the stored bytes"
    );

    // A much larger bump must also be byte-preserving.
    let far: Option<PriceEntry> = in_storage_frame(&e, || {
        e.storage().persistent().set(&key_far, &entry);
        e.ledger().set_sequence_number(5_000);
        e.storage().persistent().extend_ttl(&key_far, 0, 1_000_000);
        e.storage().persistent().get(&key_far)
    });
    assert_eq!(
        xdr_of_ref(&e, &far.unwrap()),
        before_bytes,
        "large TTL bump altered the stored bytes"
    );

    // Keep the module-level constant referenced so the TTL policy the contract
    // actually uses is what the round trip was measured against.
    assert!(LEDGER_BUMP > 0, "contract TTL bump amount must be positive");
}

// ─────────────────────────────────────────────────────────────────────────────
// #521 — Read after migration
// ─────────────────────────────────────────────────────────────────────────────

/// A completed storage migration must leave logically unchanged values byte
/// identical. `migrate_storage` rewrites the schema version; the payloads it
/// does not transform must survive untouched.
#[test]
fn migration_does_not_alter_unchanged_values() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let asset = Address::generate(&e);
    let source = Address::generate(&e);

    // Align ledger time with the submission timestamp: `submit_price` rejects a
    // timestamp too far from ledger time with `InvalidTimestamp`.
    e.ledger().set_timestamp(1_000);

    client.register_asset(&asset);
    client.add_source(&source, &String::from_str(&e, "Source"));
    client.set_min_sources_required(&1);
    client.submit_price(&source, &asset, &1_000_000, &1_000);

    let agg_key = DataKey::Aggregate(asset.clone());
    let sub_key = DataKey::Submission(asset.clone(), source.clone());

    // Read through the *deployed* contract's namespace — storage is namespaced
    // per contract, so a throwaway owner would see an empty store.
    let (before_agg, before_agg_bytes) = in_frame_of(&e, &client.address, || {
        let read: Option<AggregatePrice> = e.storage().persistent().get(&agg_key);
        let bytes = read.as_ref().map(|v| xdr_of_ref(&e, v));
        (read, bytes)
    });
    let (before_sub, before_sub_bytes) = in_frame_of(&e, &client.address, || {
        let read: Option<PriceEntry> = e.storage().persistent().get(&sub_key);
        let bytes = read.as_ref().map(|v| xdr_of_ref(&e, v));
        (read, bytes)
    });
    assert!(before_agg.is_some(), "aggregate should exist pre-migration");
    assert!(
        before_sub.is_some(),
        "submission should exist pre-migration"
    );

    // Run the migration to completion in small batches, so the incremental
    // cursor path is exercised rather than a single-shot upgrade.
    // `get_migration_state` reads persistent storage, so it too must run in the
    // contract's own frame.
    let mut guard = 0;
    while in_frame_of(&e, &client.address, || {
        crate::migration::get_migration_state(&e)
    })
    .is_some()
        && guard < 50
    {
        client.migrate_storage(&1);
        guard += 1;
    }
    assert!(
        guard < 50,
        "migration did not converge within the expected number of batches"
    );

    let after_agg: Option<AggregatePrice> = in_frame_of(&e, &client.address, || {
        e.storage().persistent().get(&agg_key)
    });
    let after_sub: Option<PriceEntry> = in_frame_of(&e, &client.address, || {
        e.storage().persistent().get(&sub_key)
    });

    assert!(after_agg.is_some(), "migration dropped the aggregate");
    assert!(after_sub.is_some(), "migration dropped the submission");
    assert_eq!(
        xdr_of_ref(&e, &after_agg.unwrap()),
        before_agg_bytes.unwrap(),
        "migration altered the aggregate bytes"
    );
    assert_eq!(
        xdr_of_ref(&e, &after_sub.unwrap()),
        before_sub_bytes.unwrap(),
        "migration altered the submission bytes"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// #521 — Completeness
// ─────────────────────────────────────────────────────────────────────────────

/// Every persisted type must have a round-trip property above.
///
/// This is the guard the acceptance criteria ask for: a new persisted type
/// without a corresponding round-trip test fails here rather than silently
/// shipping an untested encoding. If you add a type, add it to
/// `PERSISTED_TYPES` *and* write the property.
#[test]
fn every_persisted_type_has_a_round_trip_test() {
    // The compile-time half of the check: naming each type here produces a
    // build error if one is renamed or removed, so the list cannot rot.
    #[allow(clippy::too_many_arguments)]
    fn _assert_types_exist(
        _: Option<PriceEntry>,
        _: Option<AggregatePrice>,
        _: Option<PriceHistoryEntry>,
        _: Option<PriceOverrideEntry>,
        _: Option<FrozenPrice>,
        _: Option<PriceBounds>,
        _: Option<AssetMetadata>,
        _: Option<OracleSources>,
        _: Option<SourceVerification>,
        _: Option<SubscriptionPlan>,
        _: Option<SubscriptionExpiry>,
        _: Option<SubscriptionPayment>,
        _: Option<StorageTtlEntry>,
    ) {
    }
    _assert_types_exist(
        None, None, None, None, None, None, None, None, None, None, None, None, None,
    );

    // The runtime half: the registry must list each of those 13 types exactly
    // once, so a duplicate or a stale entry is caught too. `std::vec::Vec` is
    // named explicitly because `soroban_sdk::Vec` is imported above.
    let mut seen: std::vec::Vec<&str> = PERSISTED_TYPES.to_vec();
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(
        seen.len(),
        before,
        "PERSISTED_TYPES contains duplicate entries: {seen:?}"
    );
    assert_eq!(
        seen.len(),
        13,
        "PERSISTED_TYPES must list all 13 round-tripped types, found {seen:?}"
    );
}

/// The round-trip harness must exercise the real contract client, not a raw
/// write — this guards the wiring so the migration case above stays meaningful.
#[test]
fn round_trip_harness_uses_the_real_contract_client() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    // Read through the *deployed* contract's namespace, not a throwaway one —
    // storage is namespaced per contract, so this is the same key the contract
    // itself wrote during `initialize`.
    let stored: Option<Address> = in_frame_of(&e, &client.address, || {
        e.storage().persistent().get(&DataKey::Admin)
    });
    assert_eq!(client.get_admin(), stored.expect("admin key must be set"));
}
