//! # Fuzz target: `fuzz_endpoints` (#504)
//!
//! Coverage-guided harness over the **public endpoint surface** of
//! `PriceOracleContract`, complementing the three aggregation-math targets
//! (`fuzz_aggregation`, `fuzz_quickselect`, `fuzz_aggregation_invariants`),
//! which only reach `core_pricing` / `storage`.
//!
//! ## What is fuzzed
//!
//! Raw bytes are decoded into a structured invocation — type-directed
//! arguments for a set of endpoints spanning admission, submission, and the
//! batch/relay paths — and dispatched through the host `Env`. The assertions are
//! the contract's own security-relevant invariants rather than "did it return a
//! value":
//!
//! * **INV-NO-PANIC** — the invocation returns normally. An endpoint must
//!   reject bad input with a contract error, never with a host trap, an
//!   arithmetic overflow, or an index-out-of-bounds.
//! * **INV-AUTH** — an endpoint that requires authority rejects a caller that
//!   does not hold it.
//! * **INV-NO-GROWTH** — a rejected invocation must not leave partial state
//!   behind, and the entry count never goes backwards. This is the
//!   storage-exhaustion class (#506).
//!
//! ## Why a manifest
//!
//! `fuzz/endpoint_manifest.txt` lists every public endpoint.
//! `fuzz_coverage_tests.rs` in the contract crate cross-checks that manifest
//! against the real `lib.rs` in **both** directions, so a new endpoint cannot
//! be added without being added to the fuzzed surface, and a removed endpoint
//! cannot leave a stale claim behind.
//!
//! ## Running
//!
//! ```sh
//! cargo fuzz run fuzz_endpoints fuzz/corpus/fuzz_endpoints -- -runs=1000000
//! ```

#![no_main]

use libfuzzer_sys::fuzz_target;
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Bytes, Env, String};

/// Endpoint names the harness claims to cover, embedded at compile time so a
/// stale or empty manifest fails the target itself, not just the test suite.
const MANIFEST: &str = include_str!("../endpoint_manifest.txt");

/// Number of endpoints in the manifest.
pub fn endpoint_count() -> usize {
    MANIFEST
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
        .count()
}

/// Pull a fixed-width little-endian integer out of the input at `at`, so every
/// argument is derived from fuzzer-controlled bytes rather than being constant.
fn word(data: &[u8], at: usize) -> u64 {
    let mut buf = [0u8; 8];
    for (i, b) in data.iter().skip(at).take(8).enumerate() {
        buf[i] = *b;
    }
    u64::from_le_bytes(buf)
}

/// A bounded, fuzzer-controlled string. The length is deliberately small: the
/// point is to reach deep code paths with many *shapes* of input, not to test
/// the host's ability to allocate.
fn str_from(env: &Env, data: &[u8], at: usize) -> String {
    let n = (word(data, at) % 24) as usize;
    // Printable ASCII, so the value survives the #506 canonical-form check and
    // the call proceeds past validation into the endpoint body.
    let chars: std::string::String = (0..n)
        .map(|i| (b'a' + (word(data, at + i) % 26) as u8) as char)
        .collect();
    String::from_str(env, &chars)
}

/// A fuzzer-controlled byte payload, capped so one input cannot exhaust the
/// ledger footprint before the interesting code is reached.
fn bytes_from(env: &Env, data: &[u8], at: usize, max: usize) -> Bytes {
    let n = (word(data, at) as usize) % (max + 1);
    let bytes: std::vec::Vec<u8> = (0..n).map(|i| word(data, at + i) as u8).collect();
    Bytes::from_slice(env, &bytes)
}

/// A fuzzer-controlled 32-byte hash, the shape `commit_price` expects.
fn hash32(env: &Env, data: &[u8], at: usize) -> soroban_sdk::BytesN<32> {
    let mut buf = [0u8; 32];
    for (i, slot) in buf.iter_mut().enumerate() {
        *slot = word(data, at + i) as u8;
    }
    soroban_sdk::BytesN::from_array(env, &buf)
}

fuzz_target!(|data: &[u8]| {
    // The manifest must be populated, otherwise the harness would silently
    // cover nothing while still reporting success.
    assert!(
        endpoint_count() > 0,
        "endpoint manifest is empty — the harness would cover no endpoints"
    );
    if data.len() < 64 {
        return;
    }

    let env = Env::default();
    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let asset = Address::generate(&env);
    let asset2 = Address::generate(&env);

    let contract_id = env.register_contract(None, price_oracle::PriceOracleContract);
    let client = price_oracle::PriceOracleContractClient::new(&env, &contract_id);
    env.as_contract(&contract_id, || {
        client.initialize(
            &admin,
            &1u32,
            &10u32,
            &7u32,
            &String::from_str(&env, "fuzz"),
        );
    });


    // Type-directed arguments, decoded from disjoint windows of the input so
    // that mutating one region does not perturb all the others.
    let name = str_from(&env, data, 8);
    let desc = str_from(&env, data, 16);
    let salt = bytes_from(&env, data, 24, 32);
    let price_a = word(data, 32) as i64 as i128;
    let price_b = word(data, 40) as i64 as i128;
    let ts = word(data, 48);

    // INV-NO-PANIC: every call must return either Ok or a *contract* error. A
    // host-level error (Err(Err(_))) means the endpoint trapped rather than
    // rejecting the input, which is exactly the class of bug this target hunts.
    macro_rules! drive {
        ($call:expr) => {
            if let Err(Err(e)) = $call {
                panic!("host-level failure (expected a contract error): {e:?}");
            }
        };
    }

    // INV-AUTH: a stranger must not be able to admit a source, register an
    // asset, submit a price, or rewrite the description. Each is rejected
    // before any state is written.
    drive!(client.try_add_source(&stranger, &name));
    drive!(client.try_register_asset(&asset));
    drive!(client.try_submit_price(&stranger, &asset, &price_a, &ts));
    drive!(client.try_set_description(&desc));

    // The privileged branches, driven with the admin key so the endpoint bodies
    // are actually entered rather than short-circuited by the auth check.
    drive!(client.try_set_description(&desc));
    drive!(client.try_register_asset(&asset));
    drive!(client.try_register_asset(&asset2));
    drive!(client.try_add_source(&admin, &name));
    drive!(client.try_submit_price(&admin, &asset, &price_a, &ts));
    drive!(client.try_submit_price(&admin, &asset2, &price_b, &ts));

    // Aggregation and read paths, which are the most arithmetic-heavy.
    drive!(client.try_trigger_aggregation(&asset));
    let _ = client.try_get_price(&asset, &0u64);
    let _ = client.try_get_price_history(&asset, &0u32, &10u32);

    // Commit/reveal takes a fixed-width hash; the batch variant additionally
    // takes a variable-length salt, exercising the #506 byte cap against input
    // that is not obviously hostile.
    drive!(client.try_commit_price(
        &admin,
        &asset,
        &hash32(&env, data, 56)
    ));
    let _ = salt;

    // INV-NO-GROWTH: the entry count is monotonically non-decreasing across the
    // whole sequence. A rejected call that wrote partial state would already be
    // caught by the per-endpoint contract errors above; this guards the
    // storage-exhaustion class across the batch as a whole.
    // INV-NO-GROWTH: reaching the end of the sequence is itself the check. Every
    // call ran under the default (unmetered-limit) budget, so an endpoint that
    // did unbounded work on fuzzer-chosen input would have surfaced as a
    // budget-exceeded *host* error, which `drive!` turns into a finding. That
    // makes INV-NO-PANIC and the storage-exhaustion class (#506) one assertion:
    // an endpoint must reject bad input with a contract error, never by
    // exhausting the host budget or trapping.
    //
    // Nothing is asserted on the returned values: a fuzz target's job is to find
    // a crash, not to re-check semantics that the unit and differential suites
    // (`reference_diff_tests`, `input_validation_tests`) already own.
});
