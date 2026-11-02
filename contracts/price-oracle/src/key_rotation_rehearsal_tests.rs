//! #641: the staged source-key rotation that `scripts/testnet-key-rotation.sh`
//! rehearses on testnet, run natively: overlap, rollback, then retirement.
#![cfg(test)]

use crate::test_helpers::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};
#[test]
fn rotation_sequence_matches_script() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_700_000_000);
    let (c, _a) = setup_contract(&e);
    c.set_min_sources_required(&1u32);
    let old = register_test_source(&e, &c, "old");
    let peer = register_test_source(&e, &c, "peer");
    let asset = register_test_asset(&e, &c);
    let mut ts = 1_700_000_000u64;
    let mut step = |e: &Env| {
        ts += 1;
        e.ledger().with_mut(|l| {
            l.timestamp = ts;
            l.sequence_number += 1;
        });
        ts
    };
    let t = step(&e);
    c.submit_price(&old, &asset, &10_000_000, &t);
    c.submit_price(&peer, &asset, &10_000_000, &t);
    let new = Address::generate(&e);
    c.add_source(&new, &String::from_str(&e, "new"));
    let t = step(&e);
    c.submit_price(&old, &asset, &10_010_000, &t);
    c.submit_price(&new, &asset, &10_010_000, &t);
    c.remove_source(&new);
    assert!(!c.is_source(&new));
    let t = step(&e);
    c.submit_price(&old, &asset, &10_020_000, &t);
    assert!(c.try_submit_price(&new, &asset, &10_020_000, &t).is_err());
    c.add_source(&new, &String::from_str(&e, "new"));
    c.remove_source(&old);
    let t = step(&e);
    assert!(c.try_submit_price(&old, &asset, &10_030_000, &t).is_err());
    c.submit_price(&new, &asset, &10_030_000, &t);
}
