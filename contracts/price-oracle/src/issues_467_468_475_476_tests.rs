#![cfg(test)]
//! Tests for #467 (RBAC / auth escalation), #468 (signed-submission
//! forgery & replay), #475 (source influence caps) and #476 (confidence bands).

extern crate std;

use ed25519_dalek::{Signer, SigningKey};
use proptest::prelude::*;
use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger, MockAuth, MockAuthInvoke},
    Address, BytesN, Env, IntoVal, String, Symbol, TryFromVal, Vec,
};
use std::collections::BTreeSet;
use std::string::ToString;

use crate::confidence_band::{is_low_confidence, quartiles, MIN_BAND_SOURCES};
use crate::influence_cap::{apply_cap, within_cap, BPS, DEFAULT_CAP_BPS};
use crate::test_helpers::*;
use crate::types::{PolicyOverride, Role};
use crate::{PriceOracleContract, PriceOracleContractClient};

fn median(v: &[i128]) -> i128 {
    let mut s = v.to_vec();
    s.sort_unstable();
    let n = s.len();
    if n % 2 == 1 {
        s[n / 2]
    } else {
        s[n / 2 - 1] + (s[n / 2] - s[n / 2 - 1]) / 2
    }
}

fn has_event(e: &Env, name: &str) -> bool {
    let want = Symbol::new(e, name);
    e.events().all().iter().any(|(_, topics, _)| {
        topics
            .get(0)
            .and_then(|t| Symbol::try_from_val(e, &t).ok())
            .is_some_and(|s| s == want)
    })
}

// ─── #476 confidence bands ────────────────────────────────────────────────

#[test]
fn band_is_zero_when_all_sources_agree() {
    for n in 1..20 {
        let v = std::vec![1_234_567i128; n];
        assert_eq!(quartiles(&v), (1_234_567, 1_234_567));
    }
}

#[test]
fn band_flags_low_confidence_below_floor() {
    assert!(is_low_confidence(MIN_BAND_SOURCES - 1, 1));
    assert!(!is_low_confidence(MIN_BAND_SOURCES, 1));
    assert!(is_low_confidence(5, 6));
    assert!(!is_low_confidence(6, 6));
}

#[test]
fn band_exposed_by_query_and_event() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&e, &client);
    assert_eq!(client.get_confidence_band(&asset), None);

    let prices = [100i128, 110, 120, 130, 1000];
    for (i, p) in prices.iter().enumerate() {
        let s = register_test_source(&e, &client, "s");
        client.submit_price(&s, &asset, p, &(i as u64));
    }
    let band = client.get_confidence_band(&asset).unwrap();
    assert_eq!((band.lower, band.upper, band.median), (110, 130, 120));
    assert_eq!(band.num_sources, 5);
    assert_eq!(band.decimals, client.decimals());
    assert!(!band.low_confidence);

    let found = has_event(&e, "confidence_band_event");
    assert!(found, "ConfidenceBandEvent emitted on aggregation");
}

proptest! {
    #[test]
    fn band_contains_median(v in prop::collection::vec(1i128..1_000_000_000, 1..40)) {
        let (lo, hi) = quartiles(&v);
        let m = median(&v);
        prop_assert!(lo <= m && m <= hi);
    }

    #[test]
    fn band_is_decimal_scale_invariant(
        v in prop::collection::vec(1i128..1_000_000, 1..40),
        k in 0u32..12,
    ) {
        let scale = 10i128.pow(k);
        let scaled: std::vec::Vec<i128> = v.iter().map(|p| p * scale).collect();
        let (lo, hi) = quartiles(&v);
        prop_assert_eq!(quartiles(&scaled), (lo * scale, hi * scale));
    }

    /// A moderate source (inside the band) can only tighten the band within
    /// its previous bounds; it can never push it outside them.
    #[test]
    fn band_moderate_source_stays_within_band(
        v in prop::collection::vec(1i128..1_000_000, 2..40),
        t in 0u32..=100,
    ) {
        let (lo, hi) = quartiles(&v);
        let x = lo + (hi - lo) * t as i128 / 100;
        let mut w = v.clone();
        w.push(x);
        let (lo2, hi2) = quartiles(&w);
        prop_assert!(lo <= lo2 && hi2 <= hi);
    }

    /// An extreme source never pulls the band's far side inward.
    #[test]
    fn band_outlier_never_narrows_its_side(
        v in prop::collection::vec(1i128..1_000_000, 2..40),
        d in 1i128..1_000_000,
    ) {
        let (lo, hi) = quartiles(&v);
        let mut up = v.clone();
        up.push(*v.iter().max().unwrap() + d);
        prop_assert!(quartiles(&up).1 >= hi);
        let mut down = v.clone();
        down.push(*v.iter().min().unwrap() - d);
        prop_assert!(quartiles(&down).0 <= lo);
    }
}

// ─── #475 influence caps ──────────────────────────────────────────────────

proptest! {
    #[test]
    fn no_source_exceeds_cap(
        w in prop::collection::vec(1u32..=1000, 3..40),
        cap in 1_000u32..=10_000,
    ) {
        let mut w = w;
        apply_cap(&mut w, cap);
        prop_assert!(within_cap(&w, cap));
        prop_assert!(w.iter().all(|x| *x >= 1), "cap never drops a source");
    }
}

#[test]
fn infeasible_cap_falls_back_to_equal_weight() {
    let mut w = [1000u32, 100, 100];
    apply_cap(&mut w, 1_000); // 3 * 10 % < 100 %
    assert_eq!(w, [1, 1, 1]);
}

#[test]
fn cap_skips_fewer_than_three_sources() {
    let mut w = [1000u32, 100];
    apply_cap(&mut w, 1_000);
    assert_eq!(w, [1000, 100]);
}

#[test]
fn colluding_sources_bounded_by_k_times_cap() {
    // N = 10 sources, k colluders with maximum weight vs honest minimum weight.
    for k in 1..=4usize {
        let cap = 1_200u32;
        let mut w = [100u32; 10];
        w.iter_mut().take(k).for_each(|x| *x = 1000);
        apply_cap(&mut w, cap);
        let total: u64 = w.iter().map(|x| *x as u64).sum();
        let colluding: u64 = w[..k].iter().map(|x| *x as u64).sum();
        assert!(colluding * BPS as u64 <= (k as u64 * cap as u64) * total);
        // 4 * 12 % < 50 %: colluders never hold a weighted majority.
        assert!(colluding * 2 < total);
    }
}

#[test]
fn cap_config_is_admin_only_and_bounded() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    assert_eq!(client.get_influence_cap(), DEFAULT_CAP_BPS);
    assert!(client.try_set_influence_cap(&999).is_err());
    assert!(client.try_set_influence_cap(&10_001).is_err());
    client.set_influence_cap(&2_500);
    assert_eq!(client.get_influence_cap(), 2_500);

    let attacker = Address::generate(&e);
    e.mock_auths(&[MockAuth {
        address: &attacker,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "set_influence_cap",
            args: (5_000u32,).into_val(&e),
            sub_invokes: &[],
        },
    }]);
    assert!(client.try_set_influence_cap(&5_000).is_err());
}

#[test]
fn weighted_aggregation_emits_effective_influence() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let asset = register_test_asset(&e, &client);
    client.set_asset_policy(
        &asset,
        &Some(PolicyOverride {
            method: Some(4),
            min_sources: None,
            freshness_secs: None,
            max_deviation_bps: None,
        }),
    );
    for p in [100i128, 101, 102, 103] {
        let s = register_test_source(&e, &client, "s");
        client.submit_price(&s, &asset, &p, &0);
    }
    assert!(
        has_event(&e, "influence_cap_applied_event"),
        "InfluenceCapAppliedEvent emitted in weighted path"
    );
}

// ─── #468 signed submissions ──────────────────────────────────────────────

fn sign(
    e: &Env,
    contract: &Address,
    key: &SigningKey,
    source: &Address,
    asset: &Address,
    (nonce, price, ts, exp): (u64, i128, u64, u32),
) -> BytesN<64> {
    let digest = e.as_contract(contract, || {
        crate::signed_submission::hash_proof_payload(e, source, asset, nonce, price, ts, exp)
    });
    BytesN::from_array(e, &key.sign(&digest.to_array()).to_bytes())
}

fn signed_setup(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address, SigningKey) {
    let (client, _admin, source, asset) = setup_basic(e);
    let key = SigningKey::from_bytes(&[42; 32]);
    client.register_submission_key(
        &source,
        &BytesN::from_array(e, &key.verifying_key().to_bytes()),
    );
    (client, source, asset, key)
}

#[test]
fn signed_valid_accepted_and_replay_rejected() {
    let e = Env::default();
    let (c, src, asset, key) = signed_setup(&e);
    let f = (1u64, 500i128, 0u64, 100u32);
    let sig = sign(&e, &c.address, &key, &src, &asset, f);
    c.submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &sig);
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &sig)
        .is_err());
}

#[test]
fn signed_every_field_is_bound() {
    let e = Env::default();
    let (c, src, asset, key) = signed_setup(&e);
    let other_asset = register_test_asset(&e, &c);
    let f = (5u64, 500i128, 0u64, 100u32);
    let sig = sign(&e, &c.address, &key, &src, &asset, f);
    let (n, p, t, x) = f;
    assert!(c
        .try_submit_price_with_proof(&src, &other_asset, &p, &t, &n, &x, &sig)
        .is_err());
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &(p + 1), &t, &n, &x, &sig)
        .is_err());
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &p, &(t + 1), &n, &x, &sig)
        .is_err());
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &p, &t, &(n + 1), &x, &sig)
        .is_err());
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &p, &t, &n, &(x + 1), &sig)
        .is_err());
    // Another registered source with the same key cannot reuse the proof.
    let src2 = register_test_source(&e, &c, "s2");
    c.register_submission_key(
        &src2,
        &BytesN::from_array(&e, &key.verifying_key().to_bytes()),
    );
    assert!(c
        .try_submit_price_with_proof(&src2, &asset, &p, &t, &n, &x, &sig)
        .is_err());
    // The untouched proof still verifies.
    c.submit_price_with_proof(&src, &asset, &p, &t, &n, &x, &sig);
}

#[test]
fn signed_replay_across_contract_instances_rejected() {
    let e = Env::default();
    let (c1, src, asset, key) = signed_setup(&e);
    let admin2 = Address::generate(&e);
    let c2 = PriceOracleContractClient::new(&e, &e.register(PriceOracleContract, ()));
    c2.initialize(&admin2, &1, &10, &18, &String::from_str(&e, "second"));
    c2.add_source(&src, &String::from_str(&e, "s"));
    c2.register_asset(&asset);
    c2.register_submission_key(
        &src,
        &BytesN::from_array(&e, &key.verifying_key().to_bytes()),
    );

    let f = (1u64, 500i128, 0u64, 100u32);
    let sig = sign(&e, &c1.address, &key, &src, &asset, f);
    assert!(c2
        .try_submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &sig)
        .is_err());
    c1.submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &sig);
}

#[test]
fn signed_rotated_key_rejected_immediately() {
    let e = Env::default();
    let (c, src, asset, old) = signed_setup(&e);
    let new = SigningKey::from_bytes(&[43; 32]);
    c.register_submission_key(
        &src,
        &BytesN::from_array(&e, &new.verifying_key().to_bytes()),
    );
    let f = (1u64, 500i128, 0u64, 100u32);
    let stale = sign(&e, &c.address, &old, &src, &asset, f);
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &stale)
        .is_err());
    let fresh = sign(&e, &c.address, &new, &src, &asset, f);
    c.submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &fresh);
}

#[test]
fn signed_revoked_source_rejected_and_key_not_resurrected() {
    let e = Env::default();
    let (c, src, asset, key) = signed_setup(&e);
    let f = (1u64, 500i128, 0u64, 100u32);
    let sig = sign(&e, &c.address, &key, &src, &asset, f);
    c.remove_source(&src);
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &sig)
        .is_err());
    // Re-adding the source must not revive the old signing key.
    c.add_source(&src, &String::from_str(&e, "back"));
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &sig)
        .is_err());
}

#[test]
fn signed_deadline_boundary() {
    let e = Env::default();
    let (c, src, asset, key) = signed_setup(&e);
    let seq = e.ledger().sequence();
    let ok = (1u64, 500i128, 0u64, seq);
    let sig = sign(&e, &c.address, &key, &src, &asset, ok);
    c.submit_price_with_proof(&src, &asset, &ok.1, &ok.2, &ok.0, &ok.3, &sig);

    let late = (2u64, 500i128, 0u64, seq);
    let sig = sign(&e, &c.address, &key, &src, &asset, late);
    e.ledger().with_mut(|l| l.sequence_number = seq + 1);
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &late.1, &late.2, &late.0, &late.3, &sig)
        .is_err());
}

#[test]
fn signed_path_honours_freeze() {
    let e = Env::default();
    let (c, src, asset, key) = signed_setup(&e);
    c.freeze_price(&asset, &String::from_str(&e, "halt"));
    let f = (1u64, 500i128, 0u64, 100u32);
    let sig = sign(&e, &c.address, &key, &src, &asset, f);
    assert!(c
        .try_submit_price_with_proof(&src, &asset, &f.1, &f.2, &f.0, &f.3, &sig)
        .is_err());
}

// ─── #467 RBAC / privilege escalation ─────────────────────────────────────

/// Every entrypoint in `lib.rs` appears exactly once in the committed matrix.
#[test]
fn matrix_covers_every_endpoint() {
    let lib = include_str!("lib.rs");
    let body = &lib[lib.find("impl PriceOracleContract {").unwrap()..];
    let endpoints: BTreeSet<std::string::String> = body
        .lines()
        .filter_map(|l| l.strip_prefix("    pub fn "))
        .map(|l| l.split(['(', '<']).next().unwrap().trim().to_string())
        .collect();
    let matrix = include_str!("../../../docs/security/endpoint-authority-matrix.md");
    let table = &matrix[matrix.find("| Endpoint | Authority |").unwrap()..];
    let listed: BTreeSet<std::string::String> = table
        .lines()
        .filter_map(|l| l.strip_prefix("| `"))
        .filter_map(|l| l.split('`').next())
        .map(|n| n.to_string())
        .collect();
    let missing: std::vec::Vec<_> = endpoints.difference(&listed).collect();
    assert!(
        missing.is_empty(),
        "endpoints missing from matrix: {missing:?}"
    );
    let stale: std::vec::Vec<_> = listed.difference(&endpoints).collect();
    assert!(
        stale.is_empty(),
        "matrix lists removed endpoints: {stale:?}"
    );
}

fn only_auth(
    e: &Env,
    who: &Address,
    contract: &Address,
    fn_name: &'static str,
    args: soroban_sdk::Vec<soroban_sdk::Val>,
) {
    e.mock_auths(&[MockAuth {
        address: who,
        invoke: &MockAuthInvoke {
            contract,
            fn_name,
            args,
            sub_invokes: &[],
        },
    }]);
}

#[test]
fn unauthorized_caller_rejected_on_privileged_endpoints() {
    let e = Env::default();
    let (c, _admin, source, asset) = setup_basic(&e);
    let x = Address::generate(&e);
    let name = String::from_str(&e, "n");

    only_auth(
        &e,
        &x,
        &c.address,
        "add_source",
        (x.clone(), name.clone()).into_val(&e),
    );
    assert!(c.try_add_source(&x, &name).is_err());
    only_auth(
        &e,
        &x,
        &c.address,
        "remove_source",
        (source.clone(),).into_val(&e),
    );
    assert!(c.try_remove_source(&source).is_err());
    only_auth(
        &e,
        &x,
        &c.address,
        "register_asset",
        (x.clone(),).into_val(&e),
    );
    assert!(c.try_register_asset(&x).is_err());
    only_auth(
        &e,
        &x,
        &c.address,
        "delegate_role",
        (x.clone(), Role::UpgradeManager).into_val(&e),
    );
    assert!(c.try_delegate_role(&x, &Role::UpgradeManager).is_err());
    only_auth(
        &e,
        &x,
        &c.address,
        "transfer_admin",
        (x.clone(),).into_val(&e),
    );
    assert!(c.try_transfer_admin(&x).is_err());
    only_auth(
        &e,
        &x,
        &c.address,
        "set_influence_cap",
        (1_000u32,).into_val(&e),
    );
    assert!(c.try_set_influence_cap(&1_000).is_err());
    only_auth(
        &e,
        &x,
        &c.address,
        "submit_price",
        (source.clone(), asset.clone(), 1i128, 0u64).into_val(&e),
    );
    assert!(c.try_submit_price(&source, &asset, &1, &0).is_err());
    only_auth(
        &e,
        &x,
        &c.address,
        "add_authorized_consumer",
        (x.clone(),).into_val(&e),
    );
    assert!(c.try_add_authorized_consumer(&x).is_err());
}

#[test]
fn previously_unguarded_endpoints_now_require_auth() {
    let e = Env::default();
    let (c, _admin, _source, asset) = setup_basic(&e);
    let victim = Address::generate(&e);
    let op = String::from_str(&e, "op");
    let deps: Vec<String> = Vec::new(&e);

    e.mock_auths(&[]);
    assert!(c.try_create_operation(&op, &deps).is_err());
    assert!(c.try_cancel_dependent_operation(&op).is_err());
    assert!(c.try_execute_dependent_operation(&op).is_err());
    assert!(c.try_claim_rewards(&victim).is_err());
    assert!(c
        .try_challenge_price(&victim, &asset, &1, &soroban_sdk::Bytes::new(&e))
        .is_err());
}

#[test]
fn delegated_roles_grant_no_admin_authority() {
    let e = Env::default();
    let (c, _admin) = setup_contract(&e);
    let d = Address::generate(&e);
    for r in [
        Role::SourceManager,
        Role::AssetManager,
        Role::PriceUpdater,
        Role::ConfigManager,
        Role::UpgradeManager,
    ] {
        c.delegate_role(&d, &r);
    }
    let name = String::from_str(&e, "n");
    only_auth(
        &e,
        &d,
        &c.address,
        "add_source",
        (d.clone(), name.clone()).into_val(&e),
    );
    assert!(c.try_add_source(&d, &name).is_err());
    only_auth(
        &e,
        &d,
        &c.address,
        "delegate_role",
        (d.clone(), Role::SourceManager).into_val(&e),
    );
    assert!(
        c.try_delegate_role(&d, &Role::SourceManager).is_err(),
        "no self-grant"
    );
    only_auth(
        &e,
        &d,
        &c.address,
        "transfer_admin",
        (d.clone(),).into_val(&e),
    );
    assert!(c.try_transfer_admin(&d).is_err());
}

#[test]
fn revocation_is_immediate_and_complete() {
    let e = Env::default();
    let (c, _admin) = setup_contract(&e);
    let d = Address::generate(&e);
    c.delegate_role(&d, &Role::SourceManager);
    c.delegate_role(&d, &Role::AssetManager);
    c.revoke_role(&d, &Role::SourceManager);
    // Same ledger: revocation already effective, other role untouched.
    assert!(!c.has_role(&d, &Role::SourceManager));
    assert!(c.has_role(&d, &Role::AssetManager));
    c.revoke_role(&d, &Role::AssetManager);
    assert!(c.get_roles_for_holder(&d).is_empty());
}

#[test]
fn admin_transfer_is_atomic() {
    let e = Env::default();
    let (c, old) = setup_contract(&e);
    let new = Address::generate(&e);
    c.transfer_admin(&new);
    // No overlap: the old admin lost authority in the same call...
    only_auth(
        &e,
        &old,
        &c.address,
        "set_influence_cap",
        (2_000u32,).into_val(&e),
    );
    assert!(c.try_set_influence_cap(&2_000).is_err());
    // ...and no gap: the new admin already holds it.
    only_auth(
        &e,
        &new,
        &c.address,
        "set_influence_cap",
        (2_000u32,).into_val(&e),
    );
    c.set_influence_cap(&2_000);
}

#[test]
fn removed_consumer_loses_access_immediately() {
    let e = Env::default();
    let (c, _admin) = setup_contract(&e);
    let consumer = Address::generate(&e);
    c.set_consumer_access_mode(&1);
    c.add_authorized_consumer(&consumer);
    assert!(c.is_consumer_authorized(&consumer));
    c.remove_authorized_consumer(&consumer);
    assert!(!c.is_consumer_authorized(&consumer));
}
