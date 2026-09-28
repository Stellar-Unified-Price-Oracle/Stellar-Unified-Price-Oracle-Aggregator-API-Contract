#![cfg(test)]

//! Test suite for #478 — on-chain derived price feeds.
//!
//! See `docs/derived-feeds.md`. Every acceptance criterion is covered here:
//! agreement with an off-chain reference, the documented rounding direction per
//! kind, distinct rejection codes, cycle-freedom, worst-case staleness
//! propagation, provenance, the event schema, and the config/auth guards.
//!
//! The reference computations below are written out longhand and deliberately do
//! **not** call the contract's helpers, so the assertions are a real check on
//! the implementation rather than a tautology.

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    xdr::{ContractEvent, ContractEventBody, ScVal},
    Address, Env,
};

use std::cmp::min;
use std::string::ToString;

use crate::derived_feeds;
use crate::test_helpers::*;
use crate::types::{DerivedFeed, DerivedFeedKind, ErrorCode as EC, MAX_DERIVATION_DEPTH};
use crate::PriceOracleContractClient;

/// Ledger timestamp every fixture starts at.
const T0: u64 = 1_000_000;
/// Contract-wide precision used by every fixture (`setup_contract` sets 18).
const DECIMALS: u32 = 18;
/// `10^18` — the scale factor for [`DECIMALS`].
const SCALE: i128 = 1_000_000_000_000_000_000;

// ---------------------------------------------------------------------------
// Independent reference computations (longhand, no contract helpers)
// ---------------------------------------------------------------------------

/// Off-chain reference for `1 / p`: the exact rational `scale^2 / p`, returned
/// as `(floor, remainder)` so a test can assert the truncation *and* that the
/// discarded fraction is strictly between 0 and the divisor.
///
/// Computed in `u128` throughout: at 18 decimals the scaled product `p * scale`
/// overflows `i128` for any price above ~170.0, which is an ordinary price, so
/// a reference that could not represent those inputs could not check them.
fn ref_inverse(p: i128) -> (u128, u128) {
    let num = (SCALE as u128) * (SCALE as u128);
    (num / (p as u128), num % (p as u128))
}

/// Off-chain reference for `base / quote`: `(floor, remainder)` of the exact
/// rational `p_base * scale / p_quote`.
fn ref_ratio(p_base: i128, p_quote: i128) -> (u128, u128) {
    let num = (p_base as u128) * (SCALE as u128);
    (num / (p_quote as u128), num % (p_quote as u128))
}

/// Off-chain reference for the triangulated cross rate, as an exact rational
/// `(numerator, denominator)` — deliberately *not* reduced, so it is visibly a
/// different route to the answer than the contract's two-step truncation.
fn ref_triangulation_rational(p_base: i128, _p_pivot: i128, p_quote: i128) -> (u128, u128) {
    // (p_base/p_pivot) * (p_pivot/p_quote) == p_base / p_quote, so the exact
    // cross rate scaled by `scale` is exactly p_base * scale / p_quote.
    ((p_base as u128) * (SCALE as u128), p_quote as u128)
}

/// `|derived - exact|` where `exact` is the rational `num / den`, computed with
/// integer cross-multiplication so the comparison never loses precision.
///
/// `derived` is in units of `10^-decimals`, so the error is reported in the
/// same unit: `0` means exact, `1` means "off by the last scaled digit".
///
/// The difference is taken via an explicit comparison because `derived * den`
/// can exceed `num`, and `u128` has no room to represent a negative value.
fn rational_error_units(derived: i128, num: u128, den: u128) -> u128 {
    let scaled = (derived as u128) * den;
    if scaled >= num {
        (scaled - num) / den
    } else {
        (num - scaled) / den
    }
}

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

/// Contract with no derived-feed state. `min_sources_required` is 1 so a single
/// submission is enough to publish an aggregate.
fn setup<'a>(e: &'a Env) -> PriceOracleContractClient<'a> {
    let (client, _admin) = setup_contract(e);
    client.set_min_sources_required(&1u32);
    client
}

/// Registers `asset`, registers a source, and publishes an aggregate for the
/// asset so `DataKey::Aggregate` is populated.
fn seed_aggregate(
    client: &PriceOracleContractClient<'_>,
    e: &Env,
    asset: &Address,
    price: i128,
    timestamp: u64,
) {
    let source = register_test_source(e, client, "Src");
    client.submit_price(&source, asset, &price, &timestamp);
}

/// Asserts a `try_*` call failed with exactly `Error(Contract, #code)`.
///
/// The generated `try_*` clients return nested results whose `Debug` rendering
/// contains the contract error, so this matches on that rendering.
fn expect_contract_err<T: core::fmt::Debug, A: core::fmt::Debug, B: core::fmt::Debug>(
    res: Result<Result<T, A>, B>,
    code: u32,
) {
    let rendered = format!("{:?}", res);
    assert!(
        rendered.contains(&format!("#{}", code)),
        "expected Error(Contract, #{}), got {}",
        code,
        rendered
    );
}

// ---------------------------------------------------------------------------
// Event helpers
// ---------------------------------------------------------------------------

fn oracle_events(e: &Env, client: &PriceOracleContractClient<'_>) -> std::vec::Vec<ContractEvent> {
    e.events()
        .all()
        .filter_by_contract(&client.address)
        .events()
        .to_vec()
}

fn topics(ev: &ContractEvent) -> std::vec::Vec<ScVal> {
    match &ev.body {
        ContractEventBody::V0(v0) => v0.topics.to_vec(),
    }
}

/// `true` if the event is the `#[contractevent]` named `name`.
fn is_event(ev: &ContractEvent, name: &str) -> bool {
    match topics(ev).first() {
        Some(ScVal::Symbol(sym)) => sym.0.to_string() == name,
        _ => false,
    }
}

fn field(ev: &ContractEvent, key: &str) -> Option<ScVal> {
    let ContractEventBody::V0(v0) = &ev.body else {
        return None;
    };
    let ScVal::Map(Some(map)) = &v0.data else {
        return None;
    };
    map.0
        .iter()
        .find(|entry| match &entry.key {
            ScVal::Symbol(sym) => sym.0.to_string() == key,
            _ => false,
        })
        .map(|entry| entry.val.clone())
}

fn as_u32(v: &ScVal) -> Option<u32> {
    match v {
        ScVal::U32(u) => Some(*u),
        _ => None,
    }
}

fn as_i128(v: &ScVal) -> Option<i128> {
    match v {
        ScVal::I128(parts) => Some(((parts.hi as i128) << 64) | parts.lo as i128),
        _ => None,
    }
}

fn as_u64(v: &ScVal) -> Option<u64> {
    match v {
        ScVal::U64(u) => Some(*u),
        _ => None,
    }
}

/// Finds the single `DerivedFeedComputedEvent` in `evs` and returns
/// `(kind, price, staleness_secs)`.
fn computed_event(evs: &[ContractEvent]) -> (u32, i128, u64) {
    let found: std::vec::Vec<&ContractEvent> = evs
        .iter()
        .filter(|ev| is_event(ev, "derived_feed_computed_event"))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "expected exactly one DerivedFeedComputedEvent, got {}",
        found.len()
    );
    let ev = found[0];
    let kind = as_u32(&field(ev, "kind").expect("kind")).expect("kind u32");
    let price = as_i128(&field(ev, "price").expect("price")).expect("price i128");
    let stale =
        as_u64(&field(ev, "staleness_secs").expect("staleness_secs")).expect("staleness u64");
    (kind, price, stale)
}

/// The single input of an inverse feed, unwrapped.
fn only_input(feed: &DerivedFeed) -> crate::types::DerivedFeedInput {
    assert_eq!(feed.inputs.len(), 1, "inverse has exactly one input");
    feed.inputs.get_unchecked(0)
}

// ===========================================================================
// 1. The derived feed equals an independent off-chain reference within one unit
// ===========================================================================

/// An inverse computed from a live aggregate matches the exact rational
/// `scale^2 / p` to the last scaled digit, for values that do *not* divide
/// evenly.
#[test]
fn test_inverse_matches_offchain_reference_within_one_unit() {
    // (price, exact floor, remainder) — every remainder is non-zero, so every
    // case really does truncate.
    let cases: [(i128, i128, i128); 4] = [
        (
            3 * SCALE,
            333_333_333_333_333_333,
            1_000_000_000_000_000_000,
        ),
        (
            777 * SCALE + 123_456_789,
            1_287_001_287_001_082,
            396_953_667_978_976_754_302,
        ),
        // 7.000000000000000007 — a price that divides the scale square only
        // after 35 digits of 142857.
        (7 * SCALE + 7, 142_857_142_857_142_857, 1),
        (1_000_000_007, 999_999_993_000_000_048_999_999_657, 2_401),
    ];

    for (price, expected_floor, expected_rem) in cases {
        let e = Env::default();
        ledger_default(&e, 1_000, T0);
        let client = setup(&e);
        let asset = register_test_asset(&e, &client);
        seed_aggregate(&client, &e, &asset, price, T0);

        let feed = client.get_inverse_feed(&asset);
        assert_eq!(feed.price, expected_floor, "price = {}", price);

        // Independent reference: the same exact rational, recomputed here.
        let (floor, rem) = ref_inverse(price);
        assert_eq!((floor as i128, rem as i128), (expected_floor, expected_rem));
        // ...and the error against the *unrounded* rational.
        let err =
            rational_error_units(feed.price, (SCALE as u128) * (SCALE as u128), price as u128);
        assert!(err <= 1, "error {} units for price {}", err, price);
    }
}

/// A ratio computed from live aggregates matches the exact rational
/// `p_base * scale / p_quote` within one unit.
#[test]
fn test_ratio_matches_offchain_reference_within_one_unit() {
    // (base, quote) triples chosen so `p_base * scale` is not a multiple of
    // `p_quote` — i.e. the division really truncates.
    let cases: [(i128, i128); 4] = [
        (25 * SCALE / 10, 33 * SCALE / 10),
        (
            1_234_567_891 * SCALE / 1_000_000_000,
            765_432_109 * SCALE / 1_000_000_000,
        ),
        (3 * SCALE, 7 * SCALE),
        (999_999_999_999_999_999, 1_000_000_000_000_000_003),
    ];

    for (p_base, p_quote) in cases {
        let e = Env::default();
        ledger_default(&e, 1_000, T0);
        let client = setup(&e);
        let base = register_test_asset(&e, &client);
        let quote = register_test_asset(&e, &client);
        let src = register_test_source(&e, &client, "Src");
        client.submit_price(&src, &base, &p_base, &T0);
        client.submit_price(&src, &quote, &p_quote, &T0);

        let feed = client.get_ratio_feed(&base, &quote);

        let (floor, rem) = ref_ratio(p_base, p_quote);
        assert_eq!(
            feed.price, floor as i128,
            "base={} quote={}",
            p_base, p_quote
        );
        assert!(
            rational_error_units(
                feed.price,
                (p_base as u128) * (SCALE as u128),
                p_quote as u128
            ) <= 1,
            "ratio outside one unit"
        );
        // Sanity: the fixture really is a truncating division.
        assert!(rem > 0, "fixture {}/{} divides evenly", p_base, p_quote);
    }
}

/// A triangulated cross rate matches the exact real-number cross rate
/// `p_base / p_quote` within one unit, including a case that truncates at *both*
/// steps.
#[test]
fn test_triangulation_matches_offchain_reference_within_one_unit() {
    // (base, pivot, quote). The pivot cancels algebraically, so the reference
    // is the plain cross rate — a genuinely different route to the number.
    //
    // Every fixture is kept inside the range where both intermediates fit
    // `u128`: `p_base * scale` and `(p_base*scale/p_pivot) * p_pivot`. At 18
    // decimals that caps the base price well below `u128::MAX / 10^18`; the
    // wide-price boundary is covered separately by
    // `test_derivations_survive_prices_above_the_i128_scaled_product_limit`.
    let cases: [(i128, i128, i128); 4] = [
        (3 * SCALE, 7 * SCALE + 1, 11 * SCALE + 3),
        (5 * SCALE + 13, 2 * SCALE + 7, 3 * SCALE + 11),
        (
            1_234_567_890_123_456_789,
            900_000_000_000_000_000,
            765_432_109_876_543_210,
        ),
        // Deliberately awkward ratios so both steps truncate.
        (250 * SCALE + 123_456_789, 111 * SCALE + 3, 900 * SCALE + 7),
    ];

    for (p_base, p_pivot, p_quote) in cases {
        let e = Env::default();
        ledger_default(&e, 1_000, T0);
        let client = setup(&e);
        let base = register_test_asset(&e, &client);
        let pivot = register_test_asset(&e, &client);
        let quote = register_test_asset(&e, &client);
        let src = register_test_source(&e, &client, "Src");
        client.submit_price(&src, &base, &p_base, &T0);
        client.submit_price(&src, &pivot, &p_pivot, &T0);
        client.submit_price(&src, &quote, &p_quote, &T0);

        let feed = client.get_triangulated_feed(&base, &pivot, &quote);

        let (num, den) = ref_triangulation_rational(p_base, p_pivot, p_quote);
        let err = rational_error_units(feed.price, num, den);
        assert!(
            err <= 1,
            "triangulation off by {} units (b={} p={} q={})",
            err,
            p_base,
            p_pivot,
            p_quote
        );
    }
}

/// Regression: every derivation kind must work for prices above
/// `i128::MAX / 10^decimals` — roughly `170.0` at 18 decimals.
///
/// The scaled intermediate `p * 10^decimals` overflows `i128` at those prices,
/// so a narrow product would reject them with `InvalidConfiguration` even
/// though the final quotient is an ordinary number. That is a realistic price
/// for most assets, so it must not be a configuration error.
///
/// The fixtures sit at `250.0` and `125.0`, which is inside the window where
/// the scaled product exceeds `i128::MAX` (~1.7e38) but still fits `u128`
/// (~3.4e38) — the window in which the widening actually matters. The reference
/// is therefore computed in `u128`, since `p_base * SCALE` is exactly the
/// quantity that no longer fits in `i128`.
#[test]
fn test_derivations_survive_prices_above_the_i128_scaled_product_limit() {
    // 250.0 and 125.0, with non-round fractional parts so the divisions
    // still truncate.
    let p_base: i128 = 250 * SCALE + 7;
    let p_quote: i128 = 125 * SCALE + 3;

    // The scaled product really does overflow a narrow `i128`.
    let scaled = (p_base as u128) * (SCALE as u128);
    assert!(
        scaled > i128::MAX as u128 && scaled <= u128::MAX,
        "fixture must overflow i128 yet fit u128"
    );

    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");
    client.submit_price(&src, &base, &p_base, &T0);
    client.submit_price(&src, &quote, &p_quote, &T0);

    // Inverse: the exact rational `scale^2 / p_base`, truncated.
    let inv_num = (SCALE as u128) * (SCALE as u128);
    let expected_inverse = (inv_num / (p_base as u128)) as i128;
    let inverse_feed = client.get_inverse_feed(&base);
    assert_eq!(
        inverse_feed.price, expected_inverse,
        "wide inverse must equal the exact rational, truncated"
    );

    // Ratio: the exact rational `p_base * scale / p_quote`, truncated.
    let expected_ratio = (scaled / (p_quote as u128)) as i128;
    let ratio_feed = client.get_ratio_feed(&base, &quote);
    assert_eq!(
        ratio_feed.price, expected_ratio,
        "wide ratio must equal the exact rational, truncated"
    );

    // Triangulation through a third wide-priced leg exercises the same wide
    // intermediate twice and must still land within one unit of the exact
    // cross rate `p_base / p_quote` (the pivot cancels algebraically).
    let pivot = register_test_asset(&e, &client);
    client.submit_price(&src, &pivot, &p_quote, &T0);
    let tri_feed = client.get_triangulated_feed(&base, &pivot, &quote);
    let err = rational_error_units(tri_feed.price, scaled, p_quote as u128);
    assert!(
        err <= 1,
        "wide triangulation off by {} units of 10^-{}",
        err,
        DECIMALS
    );
}

// ===========================================================================
// 2. Rounding direction is documented and asserted per derivation kind
// ===========================================================================

/// `Inverse` rounds **DOWN**: for a price that does not divide `scale^2`
/// exactly, the result is the floor and is strictly less than the exact
/// rational — it never rounds up, so a consumer of `1 / p` never over-pays.
#[test]
fn test_inverse_rounds_down_never_up() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let asset = register_test_asset(&e, &client);

    // 3.0 does not divide 10^36 exactly.
    let p = 3 * SCALE;
    seed_aggregate(&client, &e, &asset, p, T0);

    let feed = client.get_inverse_feed(&asset);
    let exact_floor = 333_333_333_333_333_333;
    assert_eq!(feed.price, exact_floor, "inverse truncates toward zero");

    // Strictly below the exact rational: floor(exact) * p < scale^2.
    assert!(
        feed.price * p < SCALE * SCALE,
        "inverse must round down, never up"
    );
    // And the ceiling would have been strictly larger.
    assert!((feed.price + 1) * p > SCALE * SCALE);
}

/// `Ratio` truncates toward zero: the result is the exact floor of
/// `p_base * scale / p_quote` and is strictly below the exact rational.
#[test]
fn test_ratio_truncates_toward_zero() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");

    let p_base = 25 * SCALE / 10; // 2.5
    let p_quote = 33 * SCALE / 10; // 3.3
    client.submit_price(&src, &base, &p_base, &T0);
    client.submit_price(&src, &quote, &p_quote, &T0);

    let feed = client.get_ratio_feed(&base, &quote);
    let exact_floor = 757_575_757_575_757_575;
    assert_eq!(feed.price, exact_floor, "ratio truncates toward zero");
    // floor(exact) * quote < base * scale, i.e. never rounded up.
    assert!(
        feed.price * p_quote < p_base * SCALE,
        "ratio must round down"
    );
    assert!((feed.price + 1) * p_quote > p_base * SCALE);
}

/// `Triangulation` truncates at **each** of its two steps, so the result is
/// never above the one-step exact cross rate.
#[test]
fn test_triangulation_truncates_at_each_step() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let pivot = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");

    // 3.0 / 7.000000000000000001 / 11.000000000000000003: neither step divides
    // evenly, so both truncate.
    let p_base = 3 * SCALE;
    let p_pivot = 7 * SCALE + 1;
    let p_quote = 11 * SCALE + 3;
    client.submit_price(&src, &base, &p_base, &T0);
    client.submit_price(&src, &pivot, &p_pivot, &T0);
    client.submit_price(&src, &quote, &p_quote, &T0);

    // The two-step result, spelled out here independently of the contract.
    let r1 = (p_base * SCALE) / p_pivot;
    let two_step = (r1 * p_pivot) / p_quote;
    assert_eq!(r1, 428_571_428_571_428_571);
    assert_eq!(two_step, 272_727_272_727_272_726);

    let feed = client.get_triangulated_feed(&base, &pivot, &quote);
    assert_eq!(feed.price, two_step, "two-step truncated result");

    // The one-step exact cross rate, and the guarantee the truncation buys.
    let one_step = (p_base * SCALE) / p_quote;
    assert!(
        feed.price <= one_step,
        "two-step truncation is never above the one-step exact result"
    );
    // Both steps really did truncate in this fixture.
    assert!(r1 * p_pivot < p_base * SCALE, "step 1 must truncate");
    assert!(two_step * p_quote < r1 * p_pivot, "step 2 must truncate");
}

// ===========================================================================
// 3. Zero denominators and unknown pairs get distinct error codes
// ===========================================================================

/// A zero **base** of an `Inverse` is refused with
/// [`EC::DerivedFeedZeroDenominator`] — never silently turned into a `0` or an
/// overflowing inverse.
#[test]
fn test_zero_inverse_base_is_zero_denominator() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let asset = register_test_asset(&e, &client);
    // A zero canonical base is accepted on write...
    client.set_derived_feed_base(&asset, &0i128, &T0);
    // ...and refused at derivation time.
    expect_contract_err(client.try_get_inverse_feed(&asset), 184);
}

/// A zero **quote** of a `Ratio` is refused with the same code.
#[test]
fn test_zero_ratio_quote_is_zero_denominator() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    client.set_derived_feed_base(&base, &(2 * SCALE), &T0);
    client.set_derived_feed_base(&quote, &0i128, &T0);

    expect_contract_err(client.try_get_ratio_feed(&base, &quote), 184);
}

/// A zero **pivot** of a `Triangulation` is refused with the same code.
#[test]
fn test_zero_triangulation_pivot_is_zero_denominator() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let pivot = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    client.set_derived_feed_base(&base, &(2 * SCALE), &T0);
    client.set_derived_feed_base(&pivot, &0i128, &T0);
    client.set_derived_feed_base(&quote, &(5 * SCALE), &T0);

    expect_contract_err(client.try_get_triangulated_feed(&base, &pivot, &quote), 184);
}

/// A registered asset with neither a canonical base nor an aggregate is
/// refused with [`EC::UnknownDerivedPair`].
#[test]
fn test_registered_but_priceless_asset_is_unknown_pair() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    client.set_derived_feed_base(&base, &(2 * SCALE), &T0);
    // `quote` is registered but has no price at all.
    expect_contract_err(client.try_get_ratio_feed(&base, &quote), 185);
    expect_contract_err(client.try_get_inverse_feed(&quote), 185);
}

/// The zero-denominator and unknown-pair rejections are genuinely *different*
/// codes, so a consumer can tell "market has no price" from "pair is not
/// tracked here".
#[test]
fn test_zero_denominator_and_unknown_pair_codes_are_distinct() {
    assert_eq!(EC::DerivedFeedZeroDenominator as u32, 184);
    assert_eq!(EC::UnknownDerivedPair as u32, 185);
    assert_ne!(
        EC::DerivedFeedZeroDenominator as u32,
        EC::UnknownDerivedPair as u32
    );

    // And the two paths really do produce those two distinct codes.
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let known = register_test_asset(&e, &client);
    let unknown = register_test_asset(&e, &client);
    client.set_derived_feed_base(&known, &0i128, &T0);
    client.set_derived_feed_base(&unknown, &SCALE, &T0);
    let priceless = register_test_asset(&e, &client);

    let zero = format!("{:?}", client.try_get_inverse_feed(&known));
    let missing = format!("{:?}", client.try_get_inverse_feed(&priceless));
    assert!(zero.contains("#184"), "got {}", zero);
    assert!(missing.contains("#185"), "got {}", missing);
}

/// An unregistered asset is a *different* failure again — the registry check
/// runs before any price lookup.
#[test]
fn test_unregistered_asset_is_asset_not_registered() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let stranger = Address::generate(&e);
    expect_contract_err(client.try_get_inverse_feed(&stranger), 2);
}

// ===========================================================================
// 4. No derivation cycle is possible
// ===========================================================================

/// A self-pair `a / a` is refused with [`EC::DerivedFeedCycle`] (#186).
///
/// A self-pair is reported as a **cycle**, not as an unknown pair: both legs
/// resolve to a perfectly well-known price, so what is degenerate is the
/// *graph*, not the price lookup.
#[test]
fn test_self_ratio_is_rejected_as_cycle() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let a = register_test_asset(&e, &client);
    client.set_derived_feed_base(&a, &(3 * SCALE), &T0);

    expect_contract_err(client.try_get_ratio_feed(&a, &a), 186);
    // The same request through the generic dispatcher agrees.
    expect_contract_err(
        client.try_compute_derived_feed(&DerivedFeedKind::Ratio, &a, &a, &None),
        186,
    );
}

/// Every degenerate triangulation — any two of base/pivot/quote equal — is
/// refused with [`EC::DerivedFeedCycle`].
#[test]
fn test_degenerate_triangulations_are_rejected_as_cycles() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let a = register_test_asset(&e, &client);
    let b = register_test_asset(&e, &client);
    client.set_derived_feed_base(&a, &(3 * SCALE), &T0);
    client.set_derived_feed_base(&b, &(5 * SCALE), &T0);

    // base == pivot
    expect_contract_err(client.try_get_triangulated_feed(&a, &a, &b), 186);
    // base == quote
    expect_contract_err(client.try_get_triangulated_feed(&a, &b, &a), 186);
    // pivot == quote
    expect_contract_err(client.try_get_triangulated_feed(&a, &b, &b), 186);
    // all three equal
    expect_contract_err(client.try_get_triangulated_feed(&a, &a, &a), 186);
}

/// The *structural* guarantee behind the cycle check: a derived feed can never
/// become an input to another derivation.
///
/// Input resolution reads only `DataKey::DerivedFeedBase` and
/// `DataKey::Aggregate`, and `compute_derived_feed` writes neither — so a
/// `DerivedFeed` value is unreachable from the resolution path no matter how
/// many derivations are chained. This test demonstrates it end-to-end: a
/// derived feed is computed, and a subsequent derivation over the *same*
/// assets reads the original base prices, unchanged.
#[test]
fn test_derived_feed_can_never_be_used_as_an_input() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let a = register_test_asset(&e, &client);
    let b = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");
    client.submit_price(&src, &a, &(2 * SCALE), &T0);
    client.submit_price(&src, &b, &(3 * SCALE), &T0);

    // First derivation: 2/3.
    let first = client.get_ratio_feed(&a, &b);
    let first_price = first.price;

    // Chain a second derivation over the same pair. If the first result had
    // been fed back in, the answer would differ (or the graph would recurse).
    let second = client.get_ratio_feed(&a, &b);
    assert_eq!(
        second.price, first_price,
        "a derived feed must not become an input to the next derivation"
    );

    // The input provenance still names the *base* prices, never the derived
    // price, which is the direct evidence that nothing was written back.
    let in_a = second.inputs.get_unchecked(0);
    let in_b = second.inputs.get_unchecked(1);
    assert_eq!(in_a.price, 2 * SCALE);
    assert_eq!(in_b.price, 3 * SCALE);
    assert_ne!(in_a.price, first_price);
}

/// The depth bound is explicit in code and reachable through the
/// `assert_depth` helper, so [`EC::DerivedFeedDepthExceeded`] (#187) is a real
/// error and not dead code. The live path calls `assert_depth(env, 1)`, which
/// is trivially within the bound.
///
/// `assert_depth` is reached through the *contract itself* — a derived feed is
/// computed for depth 1, so the guard is exercised on every read.
#[test]
fn test_depth_bound_is_one_and_enforced() {
    assert_eq!(MAX_DERIVATION_DEPTH, 1);

    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let asset = register_test_asset(&e, &client);
    client.set_derived_feed_base(&asset, &(3 * SCALE), &T0);

    // The live read path calls `assert_depth(env, 1)` and succeeds.
    let feed = client.get_inverse_feed(&asset);
    assert_eq!(feed.price, 333_333_333_333_333_333);

    // One past the bound is refused by the same helper. It is `pub` in the
    // module, so it can be driven directly without a contract entrypoint.
    let over = MAX_DERIVATION_DEPTH + 1;
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        derived_feeds::assert_depth(&e, over)
    }));
    assert!(result.is_err(), "depth {} must be refused", over);
    assert_eq!(EC::DerivedFeedDepthExceeded as u32, 187);
}

// ===========================================================================
// 5. Worst-case staleness
// ===========================================================================

/// A derived feed carries the **maximum** age across its inputs, and the
/// `oldest_timestamp` of the stalest one. Two inputs at deliberately different
/// timestamps: the answer must be the staler one, not the fresher one and not
/// the average.
#[test]
fn test_staleness_is_worst_case_across_inputs() {
    let e = Env::default();
    let now = T0 + 10_000;
    ledger_default(&e, 1_000, now);
    let client = setup(&e);
    let fresh = register_test_asset(&e, &client);
    let stale = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");

    let fresh_ts = now - 100;
    let stale_ts = now - 3_600;
    client.submit_price(&src, &fresh, &(2 * SCALE), &fresh_ts);
    client.submit_price(&src, &stale, &(3 * SCALE), &stale_ts);

    let feed = client.get_ratio_feed(&fresh, &stale);

    assert_eq!(
        feed.staleness_secs,
        now - stale_ts,
        "staleness must equal the stalest input's age"
    );
    assert_eq!(feed.staleness_secs, 3_600);
    assert_ne!(
        feed.staleness_secs,
        now - fresh_ts,
        "not the freshest input"
    );
    assert_eq!(feed.oldest_timestamp, stale_ts);
    assert_eq!(feed.oldest_timestamp, min(fresh_ts, stale_ts));
}

/// A triangulation propagates the stalest of its **three** inputs.
#[test]
fn test_triangulation_staleness_is_worst_case_of_three_inputs() {
    let e = Env::default();
    let now = T0 + 10_000;
    ledger_default(&e, 1_000, now);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let pivot = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");

    let base_ts = now - 10;
    let pivot_ts = now - 20;
    let quote_ts = now - 7_200; // the stalest leg
    client.submit_price(&src, &base, &(2 * SCALE), &base_ts);
    client.submit_price(&src, &pivot, &(3 * SCALE), &pivot_ts);
    client.submit_price(&src, &quote, &(5 * SCALE), &quote_ts);

    let feed = client.get_triangulated_feed(&base, &pivot, &quote);
    assert_eq!(feed.staleness_secs, now - quote_ts);
    assert_eq!(feed.staleness_secs, 7_200);
    assert_eq!(feed.oldest_timestamp, quote_ts);
}

/// The admin base overrides the live aggregate *and* carries its own timestamp
/// for staleness purposes.
#[test]
fn test_admin_base_overrides_aggregate_and_carries_its_timestamp() {
    let e = Env::default();
    let now = T0 + 10_000;
    ledger_default(&e, 1_000, now);
    let client = setup(&e);
    let pinned = register_test_asset(&e, &client);
    let other = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");

    // An aggregate exists and is much fresher than the pinned base...
    let agg_ts = now - 5;
    client.submit_price(&src, &pinned, &(4 * SCALE), &agg_ts);
    // ...but the admin base wins, and is far staler.
    let base_ts = now - 9_000;
    client.set_derived_feed_base(&pinned, &(9 * SCALE), &base_ts);
    client.set_derived_feed_base(&other, &(3 * SCALE), &agg_ts);

    let feed = client.get_ratio_feed(&pinned, &other);

    // The price comes from the base (9), not the aggregate (4).
    assert_eq!(feed.price, ref_ratio(9 * SCALE, 3 * SCALE).0 as i128);
    assert_ne!(feed.price, ref_ratio(4 * SCALE, 3 * SCALE).0 as i128);

    // The staleness comes from the base's timestamp, and `from_base` records it.
    assert_eq!(feed.staleness_secs, now - base_ts);
    assert_eq!(feed.oldest_timestamp, base_ts);
    assert!(feed.inputs.get_unchecked(0).from_base);
}

// ===========================================================================
// 6. Provenance
// ===========================================================================

/// `inputs` is populated in evaluation order — base, quote, pivot — with the
/// `from_base` flag set per resolution path.
#[test]
fn test_provenance_inputs_are_in_evaluation_order() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let base = register_test_asset(&e, &client);
    let pivot = register_test_asset(&e, &client);
    let quote = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");

    // `base` and `quote` resolve from live aggregates; `pivot` from an admin base.
    let base_ts = T0 - 30;
    let quote_ts = T0 - 20;
    let pivot_ts = T0 - 10;
    client.submit_price(&src, &base, &(2 * SCALE), &base_ts);
    client.submit_price(&src, &quote, &(5 * SCALE), &quote_ts);
    client.set_derived_feed_base(&pivot, &(3 * SCALE), &pivot_ts);

    let feed = client.get_triangulated_feed(&base, &pivot, &quote);

    assert_eq!(feed.inputs.len(), 3);
    let i0 = feed.inputs.get_unchecked(0);
    let i1 = feed.inputs.get_unchecked(1);
    let i2 = feed.inputs.get_unchecked(2);

    // Order: base, then quote, then pivot.
    assert_eq!(i0.asset, base);
    assert_eq!(i0.price, 2 * SCALE);
    assert_eq!(i0.timestamp, base_ts);
    assert!(!i0.from_base, "base came from the live aggregate");

    assert_eq!(i1.asset, quote);
    assert_eq!(i1.price, 5 * SCALE);
    assert_eq!(i1.timestamp, quote_ts);
    assert!(!i1.from_base, "quote came from the live aggregate");

    assert_eq!(i2.asset, pivot);
    assert_eq!(i2.price, 3 * SCALE);
    assert_eq!(i2.timestamp, pivot_ts);
    assert!(i2.from_base, "pivot came from the admin canonical base");
}

/// A ratio records exactly two inputs, and an inverse exactly one (the asset is
/// both numerator and denominator, so it is not listed twice).
#[test]
fn test_provenance_input_counts_per_kind() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let a = register_test_asset(&e, &client);
    let b = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");
    client.submit_price(&src, &a, &(2 * SCALE), &T0);
    client.submit_price(&src, &b, &(3 * SCALE), &T0);

    let ratio = client.get_ratio_feed(&a, &b);
    assert_eq!(ratio.inputs.len(), 2);

    let inverse = client.get_inverse_feed(&a);
    assert_eq!(inverse.inputs.len(), 1);
    let only = only_input(&inverse);
    assert_eq!(only.asset, a);
    assert_eq!(only.price, 2 * SCALE);
    // For an inverse the `quote` field is the asset itself.
    assert_eq!(inverse.quote, a);
    assert_eq!(inverse.base, a);
}

// ===========================================================================
// 7. Events
// ===========================================================================

/// `DerivedFeedComputedEvent` is emitted on every computation, carrying the
/// kind discriminant, the price and the worst-case staleness.
///
/// The three legs are seeded at three different ages so the event's staleness
/// is a real worst-case computation: `a` is 50 s old, `b` is 500 s old and
/// `c` is 5 s old. Only the kinds that actually consume `b` report 500.
#[test]
fn test_computed_event_is_emitted_per_kind() {
    // (kind, expected worst-case staleness in seconds)
    let cases = [
        (DerivedFeedKind::Inverse, 50u64),
        (DerivedFeedKind::Ratio, 500u64),
        (DerivedFeedKind::Triangulation, 500u64),
    ];

    for (kind, expected_stale) in cases {
        let e = Env::default();
        let now = T0 + 1_000;
        ledger_default(&e, 1_000, now);
        let client = setup(&e);
        let a = register_test_asset(&e, &client);
        let b = register_test_asset(&e, &client);
        let c = register_test_asset(&e, &client);
        let src = register_test_source(&e, &client, "Src");
        client.submit_price(&src, &a, &(2 * SCALE), &(now - 50));
        client.submit_price(&src, &b, &(3 * SCALE), &(now - 500));
        client.submit_price(&src, &c, &(5 * SCALE), &(now - 5));

        // A pivot is only legal for a triangulation.
        let (feed, evs) = match kind {
            DerivedFeedKind::Triangulation => {
                let f = client.compute_derived_feed(&kind, &a, &b, &Some(c.clone()));
                let v = oracle_events(&e, &client);
                (f, v)
            }
            DerivedFeedKind::Inverse => {
                let f = client.compute_derived_feed(&kind, &a, &a, &None);
                let v = oracle_events(&e, &client);
                (f, v)
            }
            DerivedFeedKind::Ratio => {
                let f = client.compute_derived_feed(&kind, &a, &b, &None);
                let v = oracle_events(&e, &client);
                (f, v)
            }
        };

        let (ev_kind, ev_price, ev_stale) = computed_event(&evs);
        assert_eq!(ev_kind, kind.as_u32(), "kind discriminant");
        assert_eq!(ev_price, feed.price, "event price matches the feed");
        assert_eq!(ev_stale, feed.staleness_secs, "event staleness matches");
        assert_eq!(
            ev_stale, expected_stale,
            "worst-case staleness for {:?}",
            kind
        );
    }
}

/// Each getter emits exactly one event, with its own kind discriminant.
#[test]
fn test_each_getter_emits_exactly_one_event() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let a = register_test_asset(&e, &client);
    let b = register_test_asset(&e, &client);
    let c = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");
    client.submit_price(&src, &a, &(2 * SCALE), &T0);
    client.submit_price(&src, &b, &(3 * SCALE), &T0);
    client.submit_price(&src, &c, &(5 * SCALE), &T0);

    client.get_inverse_feed(&a);
    assert_eq!(computed_event(&oracle_events(&e, &client)).0, 0);

    client.get_ratio_feed(&a, &b);
    assert_eq!(computed_event(&oracle_events(&e, &client)).0, 1);

    client.get_triangulated_feed(&a, &c, &b);
    assert_eq!(computed_event(&oracle_events(&e, &client)).0, 2);
}

// ===========================================================================
// 8. Config and authorization guards on `set_derived_feed_base`
// ===========================================================================

/// Only the admin may pin a canonical base.
#[test]
#[should_panic]
fn test_set_derived_feed_base_requires_admin() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    clear_auth(&e);
    client.set_derived_feed_base(&asset, &(3 * SCALE), &T0);
}

/// An unregistered asset cannot be pinned.
#[test]
fn test_set_derived_feed_base_rejects_unregistered_asset() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let (client, _admin) = setup_contract(&e);
    let stranger = Address::generate(&e);

    expect_contract_err(
        client.try_set_derived_feed_base(&stranger, &(3 * SCALE), &T0),
        2,
    );
}

/// A timestamp beyond the configured `CfgTimestampThreshold` is refused.
#[test]
fn test_set_derived_feed_base_rejects_future_timestamp() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    // The default threshold is 300 s.
    expect_contract_err(
        client.try_set_derived_feed_base(&asset, &(3 * SCALE), &(T0 + 301)),
        9,
    );

    // Just inside the window is fine.
    client.set_derived_feed_base(&asset, &(3 * SCALE), &(T0 + 300));
}

/// A zero canonical base is accepted on write — the "no valid price" marker —
/// and a negative one is refused.
#[test]
fn test_set_derived_feed_base_accepts_zero_and_rejects_negative() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    client.set_derived_feed_base(&asset, &0i128, &T0);

    expect_contract_err(client.try_set_derived_feed_base(&asset, &(-1i128), &T0), 7);
}

// ===========================================================================
// 9. Dispatcher configuration and unknown discriminants
// ===========================================================================

/// A `pivot` supplied for a non-triangulation kind, or omitted for a
/// triangulation, is a configuration error.
#[test]
fn test_pivot_argument_must_match_the_kind() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let a = register_test_asset(&e, &client);
    let b = register_test_asset(&e, &client);
    let c = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");
    client.submit_price(&src, &a, &(2 * SCALE), &T0);
    client.submit_price(&src, &b, &(3 * SCALE), &T0);
    client.submit_price(&src, &c, &(5 * SCALE), &T0);

    let some = Some(c.clone());

    // Pivot supplied where it is not read.
    expect_contract_err(
        client.try_compute_derived_feed(&DerivedFeedKind::Inverse, &a, &a, &some),
        10,
    );
    expect_contract_err(
        client.try_compute_derived_feed(&DerivedFeedKind::Ratio, &a, &b, &some),
        10,
    );

    // Pivot omitted where it is required.
    expect_contract_err(
        client.try_compute_derived_feed(&DerivedFeedKind::Triangulation, &a, &b, &None),
        10,
    );
}

/// The `DerivedFeedKind` discriminant space is closed: `from_u32` rejects
/// anything outside `0..=2`, so an unknown on-chain kind can never be
/// dispatched.
#[test]
fn test_unknown_derived_feed_kind_discriminant_is_rejected() {
    assert_eq!(DerivedFeedKind::from_u32(0), Some(DerivedFeedKind::Inverse));
    assert_eq!(DerivedFeedKind::from_u32(1), Some(DerivedFeedKind::Ratio));
    assert_eq!(
        DerivedFeedKind::from_u32(2),
        Some(DerivedFeedKind::Triangulation)
    );

    // Round-trip: every kind's on-chain discriminant parses back to itself.
    for k in [
        DerivedFeedKind::Inverse,
        DerivedFeedKind::Ratio,
        DerivedFeedKind::Triangulation,
    ] {
        assert_eq!(DerivedFeedKind::from_u32(k.as_u32()), Some(k));
    }

    // Anything else is `None` — there is no way to smuggle an unknown kind in.
    for bogus in [3u32, 4, 8, 9, 10, u32::MAX] {
        assert_eq!(
            DerivedFeedKind::from_u32(bogus),
            None,
            "discriminant {} must be rejected",
            bogus
        );
    }
    assert_eq!(DerivedFeedKind::from_u32(9), None);
}

/// `scale_for` refuses a precision wider than the arithmetic can carry.
#[test]
fn test_scale_for_rejects_excessive_decimals() {
    let e = Env::default();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        e.as_contract(&Address::generate(&e), || derived_feeds::scale_for(&e, 19))
    }));
    assert!(result.is_err(), "decimals > 18 must be refused");
    assert_eq!(EC::InvalidConfiguration as u32, 10);
}

/// End-to-end: the generic dispatcher and the dedicated getter agree, and the
/// feed reports the contract's precision and kind.
#[test]
fn test_dispatcher_agrees_with_getter_and_reports_decimals() {
    let e = Env::default();
    ledger_default(&e, 1_000, T0);
    let client = setup(&e);
    let a = register_test_asset(&e, &client);
    let b = register_test_asset(&e, &client);
    let c = register_test_asset(&e, &client);
    let src = register_test_source(&e, &client, "Src");
    client.submit_price(&src, &a, &(2 * SCALE), &T0);
    client.submit_price(&src, &b, &(3 * SCALE), &T0);
    client.submit_price(&src, &c, &(5 * SCALE), &T0);

    let via_dispatch =
        client.compute_derived_feed(&DerivedFeedKind::Triangulation, &a, &b, &Some(c.clone()));
    let via_getter = client.get_triangulated_feed(&a, &c, &b);

    assert_eq!(via_dispatch.price, via_getter.price);
    assert_eq!(via_dispatch.kind, DerivedFeedKind::Triangulation);
    assert_eq!(via_dispatch.decimals, DECIMALS);
    assert_eq!(via_getter.decimals, DECIMALS);
}
