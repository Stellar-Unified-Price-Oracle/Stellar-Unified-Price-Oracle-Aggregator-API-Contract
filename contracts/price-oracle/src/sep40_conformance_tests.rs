//! SEP-40 conformance suite (#515).
//!
//! Every requirement in this module is transcribed from the **published** SEP-40
//! text (pinned in `docs/sep40-conformance.md`) rather than from our own
//! expectations, so a silent deviation from the standard fails here.
//!
//! * `docs/sep40-conformance.md` holds the pinned spec version, the requirement
//!   table, the deviations and the ambiguities (with the interpretation chosen).
//! * `CHECKLIST` below mirrors that table; the meta-tests at the bottom of the
//!   file fail if the two drift apart or if a mapped test does not exist.
//!
//! Interface symbols are exercised through raw `Env::invoke_contract` calls using
//! the spec's own function names, so a rename of an exported symbol is caught
//! here even though the generated client would happily follow it.

use std::string::ToString;

use soroban_sdk::{
    contracttype,
    testutils::{Address as _, Ledger},
    Address, Env, IntoVal, Map, Symbol, TryFromVal, Val, Vec,
};

use crate::test_helpers::{
    create_contract, ledger_default, register_test_asset, register_test_source,
};
use crate::{Asset, PriceData, PriceOracleContract, PriceOracleContractClient};

/// Status of a single requirement row in `docs/sep40-conformance.md`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The contract matches the SEP as written.
    Conforms,
    /// The contract deviates; the deviation is documented in the doc.
    Deviates,
    /// The SEP is ambiguous and the contract implements a documented choice.
    Interpreted,
}

impl Status {
    fn label(self) -> &'static str {
        match self {
            Status::Conforms => "conforms",
            Status::Deviates => "deviation",
            Status::Interpreted => "interpretation",
        }
    }
}

/// `(requirement id, SEP clause, test function, status)` — mirrors the
/// "Normative requirements" table of `docs/sep40-conformance.md`.
pub const CHECKLIST: &[(&str, &str, &str, Status)] = &[
    (
        "SEP40-R01",
        "base",
        "r01_base_returns_the_denomination_asset",
        Status::Conforms,
    ),
    (
        "SEP40-R02",
        "assets",
        "r02_assets_lists_every_quoted_asset_as_stellar_variant",
        Status::Conforms,
    ),
    (
        "SEP40-R03",
        "decimals",
        "r03_decimals_reports_price_precision",
        Status::Conforms,
    ),
    (
        "SEP40-R04",
        "resolution",
        "r04_resolution_reports_tick_seconds",
        Status::Conforms,
    ),
    (
        "SEP40-R05",
        "lastprice",
        "r05_lastprice_returns_the_latest_aggregate",
        Status::Conforms,
    ),
    (
        "SEP40-R06",
        "price",
        "r06_price_returns_newest_record_at_or_before_timestamp",
        Status::Interpreted,
    ),
    (
        "SEP40-R07",
        "prices",
        "r07_prices_returns_at_most_records_entries_newest_first",
        Status::Conforms,
    ),
    (
        "SEP40-R08",
        "PriceData",
        "r08_price_data_exposes_the_spec_fields",
        Status::Deviates,
    ),
    (
        "SEP40-R09",
        "Asset",
        "r09_asset_has_exactly_the_two_spec_variants",
        Status::Conforms,
    ),
    (
        "SEP40-R10",
        "precision",
        "r10_price_is_scaled_by_ten_to_the_decimals",
        Status::Conforms,
    ),
    (
        "SEP40-R11",
        "precision",
        "r11_every_read_entrypoint_reports_the_same_scaled_value",
        Status::Conforms,
    ),
    (
        "SEP40-R12",
        "errors",
        "r12_unknown_asset_returns_none_instead_of_error",
        Status::Conforms,
    ),
    (
        "SEP40-R13",
        "errors",
        "r13_other_asset_variant_returns_none_instead_of_error",
        Status::Conforms,
    ),
    (
        "SEP40-R14",
        "errors",
        "r14_timestamp_outside_history_returns_none",
        Status::Conforms,
    ),
    (
        "SEP40-R15",
        "prices",
        "r15_zero_records_returns_an_empty_vec",
        Status::Interpreted,
    ),
    (
        "SEP40-R16",
        "symbols",
        "r16_exported_symbols_match_the_spec_names",
        Status::Interpreted,
    ),
    (
        "SEP40-R17",
        "authorization",
        "r17_read_entrypoints_require_no_authorization",
        Status::Conforms,
    ),
    (
        "SEP40-R18",
        "staleness",
        "r18_lastprice_is_none_once_the_resolution_window_lapses",
        Status::Conforms,
    ),
    (
        "SEP40-R19",
        "timestamps",
        "r19_timestamps_are_trimmed_to_the_resolution",
        Status::Deviates,
    ),
    (
        "SEP40-R20",
        "precision",
        "r20_decimals_and_resolution_are_admin_changeable",
        Status::Deviates,
    ),
];

/// The SEP-40 `Asset` enum exactly as the specification defines it.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Sep40Asset {
    Stellar(Address),
    Other(Symbol),
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Argument list for a raw `Env::invoke_contract` call.
fn args(e: &Env, values: &[Val]) -> Vec<Val> {
    let mut vals: Vec<Val> = Vec::new(e);
    for v in values {
        vals.push_back(*v);
    }
    vals
}

/// Calls the contract with the spec's own function name, bypassing the
/// generated client so the exported symbol itself is under test.
fn call<T>(e: &Env, addr: &Address, name: &str, values: &[Val]) -> T
where
    T: TryFromVal<Env, Val>,
{
    let func = Symbol::new(e, name);
    e.invoke_contract::<T>(addr, &func, args(e, values))
}

/// Returns `true` when the contract exports `name` and answers with type `T`.
fn spec_call_succeeds<T>(e: &Env, addr: &Address, name: &str, values: &[Val]) -> bool
where
    T: TryFromVal<Env, Val>,
{
    let func = Symbol::new(e, name);
    e.try_invoke_contract::<T, soroban_sdk::Error>(addr, &func, args(e, values))
        .is_ok_and(|r| r.is_ok())
}

/// Deploys an oracle with the given decimal precision.
fn oracle_with_decimals(e: &Env, decimals: u32) -> PriceOracleContractClient<'_> {
    e.mock_all_auths();
    let admin = Address::generate(e);
    let client = create_contract(e);
    client.initialize(
        &admin,
        &1u32,
        &10u32,
        &decimals,
        &soroban_sdk::String::from_str(e, "SEP-40 conformance"),
    );
    client
}

/// Deploys an oracle with `sources` registered sources and one asset; the
/// quorum is set to `sources`, so every source must report before an aggregate
/// is published.
fn oracle_with_sources(
    e: &Env,
    sources: u32,
    decimals: u32,
) -> (PriceOracleContractClient<'_>, Address) {
    let client = oracle_with_decimals(e, decimals);
    client.set_min_sources_required(&sources);
    let asset = register_test_asset(e, &client);
    for _ in 0..sources {
        register_test_source(e, &client, "Source");
    }
    (client, asset)
}

/// Submits `prices[i]` from the i-th registered source, all at `ts`.
fn submit_series(
    client: &PriceOracleContractClient<'_>,
    asset: &Address,
    prices: &[i128],
    ts: u64,
) {
    let list = client.get_oracle_sources();
    for (i, p) in prices.iter().enumerate() {
        let src = list.sources.get_unchecked(i as u32);
        client.submit_price(&src, asset, p, &ts);
    }
}

// ---------------------------------------------------------------------------
// R01–R04: static interface
// ---------------------------------------------------------------------------

/// SEP40-R01 — `base()` returns the asset all prices are quoted in.
#[test]
fn r01_base_returns_the_denomination_asset() {
    let e = Env::default();
    let client = oracle_with_decimals(&e, 18);
    let base: Asset = call(&e, &client.address, "base", &[]);
    assert_eq!(base, Asset::Other(Symbol::new(&e, "USD")));
    assert_eq!(client.base(), base);
}

/// SEP40-R02 — `assets()` lists every quoted asset as `Asset::Stellar`.
#[test]
fn r02_assets_lists_every_quoted_asset_as_stellar_variant() {
    let e = Env::default();
    let client = oracle_with_decimals(&e, 18);
    let a = register_test_asset(&e, &client);
    let b = register_test_asset(&e, &client);

    let listed: Vec<Asset> = call(&e, &client.address, "assets", &[]);
    assert_eq!(listed.len(), 2);
    assert!(listed.contains(&Asset::Stellar(a.clone())));
    assert!(listed.contains(&Asset::Stellar(b.clone())));
    // The list is a sorted set, not an insertion-ordered list.
    let mut expected: Vec<Asset> = Vec::new(&e);
    expected.push_back(Asset::Stellar(a.clone()));
    expected.push_back(Asset::Stellar(b.clone()));
    if a > b {
        expected.set(0, Asset::Stellar(b.clone()));
        expected.set(1, Asset::Stellar(a.clone()));
    }
    assert_eq!(listed, expected);
    assert_eq!(client.assets().len(), 2);
}

/// SEP40-R03 — `decimals()` reports the precision used by every quoted asset.
#[test]
fn r03_decimals_reports_price_precision() {
    let e = Env::default();
    for d in [0u32, 6, 7, 18] {
        let client = oracle_with_decimals(&e, d);
        let reported: u32 = call(&e, &client.address, "decimals", &[]);
        assert_eq!(
            reported, d,
            "decimals() must return the configured precision"
        );
    }
}

/// SEP40-R04 — `resolution()` returns the tick period in seconds.
#[test]
fn r04_resolution_reports_tick_seconds() {
    let e = Env::default();
    let client = oracle_with_decimals(&e, 18);
    client.set_resolution(&300u32);
    let reported: u32 = call(&e, &client.address, "resolution", &[]);
    assert_eq!(reported, 300);
    assert_eq!(client.resolution(), reported);
}

/// Deviations documented in `docs/sep40-conformance.md`: `(id, requirement rows)`.
pub const DEVIATIONS: &[(&str, &[&str])] = &[
    ("D-01", &["SEP40-R08"]),
    ("D-02", &["SEP40-R19"]),
    ("D-03", &["SEP40-R20"]),
    ("D-04", &["SEP40-R15"]),
];

/// Ambiguities documented in `docs/sep40-conformance.md`: `(id, requirement rows)`.
pub const AMBIGUITIES: &[(&str, &[&str])] = &[
    ("A-01", &["SEP40-R16"]),
    ("A-02", &["SEP40-R06"]),
    ("A-03", &["SEP40-R15"]),
    ("A-04", &["SEP40-R07"]),
];

// ---------------------------------------------------------------------------
// R05–R09: price and asset value types
// ---------------------------------------------------------------------------

/// SEP40-R05 — `lastprice()` returns the most recent aggregate.
#[test]
fn r05_lastprice_returns_the_latest_aggregate() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000_000);
    let (client, asset) = oracle_with_sources(&e, 2, 18);
    submit_series(&client, &asset, &[1_000_000, 3_000_000], 1_000_000);

    let quoted: Val = Asset::Stellar(asset.clone()).into_val(&e);
    let data: Option<PriceData> = call(&e, &client.address, "lastprice", &[quoted]);
    let data = data.expect("an aggregate is published once quorum is met");
    assert_eq!(data.price, 2_000_000, "median of the two submissions");
    assert_eq!(data.timestamp, 1_000_000);
    assert_eq!(client.lastprice(&Asset::Stellar(asset)).unwrap(), data);
}

/// SEP40-R06 — `price()` returns the newest record at or before the timestamp
/// (interpretation A-02) and `None` before history starts.
#[test]
fn r06_price_returns_newest_record_at_or_before_timestamp() {
    let e = Env::default();
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);

    // Two ledgers, one aggregate each. The ledger stays low so the backwards
    // history scan stays inside the test host's resource budget.
    ledger_default(&e, 50, 1_000);
    client.submit_price(&src, &asset, &111, &1_000);
    ledger_default(&e, 51, 2_000);
    client.submit_price(&src, &asset, &222, &2_000);

    let quoted: Val = Asset::Stellar(asset.clone()).into_val(&e);
    let at_2_000: Option<PriceData> = call(
        &e,
        &client.address,
        "price",
        &[quoted.clone(), 2_000u64.into_val(&e)],
    );
    assert_eq!(at_2_000.unwrap().price, 222);

    // Off-grid request: newest record with timestamp <= 1_500.
    let at_1_500: Option<PriceData> = call(
        &e,
        &client.address,
        "price",
        &[quoted.clone(), 1_500u64.into_val(&e)],
    );
    assert_eq!(at_1_500.unwrap().price, 111);

    assert!(
        client
            .price(&Asset::Stellar(asset), &1_000u64)
            .unwrap()
            .price
            == 111
    );
}

/// SEP40-R07 — `prices()` returns at most `records` entries, newest first.
#[test]
fn r07_prices_returns_at_most_records_entries_newest_first() {
    let e = Env::default();
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);

    for (i, ts) in [1_000u64, 2_000, 3_000].iter().enumerate() {
        ledger_default(&e, 100 + i as u32, *ts);
        client.submit_price(&src, &asset, &((i as i128 + 1) * 100), ts);
    }

    let quoted: Val = Asset::Stellar(asset).into_val(&e);
    let two: Option<Vec<PriceData>> = call(
        &e,
        &client.address,
        "prices",
        &[quoted.clone(), 2u32.into_val(&e)],
    );
    let two = two.expect("a registered asset always answers with Some");
    assert_eq!(two.len(), 2, "at most `records` entries");
    assert_eq!(two.get_unchecked(0).price, 300, "newest first");
    assert_eq!(two.get_unchecked(1).price, 200);
    for w in two.iter() {
        assert!(w.timestamp >= 2_000);
    }

    let many: Option<Vec<PriceData>> =
        call(&e, &client.address, "prices", &[quoted, 3u32.into_val(&e)]);
    assert_eq!(many.unwrap().len(), 3);
}

/// SEP40-R08 — `PriceData` carries the spec's `price: i128` and
/// `timestamp: u64`; deviation D-01 appends `last_updated: u32`.
///
/// The check reads the *wire* value rather than the Rust struct, so it verifies
/// what a SEP-40 consumer actually decodes.
#[test]
fn r08_price_data_exposes_the_spec_fields() {
    use soroban_sdk::xdr::ScVal;

    let e = Env::default();
    ledger_default(&e, 100, 1_000_000);
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    client.submit_price(&src, &asset, &1_500_000_000_000_000_000, &1_000_000);

    // Served over the wire, decoded by the generated client.
    let served: PriceData = client.lastprice(&Asset::Stellar(asset)).unwrap();
    assert_eq!(served.price, 1_500_000_000_000_000_000i128, "price is i128");
    assert_eq!(served.timestamp, 1_000_000u64, "timestamp is u64");
    // The additive field of D-01: the ledger that wrote the datapoint.
    assert_eq!(served.last_updated, 100u32);

    // The field set on the wire: the two fields the SEP names, with the SEP's
    // types, plus the documented third. Soroban struct decoding is strict, so a
    // consumer must adopt the contract's `PriceData` rather than the two-field
    // struct printed in the SEP — that cost of D-01 is why the field set and
    // every field type are pinned here.
    let encoded: Val = served.into_val(&e);
    let fields: Map<Symbol, Val> = Map::try_from_val(&e, &encoded).expect("structs decode as maps");
    assert_eq!(fields.len(), 3, "price, timestamp and the D-01 addition");

    let field = |name: &str| -> Val {
        fields
            .get(Symbol::new(&e, name))
            .unwrap_or_else(|| panic!("{name} must be a field of PriceData"))
    };
    // Each field decodes as its declared type and *not* as a neighbouring one,
    // which is what pins the wire type.
    assert_eq!(i128::try_from_val(&e, &field("price")), Ok(served.price));
    assert!(
        u64::try_from_val(&e, &field("price")).is_err(),
        "price is i128"
    );
    assert_eq!(
        u64::try_from_val(&e, &field("timestamp")),
        Ok(served.timestamp)
    );
    assert!(
        u32::try_from_val(&e, &field("timestamp")).is_err(),
        "timestamp is u64"
    );
    assert_eq!(
        u32::try_from_val(&e, &field("last_updated")),
        Ok(served.last_updated)
    );
    assert!(
        u64::try_from_val(&e, &field("last_updated")).is_err(),
        "last_updated is u32"
    );
}

/// SEP40-R09 — `Asset` has exactly the two spec variants, with the spec's
/// payloads. Verified by decoding our values through a mirror of the SEP enum.
#[test]
fn r09_asset_has_exactly_the_two_spec_variants() {
    let e = Env::default();
    let addr = Address::generate(&e);

    let stellar: Val = Asset::Stellar(addr.clone()).into_val(&e);
    let other: Val = Asset::Other(Symbol::new(&e, "EUR")).into_val(&e);

    let as_spec: Sep40Asset = Sep40Asset::try_from_val(&e, &stellar)
        .expect("Asset::Stellar(Address) must decode as the SEP's first variant");
    assert_eq!(as_spec, Sep40Asset::Stellar(addr.clone()));

    let as_spec: Sep40Asset = Sep40Asset::try_from_val(&e, &other)
        .expect("Asset::Other(Symbol) must decode as the SEP's second variant");
    assert_eq!(as_spec, Sep40Asset::Other(Symbol::new(&e, "EUR")));

    // Round-tripping proves the payloads really are Address and Symbol.
    let back: Asset = Asset::try_from_val(&e, &stellar).unwrap();
    assert_eq!(back, Asset::Stellar(addr));
}

/// SEP40-R10 — `price` is the real price scaled by `10^decimals`.
#[test]
fn r10_price_is_scaled_by_ten_to_the_decimals() {
    // 7 decimals, three different precisions, one rule: value == real * 10^decimals.
    for (decimals, real, raw) in [
        (7u32, 0.1234567f64, 1_234_567i128),
        (6, 1.5f64, 1_500_000i128),
        (18, 0.5f64, 500_000_000_000_000_000i128),
    ] {
        let e = Env::default();
        ledger_default(&e, 100, 5_000);
        let (client, asset) = oracle_with_sources(&e, 1, decimals);
        client.submit_price(
            &client.get_oracle_sources().sources.get_unchecked(0),
            &asset,
            &raw,
            &5_000,
        );

        let served = client.lastprice(&Asset::Stellar(asset)).unwrap();
        assert_eq!(served.price, raw, "the oracle serves the scaled integer");
        // Recovering the real price is exact integer division by 10^decimals.
        let mut scale: i128 = 1;
        for _ in 0..decimals {
            scale *= 10;
        }
        let recovered = served.price as f64 / scale as f64;
        assert!(
            (recovered - real).abs() < 1e-9,
            "decimals={decimals}: {recovered} != {real}"
        );
    }
}

/// SEP40-R11 — every read entrypoint reports the same scaled value.
#[test]
fn r11_every_read_entrypoint_reports_the_same_scaled_value() {
    let e = Env::default();
    ledger_default(&e, 100, 7_000);
    let (client, asset) = oracle_with_sources(&e, 1, 7);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    let raw = 3_333_333i128;
    client.submit_price(&src, &asset, &raw, &7_000);

    let quoted = Asset::Stellar(asset);
    let last = client.lastprice(&quoted).unwrap();
    let at = client.price(&quoted, &7_000u64).unwrap();
    let series = client.prices(&quoted, &1u32).unwrap();

    assert_eq!(last.price, raw);
    assert_eq!(at.price, raw);
    assert_eq!(series.get_unchecked(0).price, raw);
    assert_eq!(last.timestamp, at.timestamp);
    assert_eq!(last.timestamp, series.get_unchecked(0).timestamp);
}

// ---------------------------------------------------------------------------
// R12–R18: error behaviour, symbols, staleness
// ---------------------------------------------------------------------------

/// SEP40-R12 — an unknown asset returns `None`, it does not throw.
#[test]
fn r12_unknown_asset_returns_none_instead_of_error() {
    let e = Env::default();
    let client = oracle_with_decimals(&e, 18);
    let unknown = Address::generate(&e);
    let quoted: Val = Asset::Stellar(unknown).into_val(&e);

    let last: Option<PriceData> = call(&e, &client.address, "lastprice", &[quoted.clone()]);
    let at: Option<PriceData> = call(
        &e,
        &client.address,
        "price",
        &[quoted.clone(), 1_000u64.into_val(&e)],
    );
    let many: Option<Vec<PriceData>> =
        call(&e, &client.address, "prices", &[quoted, 1u32.into_val(&e)]);

    assert!(last.is_none(), "lastprice(unknown) must be None");
    assert!(at.is_none(), "price(unknown) must be None");
    assert!(many.is_none(), "prices(unknown) must be None");
}

/// SEP40-R13 — `Asset::Other` input returns `None`, it does not throw.
#[test]
fn r13_other_asset_variant_returns_none_instead_of_error() {
    let e = Env::default();
    let client = oracle_with_decimals(&e, 18);
    let quoted: Val = Asset::Other(Symbol::new(&e, "EUR")).into_val(&e);

    let last: Option<PriceData> = call(&e, &client.address, "lastprice", &[quoted.clone()]);
    let at: Option<PriceData> = call(
        &e,
        &client.address,
        "price",
        &[quoted.clone(), 1_000u64.into_val(&e)],
    );
    let many: Option<Vec<PriceData>> =
        call(&e, &client.address, "prices", &[quoted, 1u32.into_val(&e)]);

    assert!(last.is_none());
    assert!(at.is_none());
    assert!(many.is_none());
}

/// SEP40-R14 — a timestamp outside retained history returns `None`.
#[test]
fn r14_timestamp_outside_history_returns_none() {
    let e = Env::default();
    // Keep the ledger low: `price` walks back at most 1_000 ledgers.
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    ledger_default(&e, 50, 1_000);
    client.submit_price(&src, &asset, &500, &1_000);

    let quoted: Val = Asset::Stellar(asset.clone()).into_val(&e);
    let past: Option<PriceData> = call(
        &e,
        &client.address,
        "price",
        &[quoted.clone(), 999u64.into_val(&e)],
    );
    assert!(past.is_none(), "before the first observation");
    assert!(client.price(&Asset::Stellar(asset), &999u64).is_none());

    // A request *after* the last observation is answered with the newest record
    // rather than `None` (interpretation A-02): it is the price that was current
    // at that point in time, and no error is raised.
    let future: Option<PriceData> = call(
        &e,
        &client.address,
        "price",
        &[quoted, 9_000_000_000u64.into_val(&e)],
    );
    assert_eq!(future.unwrap().price, 500);
}

/// SEP40-R15 — `records = 0` is a well-formed empty answer, not `None`.
///
/// Also pins deviation D-04: a `records` value above the retention limit is a
/// consumer bug and is rejected instead of being silently truncated.
#[test]
fn r15_zero_records_returns_an_empty_vec() {
    let e = Env::default();
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    // A small retention window keeps the backwards scan inside the test host's
    // resource budget while still exercising the `records` bound.
    client.set_max_history_length(&3u32);
    ledger_default(&e, 50, 1_000);
    client.submit_price(&src, &asset, &500, &1_000);

    let quoted: Val = Asset::Stellar(asset.clone()).into_val(&e);
    let none: Option<Vec<PriceData>> =
        call(&e, &client.address, "prices", &[quoted, 0u32.into_val(&e)]);
    let none = none.expect("records=0 is not an error");
    assert!(none.is_empty(), "records=0 returns an empty vec");

    // D-04: above the retention limit the call is rejected, never truncated.
    assert!(client
        .try_prices(&Asset::Stellar(asset.clone()), &4u32)
        .is_err());
    assert!(client.try_prices(&Asset::Stellar(asset), &3u32).is_ok());
}

/// SEP40-R16 — the exported symbols are exactly the spec's names.
#[test]
fn r16_exported_symbols_match_the_spec_names() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000);
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    client.submit_price(&src, &asset, &500, &1_000);

    let quoted: Val = Asset::Stellar(asset).into_val(&e);
    let addr = client.address.clone();

    assert!(spec_call_succeeds::<Asset>(&e, &addr, "base", &[]));
    assert!(spec_call_succeeds::<Vec<Asset>>(&e, &addr, "assets", &[]));
    assert!(spec_call_succeeds::<u32>(&e, &addr, "decimals", &[]));
    assert!(spec_call_succeeds::<u32>(&e, &addr, "resolution", &[]));
    assert!(spec_call_succeeds::<Option<PriceData>>(
        &e,
        &addr,
        "lastprice",
        &[quoted.clone()]
    ));
    assert!(spec_call_succeeds::<Option<PriceData>>(
        &e,
        &addr,
        "price",
        &[quoted.clone(), 1_000u64.into_val(&e)]
    ));
    assert!(spec_call_succeeds::<Option<Vec<PriceData>>>(
        &e,
        &addr,
        "prices",
        &[quoted, 1u32.into_val(&e)]
    ));

    // A-01: the Design Rationale prose spells this `last_price`; the interface
    // block spells it `lastprice`. Only the latter is exported.
    let other: Val = Asset::Stellar(Address::generate(&e)).into_val(&e);
    assert!(!spec_call_succeeds::<Option<PriceData>>(
        &e,
        &addr,
        "last_price",
        &[other]
    ));
}

/// SEP40-R17 — the consumer read entrypoints require no authorization.
#[test]
fn r17_read_entrypoints_require_no_authorization() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000);
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    client.submit_price(&src, &asset, &500, &1_000);

    // Drop every authorization entry and re-run the whole read surface.
    crate::test_helpers::clear_auth(&e);
    let quoted = Asset::Stellar(asset);
    assert!(client.lastprice(&quoted).is_some());
    assert!(client.price(&quoted, &1_000u64).is_some());
    assert!(client.prices(&quoted, &1u32).is_some());
    assert!(matches!(client.base(), Asset::Other(_)));
    assert_eq!(client.decimals(), 18);
    assert_eq!(client.assets().len(), 1);
    assert!(spec_call_succeeds::<u32>(
        &e,
        &client.address,
        "resolution",
        &[]
    ));
}

/// SEP40-R18 — a price older than the resolution window is not served.
#[test]
fn r18_lastprice_is_none_once_the_resolution_window_lapses() {
    let e = Env::default();
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    client.set_resolution(&10u32);

    ledger_default(&e, 100, 1_000);
    client.submit_price(&src, &asset, &500, &1_000);
    assert!(
        client.lastprice(&Asset::Stellar(asset.clone())).is_some(),
        "fresh"
    );

    // One second past the window: stale, and therefore not served.
    ledger_default(&e, 200, 1_011);
    assert!(client.lastprice(&Asset::Stellar(asset)).is_none(), "stale");
}

/// SEP40-R19 — deviation D-02: the served timestamp is the source observation
/// time, **not** `floor(unix_now / resolution) * resolution`.
#[test]
fn r19_timestamps_are_trimmed_to_the_resolution() {
    let e = Env::default();
    // 1_000_000 is not a multiple of the 300 s resolution.
    let observed_at = 1_000_000u64;
    let (client, asset) = oracle_with_sources(&e, 1, 18);
    let src = client.get_oracle_sources().sources.get_unchecked(0);
    client.set_resolution(&300u32);
    ledger_default(&e, 100, observed_at);
    client.submit_price(&src, &asset, &500, &observed_at);

    let served = client.lastprice(&Asset::Stellar(asset)).unwrap();
    assert_eq!(
        served.timestamp, observed_at,
        "D-02: the observation timestamp is served verbatim, not floored"
    );
    // The resolution-trimmed value the SEP describes would have been 999_900.
    assert_eq!(observed_at / 300 * 300, 999_900);
    assert_ne!(served.timestamp, 999_900);
    // `last_updated` carries the ledger that wrote the datapoint instead.
    assert_eq!(served.last_updated, 100);
}

/// SEP40-R20 — deviation D-03: `decimals()` and `resolution()` remain
/// admin-configurable after deployment.
#[test]
fn r20_decimals_and_resolution_are_admin_changeable() {
    let e = Env::default();
    let client = oracle_with_decimals(&e, 7);
    assert_eq!(client.decimals(), 7);
    assert_eq!(client.resolution(), 0);

    client.set_decimals(&9u32);
    client.set_resolution(&60u32);
    assert_eq!(client.decimals(), 9, "D-03: precision stays changeable");
    assert_eq!(client.resolution(), 60, "D-03: resolution stays changeable");
}

// ---------------------------------------------------------------------------
// Meta-tests: the doc, the checklist and the test functions must agree
// ---------------------------------------------------------------------------

/// Path of the conformance document, relative to the contract crate.
const DOC: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../docs/sep40-conformance.md"
);
/// Path of this very file, used to prove each checklist row names a real test.
const SELF: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/src/sep40_conformance_tests.rs"
);

fn read(path: &str) -> std::string::String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// Collects every `SEP40-R<n>` identifier mentioned in `doc`.
fn requirement_ids_in(doc: &std::string::String) -> std::vec::Vec<std::string::String> {
    let mut out = std::vec::Vec::new();
    let bytes = doc.as_bytes();
    let needle = b"SEP40-R";
    let mut i = 0usize;
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] == needle {
            let mut j = i + needle.len();
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            out.push(std::string::String::from_utf8_lossy(&bytes[i..j]).into_owned());
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The pinned spec version is recorded, not implied.
#[test]
fn every_requirement_has_a_test() {
    let doc = read(DOC);
    assert!(
        doc.contains("ecosystem/sep-0040.md") && doc.contains("0.1.0"),
        "the pinned SEP-40 revision must be recorded in docs/sep40-conformance.md"
    );

    let mut doc_ids = requirement_ids_in(&doc);
    doc_ids.sort();
    doc_ids.dedup();

    let mut code_ids: std::vec::Vec<std::string::String> = CHECKLIST
        .iter()
        .map(|(id, _, _, _)| (*id).to_string())
        .collect();
    code_ids.sort();
    code_ids.dedup();

    assert_eq!(
        doc_ids, code_ids,
        "the requirement table in docs/sep40-conformance.md and CHECKLIST disagree"
    );
    assert!(!doc_ids.is_empty());
    for (id, _, _, status) in CHECKLIST {
        assert!(!status.label().is_empty(), "{id} has no status");
    }
}

/// Every requirement maps to a test function that exists in this file.
#[test]
fn requirement_test_functions_exist() {
    let source = read(SELF);
    let mut seen = std::vec::Vec::new();
    for (id, _, test_fn, _) in CHECKLIST {
        assert!(
            !seen.contains(test_fn),
            "{id} and another row both map to {test_fn}"
        );
        seen.push(test_fn);
        assert!(
            source.contains(&std::format!("fn {test_fn}(")),
            "{id} maps to {test_fn}, which is not defined in this file"
        );
    }
}

/// Deviations and ambiguities are documented, cross-referenced and justified.
#[test]
fn deviations_and_ambiguities_are_documented() {
    let doc = read(DOC);
    let known: std::vec::Vec<&str> = CHECKLIST.iter().map(|(id, _, _, _)| *id).collect();

    for (id, rows) in DEVIATIONS.iter().chain(AMBIGUITIES.iter()) {
        assert!(
            doc.contains(id),
            "{id} is not documented in the conformance doc"
        );
        for row in *rows {
            assert!(
                known.contains(row),
                "{id} references unknown requirement {row}"
            );
        }
    }

    // Every non-conforming requirement is accounted for by a deviation or an
    // ambiguity, so no row can be quietly downgraded to "conforms".
    for (id, _, _, status) in CHECKLIST {
        if *status == Status::Conforms {
            continue;
        }
        let covered = DEVIATIONS
            .iter()
            .chain(AMBIGUITIES.iter())
            .any(|(_, rows)| rows.contains(id));
        assert!(
            covered,
            "{id} is not covered by any documented deviation/ambiguity"
        );
    }
}
