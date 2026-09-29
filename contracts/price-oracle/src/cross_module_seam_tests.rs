//! Cross-module seam regression suite (#411).
//!
//! Each test targets the boundary between two modules and is named
//! `seam_<module_a>_<module_b>__<invariant>`. The invariant catalogue, and
//! the mutant that each test kills, live in `docs/seam-invariants.md`.
#![cfg(test)]
#![allow(non_snake_case)]

use crate::test_helpers::*;
use crate::PriceOracleContractClient;
use soroban_sdk::{Address, Env, String};

const TS: u64 = 1_234_567_890;

fn setup<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address, Address, Address) {
    ledger_default(e, 100, TS);
    let (client, _) = setup_contract(e);
    let s1 = register_test_source(e, &client, "A");
    let s2 = register_test_source(e, &client, "B");
    client.set_min_sources_required(&2u32);
    let asset = register_test_asset(e, &client);
    (client, s1, s2, asset)
}

fn price(client: &PriceOracleContractClient<'_>, asset: &Address) -> Option<i128> {
    client.get_price(asset, &0u64).map(|p| p.price)
}

/// A pause landing between the two halves of a quorum must reject the second
/// half without recording it, so no aggregate is produced from half the set.
#[test]
fn seam_pause_submission__rejected_half_leaves_no_partial_state() {
    let e = Env::default();
    let (client, s1, s2, asset) = setup(&e);

    submit_test_price(&client, &s1, &asset, 100, TS);
    client.pause();
    assert!(client.try_submit_price(&s2, &asset, &999, &TS).is_err());
    assert_eq!(
        price(&client, &asset),
        None,
        "rejected submission must not complete quorum"
    );

    client.unpause();
    submit_test_price(&client, &s2, &asset, 110, TS);
    assert_eq!(
        price(&client, &asset),
        Some(105),
        "rejected 999 must never be counted"
    );
}

/// Reversed ordering: pausing before either half, then unpausing, must yield
/// exactly the aggregate of the post-unpause submissions.
#[test]
fn seam_pause_submission__reversed_order_only_counts_accepted() {
    let e = Env::default();
    let (client, s1, s2, asset) = setup(&e);

    client.pause();
    assert!(client.try_submit_price(&s1, &asset, &1, &TS).is_err());
    assert!(client.try_submit_price(&s2, &asset, &1, &TS).is_err());
    client.unpause();
    submit_test_price(&client, &s1, &asset, 200, TS);
    submit_test_price(&client, &s2, &asset, 300, TS);
    assert_eq!(price(&client, &asset), Some(250));
}

/// Lifting the global pause must not implicitly lift an asset-level pause.
#[test]
fn seam_global_pause_asset_pause__unpause_does_not_clear_asset_pause() {
    let e = Env::default();
    let (client, s1, _, asset) = setup(&e);

    client.pause_asset(&asset);
    client.pause();
    client.unpause();
    assert!(client.is_asset_paused(&asset));
    assert!(client.try_submit_price(&s1, &asset, &100, &TS).is_err());
}

/// Lifting the global pause must not implicitly unfreeze a frozen price.
#[test]
fn seam_pause_freeze__unpause_does_not_unfreeze() {
    let e = Env::default();
    let (client, s1, s2, asset) = setup(&e);

    submit_test_price(&client, &s1, &asset, 100, TS);
    submit_test_price(&client, &s2, &asset, 110, TS);
    client.freeze_price(&asset, &String::from_str(&e, "incident"));
    client.pause();
    client.unpause();
    assert!(client.is_price_frozen(&asset));

    let _ = client.try_submit_price(&s1, &asset, &10_000, &TS);
    let _ = client.try_submit_price(&s2, &asset, &10_000, &TS);
    assert_eq!(
        price(&client, &asset),
        Some(105),
        "frozen aggregate moved after unpause"
    );
}

/// Interleaving: a source is removed between two rounds. Its stale input must
/// not keep contributing to aggregates computed after removal.
#[test]
fn seam_sources_aggregation__removed_source_has_no_influence() {
    let e = Env::default();
    let (client, s1, s2, asset) = setup(&e);
    let s3 = register_test_source(&e, &client, "C");

    submit_test_price(&client, &s3, &asset, 1_000_000, TS);
    client.remove_source(&s3);
    assert!(client
        .try_submit_price(&s3, &asset, &1_000_000, &TS)
        .is_err());

    submit_test_price(&client, &s1, &asset, 100, TS);
    submit_test_price(&client, &s2, &asset, 110, TS);
    assert_eq!(
        price(&client, &asset),
        Some(105),
        "removed source still influences median"
    );
}

/// A rejected configuration change must not leave the quorum partially
/// lowered: min_sources stays enforced for the next submission.
#[test]
fn seam_config_aggregation__quorum_holds_after_rejected_submission() {
    let e = Env::default();
    let (client, s1, _, asset) = setup(&e);

    assert!(client.try_submit_price(&s1, &asset, &-5, &TS).is_err());
    submit_test_price(&client, &s1, &asset, 100, TS);
    assert_eq!(
        price(&client, &asset),
        None,
        "single source must not meet quorum of 2"
    );
}
