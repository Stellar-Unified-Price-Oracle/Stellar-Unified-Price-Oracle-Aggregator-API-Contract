//! Issue #457 — Rate-limit and quota evasion, plus quota-based denial of service.
//!
//! # Limit inventory
//!
//! | Limiter | Scope | Storage | Limit | Reset | Wired into an endpoint? |
//! |---|---|---|---|---|---|
//! | `rate_limiting::check_rate_limit` | per consumer, per ledger | persistent `QueryCount(consumer, seq)` | Free 10 / Basic 100 / Premium unlimited | new ledger sequence | no |
//! | `prices::check_rate_limit_and_increment` | per consumer, per ledger | temporary `QueryCount(consumer, seq)` | `QueryRateLimit` (default 100); subscribers exempt | new ledger sequence | no |
//! | `storage::check_rate_limit` + `increment_query_count` | per consumer, per ledger | temporary `QueryCount(consumer, seq)` | `QueryRateLimit` | new ledger sequence | no |
//!
//! There is no per-source, per-asset or global limiter. None of the consumer
//! limiters is called from a public endpoint, so today no query is throttled
//! and there is no limiter to enforce "before expensive work".
//!
//! # Findings
//!
//! * **Identity rotation (evasion).** Counters are keyed by the caller-supplied
//!   `consumer` address, which is not authenticated. Rotation costs nothing:
//!   `N` fresh addresses give `N × limit` queries per ledger (10 addresses on the
//!   Free tier = 100 queries per ledger, the same as one Basic subscription).
//! * **Fragmentation.** The bound is `limit` per `(consumer, ledger)`; splitting
//!   work across calls in one ledger cannot exceed it. Across ledgers the
//!   aggregate bound is `limit × ledgers` by design.
//! * **Same-ledger races.** Soroban executes transactions serially, so calls in
//!   one ledger see each other's increments; the `limit + 1`-th call is denied.
//! * **Quota exhaustion (abuse).** Because `consumer` is not authenticated, any
//!   caller can burn a victim's per-ledger quota and deterministically deny the
//!   victim for the rest of that ledger. Residual exposure: one ledger per
//!   attack transaction batch. Must be fixed (require `consumer.require_auth()`)
//!   before any limiter is wired into an endpoint.
//! * **Error path.** A denied call does not write the counter, so repeatedly
//!   hitting the limit costs the target no storage.
//! * **Key collision.** `rate_limiting` stores `QueryCount` in persistent
//!   storage while `prices`/`storage` use temporary storage under the same key;
//!   the two counters never see each other.

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::rate_limiting::{check_rate_limit, grant_enterprise_tier};
use crate::test_helpers::{ledger_default, setup_contract};
use crate::types::DataKey;

fn persistent_count(e: &Env, id: &Address, consumer: &Address) -> u32 {
    e.as_contract(id, || {
        let key = DataKey::QueryCount(consumer.clone(), e.ledger().sequence());
        e.storage().persistent().get(&key).unwrap_or(0)
    })
}

fn allowed(e: &Env, id: &Address, consumer: &Address) -> bool {
    e.as_contract(id, || !check_rate_limit(e, consumer.clone()))
}

#[test]
fn free_tier_denies_the_eleventh_call_in_one_ledger() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);
    let consumer = Address::generate(&e);

    for _ in 0..10 {
        assert!(allowed(&e, &client.address, &consumer));
    }
    assert!(!allowed(&e, &client.address, &consumer));
    assert_eq!(persistent_count(&e, &client.address, &consumer), 10);
}

#[test]
fn fragmentation_cannot_exceed_per_ledger_bound() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let consumer = Address::generate(&e);

    for seq in 100..103u32 {
        ledger_default(&e, seq, seq as u64 * 5);
        let mut granted = 0;
        for _ in 0..25 {
            if allowed(&e, &client.address, &consumer) {
                granted += 1;
            }
        }
        assert_eq!(granted, 10);
    }
}

#[test]
fn denied_calls_do_not_write_storage() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);
    let consumer = Address::generate(&e);

    for _ in 0..50 {
        allowed(&e, &client.address, &consumer);
    }
    assert_eq!(persistent_count(&e, &client.address, &consumer), 10);
}

#[test]
fn identity_rotation_multiplies_quota_linearly() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);

    let mut granted = 0u32;
    for _ in 0..10 {
        let fresh = Address::generate(&e);
        while allowed(&e, &client.address, &fresh) {
            granted += 1;
        }
    }
    // Documented exposure: 10 free identities == one Basic subscription.
    assert_eq!(granted, 100);
}

#[test]
fn gap_unauthenticated_consumer_lets_attacker_exhaust_victim_quota() {
    let e = Env::default();
    ledger_default(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);
    let victim = Address::generate(&e);

    // Attacker spends the victim's quota without the victim's auth.
    e.set_auths(&[]);
    for _ in 0..10 {
        allowed(&e, &client.address, &victim);
    }
    assert!(!allowed(&e, &client.address, &victim));

    // The denial lasts only until the next ledger.
    ledger_default(&e, 101, 1_005);
    assert!(allowed(&e, &client.address, &victim));
}

#[test]
fn enterprise_tier_is_unlimited_and_not_counted() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);
    let consumer = Address::generate(&e);
    e.as_contract(&client.address, || {
        grant_enterprise_tier(&e, consumer.clone())
    });

    for _ in 0..200 {
        assert!(allowed(&e, &client.address, &consumer));
    }
    assert_eq!(persistent_count(&e, &client.address, &consumer), 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #16)")]
fn query_limiter_panics_before_incrementing() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);
    client.set_query_rate_limit(&3u32);
    let consumer = Address::generate(&e);

    e.as_contract(&client.address, || {
        for _ in 0..3 {
            crate::prices::check_rate_limit_and_increment(&e, &consumer);
        }
        let key = DataKey::QueryCount(consumer.clone(), 100);
        assert_eq!(e.storage().temporary().get::<_, u32>(&key), Some(3));
        crate::prices::check_rate_limit_and_increment(&e, &consumer);
    });
}

#[test]
fn persistent_and_temporary_counters_do_not_share_state() {
    let e = Env::default();
    e.mock_all_auths();
    ledger_default(&e, 100, 1_000);
    let (client, _) = setup_contract(&e);
    let consumer = Address::generate(&e);

    for _ in 0..10 {
        allowed(&e, &client.address, &consumer);
    }
    e.as_contract(&client.address, || {
        assert!(crate::storage::check_rate_limit(&e, &consumer));
    });
}
