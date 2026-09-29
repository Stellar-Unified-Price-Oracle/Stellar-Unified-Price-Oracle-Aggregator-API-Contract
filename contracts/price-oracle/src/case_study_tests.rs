//! Executable snippets for `docs/case-studies.md` (#408).
//!
//! Each test is one exploit/countermeasure pair from a case study. The
//! document quotes these tests; CI runs them with the rest of the suite.
#![cfg(test)]

use crate::test_helpers::*;
use soroban_sdk::Env;
use soroban_sdk::{testutils::Ledger, String};

const TS: u64 = 1_700_000_000;

/// Lending: `max_age = 0` disables the staleness check, so a price from an
/// hour ago is returned as if current. Passing a bound rejects it.
#[test]
fn case_lending_stale_price_accepted_when_max_age_is_zero() {
    let e = Env::default();
    ledger_default(&e, 100, TS);
    let (client, _) = setup_contract(&e);
    let s1 = register_test_source(&e, &client, "A");
    let s2 = register_test_source(&e, &client, "B");
    let asset = register_test_asset(&e, &client);
    submit_test_price(&client, &s1, &asset, 100, TS);
    submit_test_price(&client, &s2, &asset, 100, TS);

    e.ledger().with_mut(|l| l.timestamp = TS + 3_600);

    // Naive: stale price comes back.
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 100);
    // Correct: bounded max_age → None.
    assert!(client.get_price(&asset, &60u64).is_none());
}

/// DEX: with a quorum of 3 and median aggregation, one manipulated source
/// cannot move the published price outside the honest range.
#[test]
fn case_dex_single_source_cannot_move_median() {
    let e = Env::default();
    ledger_default(&e, 100, TS);
    let (client, _) = setup_contract(&e);
    let a = register_test_source(&e, &client, "A");
    let b = register_test_source(&e, &client, "B");
    let m = register_test_source(&e, &client, "Manipulated");
    client.set_min_sources_required(&3u32);
    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &a, &asset, 100, TS);
    submit_test_price(&client, &m, &asset, 1_000_000, TS);
    assert!(
        client.get_price(&asset, &60u64).is_none(),
        "no quorum → no price"
    );
    submit_test_price(&client, &b, &asset, 102, TS);

    let p = client.get_price(&asset, &60u64).unwrap();
    assert!((100..=102).contains(&p.price));
    assert_eq!(p.num_sources, 3);
}

/// Payments: a frozen price is returned with `num_sources == 0` and its
/// original timestamp even when it is older than `max_age`. Integrators must
/// check `num_sources` / `timestamp` themselves or query `is_price_frozen`.
#[test]
fn case_payments_frozen_price_bypasses_max_age() {
    let e = Env::default();
    ledger_default(&e, 100, TS);
    let (client, _) = setup_contract(&e);
    let s1 = register_test_source(&e, &client, "A");
    let s2 = register_test_source(&e, &client, "B");
    let asset = register_test_asset(&e, &client);
    submit_test_price(&client, &s1, &asset, 100, TS);
    submit_test_price(&client, &s2, &asset, 100, TS);
    client.freeze_price(&asset, &String::from_str(&e, "incident"));

    e.ledger().with_mut(|l| l.timestamp = TS + 86_400);

    let p = client.get_price(&asset, &60u64).unwrap();
    assert_eq!(p.num_sources, 0);
    assert_eq!(p.timestamp, TS);
    assert!(client.is_price_frozen(&asset));
    assert_eq!(p.decimals, client.decimals());
}
