//! Source reputation gaming and reputation-laundering tests (#465).
//!
//! See `docs/security/reputation-gaming.md` for the cost model and findings.

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::reputation::{
    apply_reputation_decay, get_reputation, get_slash_threshold, update_reputation_on_submission,
    INITIAL_REPUTATION,
};
use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};
use crate::PriceOracleContractClient;

const MEDIAN: i128 = 1_000_000;
/// Highest score reachable under the default decay factor (see doc, finding R-1).
const FARM_CEILING: u32 = 81;

fn submit(e: &Env, c: &PriceOracleContractClient<'_>, src: &Address, price: i128) -> u32 {
    e.as_contract(&c.address, || {
        update_reputation_on_submission(e, src, price, MEDIAN);
        get_reputation(e, src.clone())
    })
}

fn decay(e: &Env, c: &PriceOracleContractClient<'_>, src: &Address) -> u32 {
    e.as_contract(&c.address, || {
        apply_reputation_decay(e, src);
        get_reputation(e, src.clone())
    })
}

/// Number of accurate submissions needed to farm from neutral (50) to `target`.
fn farm_cost(e: &Env, c: &PriceOracleContractClient<'_>, src: &Address, target: u32) -> u32 {
    let mut n = 0;
    while submit(e, c, src, MEDIAN) < target {
        n += 1;
        assert!(n < 1_000, "farming never converges");
    }
    n + 1
}

#[test]
fn farming_cost_is_quantified() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, _) = setup_contract(&e);
    let src = register_test_source(&e, &c, "farmer");
    // Integer EMA with decay 5: new = floor((old * 95 + 100 * 5) / 100). Flooring
    // makes 81 a fixed point, so perfect behaviour saturates there.
    let to_80 = farm_cost(&e, &c, &src, 80);
    assert_eq!(to_80, 24);
    for _ in 0..500 {
        submit(&e, &c, &src, MEDIAN);
    }
    assert_eq!(get_reputation_of(&e, &c, &src), FARM_CEILING);
    // Farmed trust is spent quickly: one outlier (accuracy 0) costs 5 points, and
    // reputation never gates the median itself (see doc), so the exploit gain is nil.
    let before = get_reputation_of(&e, &c, &src);
    let after = submit(&e, &c, &src, MEDIAN * 10);
    assert!(after < before && before - after <= 5);
}

fn get_reputation_of(e: &Env, c: &PriceOracleContractClient<'_>, src: &Address) -> u32 {
    e.as_contract(&c.address, || get_reputation(e, src.clone()))
}

#[test]
fn identity_rotation_cannot_shed_record() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, _) = setup_contract(&e);
    let bad = register_test_source(&e, &c, "tainted");
    for _ in 0..40 {
        submit(&e, &c, &bad, MEDIAN * 3);
    }
    let tainted = get_reputation_of(&e, &c, &bad);
    assert!(tainted < get_slash_threshold_of(&e, &c));

    // Remove and re-admit the same identity: the record is keyed by address and survives.
    c.remove_source(&bad);
    c.add_source(&bad, &String::from_str(&e, "tainted-again"));
    assert_eq!(get_reputation_of(&e, &c, &bad), tainted);
    assert_eq!(c.get_source_reputation(&bad), tainted as i128);

    // A fresh identity cannot self-admit: it must be admitted by the admin, so
    // rotation is an admin decision, never a unilateral laundering path.
    let fresh = Address::generate(&e);
    let asset = register_test_asset(&e, &c);
    assert!(c
        .try_submit_price(&fresh, &asset, &MEDIAN, &e.ledger().timestamp())
        .is_err());
}

fn get_slash_threshold_of(e: &Env, c: &PriceOracleContractClient<'_>) -> u32 {
    e.as_contract(&c.address, || get_slash_threshold(e))
}

#[test]
fn quiet_then_abuse_requires_sustained_performance() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, _) = setup_contract(&e);
    let src = register_test_source(&e, &c, "sleeper");
    for _ in 0..100 {
        submit(&e, &c, &src, MEDIAN);
    }
    let peak = get_reputation_of(&e, &c, &src);
    assert_eq!(peak, FARM_CEILING);

    // Going quiet: idle decay pulls the farmed score back towards neutral.
    let mut score = peak;
    for _ in 0..60 {
        score = decay(&e, &c, &src);
    }
    assert!(score < peak);
    // Decay moves 5% of the distance to neutral (floored), so it stops within 20 of 50.
    assert!(score < INITIAL_REPUTATION as u32 + 20, "score={score}");

    // Abuse in a volatile window: repeated outliers drive it below the slash threshold.
    let mut n = 0;
    while submit(&e, &c, &src, MEDIAN * 3) >= get_slash_threshold_of(&e, &c) {
        n += 1;
        assert!(n < 100);
    }
}

#[test]
fn third_party_cannot_suppress_honest_source() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, _) = setup_contract(&e);
    let honest = register_test_source(&e, &c, "honest");
    let attacker = register_test_source(&e, &c, "attacker");
    let start = get_reputation_of(&e, &c, &honest);
    for i in 0..50 {
        // Attacker spams correct-but-unhelpful and wild submissions.
        submit(
            &e,
            &c,
            &attacker,
            if i % 2 == 0 { MEDIAN } else { MEDIAN * 5 },
        );
        // Honest source keeps reporting within 1% of the median.
        let s = submit(&e, &c, &honest, MEDIAN + MEDIAN / 200);
        assert!(s >= start, "honest score dropped to {s}");
    }
    // The attacker's behaviour only moved its own record.
    assert!(get_reputation_of(&e, &c, &honest) > start);
}

#[test]
fn update_gap_is_bounded_to_one_step() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, _) = setup_contract(&e);
    let src = register_test_source(&e, &c, "gap");
    // Reputation is updated in the same call as the behaviour; the largest single-step
    // move (decay factor 5%) bounds what can be done in the gap.
    for _ in 0..200 {
        let before = get_reputation_of(&e, &c, &src);
        let after = submit(&e, &c, &src, MEDIAN * 10);
        assert!(before - after <= 5);
    }
}
