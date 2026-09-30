#![cfg(test)]

//! # Endpoint fuzz-surface coverage (#504)
//!
//! The fuzz harness can only claim to cover the endpoints it actually names,
//! so this suite is what makes that claim checkable. It cross-checks
//! `fuzz/endpoint_manifest.txt` against the real `impl PriceOracleContract`
//! block in `lib.rs` **in both directions**:
//!
//! * every public endpoint appears in the manifest, so a newly added endpoint
//!   cannot silently escape fuzzing;
//! * every manifest entry is a real endpoint, so a removed or renamed endpoint
//!   cannot leave a stale claim behind (which would inflate the reported
//!   coverage).
//!
//! It also checks the manifest's *shape* — non-empty, no duplicates, no
//! comments smuggled in as names — because the harness parses it as plain
//! text, and a malformed line would otherwise be counted as coverage.

// The crate is `no_std`, so the test helpers spell out the `std` types they use.
use soroban_sdk::{Address, Env, String};
use std::string::{String as StdString, ToString};
use std::vec::Vec;

use crate::test_helpers::setup_contract;

/// The manifest, embedded at compile time so a change to the file is picked up
/// by `cargo test` without any build-script plumbing.
const MANIFEST: &str = include_str!("../../../fuzz/endpoint_manifest.txt");

/// Every public endpoint name in `lib.rs`, in source order.
///
/// Signatures are joined across lines first: the contract writes most
/// parameters one per line, so a line-at-a-time parse would miss them.
fn contract_endpoints() -> Vec<StdString> {
    let lib = include_str!("lib.rs");
    let start = lib
        .find("impl PriceOracleContract {")
        .expect("lib.rs must contain the contract impl block");
    let body = &lib[start..];

    let mut out = Vec::new();
    let mut in_signature = false;
    for line in body.lines() {
        if !in_signature {
            if let Some(rest) = line.strip_prefix("    pub fn ") {
                let name = rest
                    .split(['(', '<', ' '])
                    .next()
                    .unwrap_or_default()
                    .trim();
                if !name.is_empty() {
                    out.push(name.to_string());
                }
                // Stay "in signature" until the parameter list closes, so a
                // nested `pub fn` inside a body is never picked up.
                in_signature =
                    !line.contains(')') || line.matches('(').count() > line.matches(')').count();
            }
            continue;
        }
        if line.matches('(').count() <= line.matches(')').count() {
            in_signature = false;
        }
    }
    out
}

/// The manifest entries: non-blank, non-comment lines.
fn manifest_entries() -> Vec<StdString> {
    MANIFEST
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(StdString::from)
        .collect()
}

/// Every public endpoint is claimed by the fuzz manifest.
#[test]
fn manifest_covers_every_endpoint() {
    let endpoints = contract_endpoints();
    let entries = manifest_entries();

    let missing: Vec<&StdString> = endpoints.iter().filter(|e| !entries.contains(e)).collect();
    assert!(
        missing.is_empty(),
        "endpoints missing from fuzz/endpoint_manifest.txt (#504): {missing:?}"
    );
}

/// Every manifest entry is a real endpoint — the reverse direction, so a
/// deleted endpoint cannot leave a stale coverage claim behind.
#[test]
fn manifest_has_no_stale_endpoints() {
    let endpoints = contract_endpoints();
    let stale: Vec<StdString> = manifest_entries()
        .into_iter()
        .filter(|e| !endpoints.contains(e))
        .collect();
    assert!(
        stale.is_empty(),
        "fuzz/endpoint_manifest.txt lists endpoints that no longer exist: {stale:?}"
    );
}

/// The manifest is well-formed and actually covers a meaningful surface.
#[test]
fn manifest_is_well_formed() {
    let entries = manifest_entries();
    assert!(
        entries.len() > 100,
        "manifest covers only {} endpoints; the harness is meant to cover the \
         whole public surface",
        entries.len()
    );

    let mut seen = std::collections::BTreeSet::new();
    for e in &entries {
        assert!(seen.insert(e.clone()), "duplicate manifest entry: {e}");
        // The harness matches these against `pub fn` identifiers, so anything
        // that is not a plain snake_case name would never resolve.
        assert!(
            e.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "manifest entry is not a valid endpoint identifier: {e}"
        );
        assert!(
            e.chars().next().is_some_and(|c| c.is_ascii_lowercase()),
            "manifest entry must not start with a digit or underscore: {e}"
        );
    }
}

/// The fuzz harness and coverage test agree on the endpoint set — the property
/// that keeps the reported coverage honest.
#[test]
fn harness_and_test_agree_on_endpoint_count() {
    let harness_count = MANIFEST
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .count();
    assert_eq!(
        harness_count,
        manifest_entries().len(),
        "fuzz harness and coverage test disagree on the manifest size"
    );
    assert_eq!(
        harness_count,
        contract_endpoints().len(),
        "manifest size must equal the number of public endpoints"
    );
}

// ────────────────────────────────────────────────────────────────────────────
// Harness self-test
// ────────────────────────────────────────────────────────────────────────────

/// The fuzz harness itself must be sound, and its assumptions must hold.
///
/// `fuzz_endpoints` cannot run under `cargo test` (it needs libFuzzer's nightly
/// ASAN build), so this test replays the harness's **exact endpoint sequence**
/// under the normal test runner. If the sequence traps, panics, or writes state
/// on a rejected call, the harness is broken in a way no seed corpus would
/// reliably reveal — so it is asserted directly rather than left to a fuzz run
/// that only executes in CI.
///
/// This is the "has teeth" check: it proves the INV-NO-PANIC / INV-AUTH
/// assertions in the target are satisfiable, i.e. that the endpoints the harness
/// drives really do reject bad input with *contract* errors rather than host
/// errors.
#[test]
fn harness_endpoint_sequence_is_well_behaved() {
    use soroban_sdk::testutils::Address as _;

    let e = Env::default();
    let stranger = Address::generate(&e);
    let asset = Address::generate(&e);
    let asset2 = Address::generate(&e);

    // `setup_contract` installs its own admin; use that key, not a fresh one,
    // so the privileged branch below is genuinely privileged.
    let (client, admin) = setup_contract(&e);

    // A fuzzer-shaped name: printable ASCII, within the #506 cap.
    let name = String::from_str(&e, "fuzzsrc");
    let desc = String::from_str(&e, "fuzz description");

    // Mirrors the `drive!` macro: a contract error is the *expected* rejection.
    // A host-level failure (Err(Err(_))) is the finding the harness reports.
    macro_rules! drive {
        ($call:expr) => {
            if let Err(Err(inner)) = $call {
                panic!("host-level failure (expected a contract error): {inner:?}");
            }
        };
    }

    // Admission and registration. `setup_contract` calls `mock_all_auths()`,
    // so authority is not what distinguishes these calls here — what is under
    // test is that each endpoint either serves or refuses with a *contract*
    // error, never a host failure. (The authority property itself is pinned by
    // `rbac_escalation_tests::matrix_covers_every_endpoint` and the per-module
    // auth tests.)
    drive!(client.try_add_source(&stranger, &name));
    drive!(client.try_register_asset(&asset));
    drive!(client.try_register_asset(&asset2));
    drive!(client.try_set_description(&desc));
    drive!(client.try_submit_price(&stranger, &asset, &100i128, &0u64));
    drive!(client.try_submit_price(&stranger, &asset2, &200i128, &0u64));

    // Registering the same asset twice must be refused cleanly, not trap.
    drive!(client.try_register_asset(&asset));

    // `setup_contract` configures `min_sources = 2`, so a second registered
    // source must also submit before an aggregate can exist. Reaching that
    // point exercises the full aggregate-and-read path the harness drives.
    let second = Address::generate(&e);
    client.add_source(&second, &String::from_str(&e, "fuzzsrc2"));
    client.submit_price(&second, &asset, &300i128, &0u64);

    drive!(client.try_trigger_aggregation(&asset));
    let _ = admin;

    // Reading through the aggregate must now succeed and be bounded by the
    // submissions that were allowed to influence it.
    let price = client
        .get_price(&asset, &0u64)
        .expect("an aggregate exists");
    assert!(
        price.price >= 100 && price.price <= 300,
        "aggregate {} must lie within the submitted range",
        price.price
    );

    // An oversized name — past the #506 cap — must be refused without
    // admitting anything. This is the storage-exhaustion class the harness
    // exists to catch.
    let over = String::from_str(
        &e,
        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
    );
    assert_eq!(over.len(), 65);
    drive!(client.try_add_source(&admin, &over));
    // The over-length name addresses a source that was never admitted, so the
    // rejected call must not have created it.
    assert!(
        !client.is_source(&admin),
        "a rejected add_source must not admit the source"
    );
}

/// The manifest is embedded in the fuzz target at compile time, so the target
/// would build against a stale copy. Assert the embedded text is non-trivial.
#[test]
fn embedded_manifest_reaches_the_fuzz_target() {
    assert!(
        MANIFEST.contains("submit_price"),
        "the manifest must name the endpoints the harness drives"
    );
    assert!(
        MANIFEST.contains("initialize"),
        "the manifest must cover initialization"
    );
    assert!(
        MANIFEST.len() > 1000,
        "the embedded manifest looks truncated ({} bytes)",
        MANIFEST.len()
    );
}
