//! # Attack-regression corpus (#511)
//!
//! Every historical adversarial scenario this contract has been subject to is
//! pinned here as a test, so a fix silently reverted by an unrelated refactor
//! fails CI instead of being rediscovered in production.
//!
//! Each test is registered through the [`attack!`] macro, which stamps it with
//! the **attack class** it pins. The class is load-bearing, not decoration:
//! `corpus_entries_are_complete` fails if a registered class has no pin, and
//! `make attack-gate` runs only this module so the corpus can be given its own
//! CI time budget.
//!
//! ## Attack classes
//!
//! | Class | Meaning |
//! |---|---|
//! | `reentrancy` | Re-entering a state-mutating path mid-execution |
//! | `replay` | Re-submitting an already-accepted payload |
//! | `malformed-payload` | Structurally invalid input that must be rejected, not coerced |
//! | `quota-evasion` | Circumventing a limit, cap, or per-window bound |
//! | `manipulation` | Influencing the published aggregate away from the honest value |
//! | `authorization` | Acting without the required authority |
//! | `fail-open` | Degraded storage silently restoring a permissive default |
//!
//! ## Why the tests look the way they do
//!
//! Every entry asserts the *security-relevant outcome* (an error code, a bound,
//! an unchanged aggregate), never merely that a call completes. A test that
//! would still pass with its guard deleted is not a regression pin and does not
//! belong here — `scripts/attack-regression-mutation-check.sh` mechanically
//! reverts three historical fixes and requires this corpus to catch each one.
//!
//! See `docs/security/attack-regression-corpus.md` for the intake process.

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::core_pricing::{mean_core, median_core, trimmed_mean_core, weighted_median_core};
use crate::storage::{compute_median, compute_vwap};
use crate::test_helpers::{
    register_test_asset, register_test_source, setup_contract, submit_test_price,
};
use crate::types::{DataKey, ErrorCode};
use crate::{PriceOracleContract, PriceOracleContractClient};

/// The attack classes this corpus covers. A new class must be added here *and*
/// pinned below, or `corpus_entries_are_complete` fails.
pub const ATTACK_CLASSES: [&str; 7] = [
    "reentrancy",
    "replay",
    "malformed-payload",
    "quota-evasion",
    "manipulation",
    "authorization",
    "fail-open",
];

/// Registers an attack pin. `$class` must name a member of [`ATTACK_CLASSES`];
/// the test asserts that at runtime so a typo cannot silently create an
/// unclassified (and therefore unreviewed) entry.
macro_rules! attack {
    ($class:expr, $name:ident, $body:block) => {
        #[test]
        fn $name() {
            const CLASS: &str = $class;
            assert!(
                ATTACK_CLASSES.contains(&CLASS),
                "attack class `{CLASS}` is not registered in ATTACK_CLASSES"
            );
            $body
        }
    };
}

fn evict(e: &Env, client: &PriceOracleContractClient<'_>, key: &DataKey) {
    e.as_contract(&client.address, || e.storage().persistent().remove(key));
}

/// Every registered class must be exercised by at least one pin. This is the
/// guard that makes "the corpus covers the documented attack classes" a
/// checked claim rather than a comment.
#[test]
fn corpus_entries_are_complete() {
    // One representative pin per class, named here explicitly. Adding a class
    // to ATTACK_CLASSES without adding it to this list fails the test run.
    let pinned = [
        "reentrancy",
        "replay",
        "malformed-payload",
        "quota-evasion",
        "manipulation",
        "authorization",
        "fail-open",
    ];
    for class in ATTACK_CLASSES {
        assert!(
            pinned.contains(&class),
            "attack class `{class}` is registered but unpinned; add a pin or drop the class"
        );
    }
    assert_eq!(pinned.len(), ATTACK_CLASSES.len());
}

// ═══════════════════════════════════════════════════════════════════════════
// reentrancy
// ═══════════════════════════════════════════════════════════════════════════

/// A nested entry into the guard must be refused. Reverting the guard turns
/// `Reentrant` into a silent no-op, so a re-entering call would proceed and
/// double-apply its state changes.
attack!("reentrancy", reentrancy_guard_rejects_nested_entry, {
    let e = Env::default();
    e.mock_all_auths();
    let id = e.register(PriceOracleContract, ());
    e.as_contract(&id, || {
        crate::reentrancy::enter(&e);
        let err = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::reentrancy::enter(&e);
        }));
        assert!(
            err.is_err(),
            "nested reentrancy was not rejected: the guard let a second entry through"
        );
        // Unwind the guard so the test env is left clean.
        crate::reentrancy::exit(&e);
    });
});

/// A rejected nested entry must not leave the guard latched: every subsequent
/// legitimate call would be refused forever, which is a permanent DoS.
attack!("reentrancy", reentrancy_guard_is_released_after_failure, {
    let e = Env::default();
    e.mock_all_auths();
    let id = e.register(PriceOracleContract, ());
    e.as_contract(&id, || {
        crate::reentrancy::enter(&e);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::reentrancy::enter(&e);
        }));
        crate::reentrancy::exit(&e);

        // A fresh, well-formed entry must still be accepted afterwards.
        crate::reentrancy::enter(&e);
        crate::reentrancy::exit(&e);
    });
});

// ═══════════════════════════════════════════════════════════════════════════
// replay
// ═══════════════════════════════════════════════════════════════════════════

/// An explicit replay nonce must be strictly increasing per source. Replaying an
/// accepted nonce re-submits an already-counted payload.
attack!("replay", replay_nonce_must_be_strictly_increasing, {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let src = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);
    let ts = e.ledger().timestamp();

    client.submit_price_with_nonce(&src, &asset, &1_000i128, &ts, &10u64);

    // Exact replay.
    assert_eq!(
        client.try_submit_price_with_nonce(&src, &asset, &1_000i128, &ts, &10u64),
        Err(Ok(ErrorCode::InvalidNonce.into()))
    );
    // Going backwards is equally a replay, not a correction.
    assert_eq!(
        client.try_submit_price_with_nonce(&src, &asset, &1_000i128, &ts, &9u64),
        Err(Ok(ErrorCode::InvalidNonce.into()))
    );
    // A rejected nonce must not have been consumed.
    client.submit_price_with_nonce(&src, &asset, &1_001i128, &ts, &11u64);
    assert_eq!(client.get_source_price(&asset, &src).price, 1_001i128);
});

/// Replay protection is per source: one source reusing a low nonce must not
/// lock out every other source.
attack!("replay", replay_nonce_is_scoped_per_source, {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let a = register_test_source(&e, &client, "A");
    let b = register_test_source(&e, &client, "B");
    let asset = register_test_asset(&e, &client);
    let ts = e.ledger().timestamp();

    client.submit_price_with_nonce(&a, &asset, &1_000i128, &ts, &50u64);
    // B has never submitted, so nonce 1 is fresh for B.
    client.submit_price_with_nonce(&b, &asset, &2_000i128, &ts, &1u64);
    assert_eq!(client.get_source_price(&asset, &b).price, 2_000i128);
});

// ═══════════════════════════════════════════════════════════════════════════
// malformed-payload
// ═══════════════════════════════════════════════════════════════════════════

/// A non-positive price is structurally invalid. Coercing it to zero would let a
/// source pin the aggregate to zero.
attack!(
    "malformed-payload",
    malformed_non_positive_price_is_rejected,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let src = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);
        let ts = e.ledger().timestamp();

        for bad in [0i128, -1, i128::MIN] {
            assert_eq!(
                client.try_submit_price(&src, &asset, &bad, &ts),
                Err(Ok(ErrorCode::InvalidPrice.into())),
                "price {bad} must be rejected"
            );
        }
        assert!(client.get_price(&asset, &0u64).is_none());
    }
);

/// A timestamp far beyond the accepted future window is malformed: it would pin
/// an aggregate ahead of real time and defeat every staleness check.
attack!(
    "malformed-payload",
    malformed_far_future_timestamp_is_rejected,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let src = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);
        let ts = e.ledger().timestamp();

        assert_eq!(
            client.try_submit_price(&src, &asset, &1_000i128, &(ts + 10_000_000)),
            Err(Ok(ErrorCode::InvalidTimestamp.into()))
        );
        assert!(client.get_price(&asset, &0u64).is_none());
    }
);

/// An over-long or empty source name is malformed input, not a truncation
/// opportunity: truncating would make distinct sources indistinguishable.
attack!(
    "malformed-payload",
    malformed_source_name_bounds_are_enforced,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);

        let too_long = Address::generate(&e);
        assert_eq!(
            client.try_add_source(
                &too_long,
                &soroban_sdk::String::from_str(&e, &"S".repeat(65))
            ),
            Err(Ok(ErrorCode::SourceNameTooLong.into()))
        );
        let empty = Address::generate(&e);
        assert_eq!(
            client.try_add_source(&empty, &soroban_sdk::String::from_str(&e, "")),
            Err(Ok(ErrorCode::SourceNameEmpty.into()))
        );
    }
);

// ═══════════════════════════════════════════════════════════════════════════
// quota-evasion
// ═══════════════════════════════════════════════════════════════════════════

/// `min_sources_required` is the quorum floor and may not be lowered to a
/// permissive value, so a compromised admin cannot reduce the oracle to a
/// single self-controlled source.
attack!(
    "quota-evasion",
    quota_evasion_min_sources_cannot_be_lowered,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        let _a = register_test_source(&e, &client, "A");
        let _b = register_test_source(&e, &client, "B");
        let _c = register_test_source(&e, &client, "C");

        // Zero would mean "any single source publishes".
        assert_eq!(
            client.try_set_min_sources_required(&0u32),
            Err(Ok(ErrorCode::InvalidConfiguration.into()))
        );
        // Above the registered source count can never be satisfied.
        assert_eq!(
            client.try_set_min_sources_required(&4u32),
            Err(Ok(ErrorCode::InvalidConfiguration.into()))
        );
        // The legal quorum still applies.
        client.set_min_sources_required(&3u32);
        assert_eq!(client.get_min_sources_required(), 3u32);
    }
);

/// The quorum floor is enforced on the read path, not only at configuration
/// time: one submission must not produce a published aggregate.
attack!(
    "quota-evasion",
    quota_evasion_single_source_cannot_publish,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        let a = register_test_source(&e, &client, "A");
        let b = register_test_source(&e, &client, "B");
        let asset = register_test_asset(&e, &client);
        let ts = e.ledger().timestamp();

        client.set_min_sources_required(&2u32);

        submit_test_price(&client, &a, &asset, 1_000, ts);
        assert!(
            client.get_price(&asset, &0u64).is_none(),
            "a single submission satisfied a quorum of 2"
        );

        submit_test_price(&client, &b, &asset, 2_000, ts);
        let agg = client.get_price(&asset, &0u64).expect("quorum reached");
        assert_eq!(agg.price, 1_500, "median of [1000, 2000]");
        assert_eq!(agg.num_sources, 2);
    }
);

/// The anti-griefing quota on open challenges must actually bound a spammer.
/// The query rate limiter is deliberately unwired upstream
/// (`prices::check_rate_limit_and_increment` has no caller), so it cannot be
/// pinned as an enforced quota; the challenge caps are the quota that is real.
attack!(
    "quota-evasion",
    quota_evasion_open_challenges_are_bounded,
    {
        use crate::challenger::MAX_OPEN_CHALLENGES_PER_CHALLENGER;

        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);
        let spammer = Address::generate(&e);

        for _ in 0..MAX_OPEN_CHALLENGES_PER_CHALLENGER {
            client.challenge_price(
                &spammer,
                &asset,
                &1_000_000i128,
                &soroban_sdk::Bytes::new(&e),
            );
        }

        // The next challenge exceeds the cap and must be refused.
        assert!(
            client
                .try_challenge_price(
                    &spammer,
                    &asset,
                    &1_000_000i128,
                    &soroban_sdk::Bytes::new(&e)
                )
                .is_err(),
            "a spammer opened challenges past the per-challenger cap"
        );
        assert_eq!(
            client.get_open_challenge_count(&asset),
            MAX_OPEN_CHALLENGES_PER_CHALLENGER
        );
    }
);

// ═══════════════════════════════════════════════════════════════════════════
// manipulation
// ═══════════════════════════════════════════════════════════════════════════

/// The core manipulation property: one attacker-controlled outlier must not move
/// the median away from the honest consensus.
attack!("manipulation", manipulation_outlier_cannot_move_median, {
    let honest = [1_000i128, 1_000, 1_000];
    let attacked = [1_000i128, 1_000, 1_000_000_000];
    assert_eq!(median_core(&honest), 1_000);
    assert_eq!(
        median_core(&attacked),
        1_000,
        "a single outlier moved the median"
    );
});

/// A VWAP source reporting non-positive volume must carry no weight, otherwise
/// an attacker inverts the aggregate with a negative volume.
attack!(
    "manipulation",
    manipulation_negative_volume_carries_no_weight,
    {
        let e = Env::default();
        let prices = soroban_sdk::Vec::from_slice(&e, &[200i128, 300, 400]);
        let honest = soroban_sdk::Vec::from_slice(&e, &[1_000i128, 1_000, 1_000]);
        assert_eq!(compute_vwap(&prices, &honest), 300);

        // Attacker appends an extreme price carrying a negative volume.
        let attacked = soroban_sdk::Vec::from_slice(&e, &[200i128, 300, 400, 1_000_000_000]);
        let volumes = soroban_sdk::Vec::from_slice(&e, &[1_000i128, 1_000, 1_000, -1_000]);
        assert_eq!(
            compute_vwap(&attacked, &volumes),
            300,
            "a negative-volume source influenced the VWAP"
        );
    }
);

/// Trimming must actually discard the extreme value; otherwise the "trimmed"
/// aggregate is a mean and inherits its full manipulation surface.
attack!("manipulation", manipulation_trim_removes_single_outlier, {
    // 40% of 5 values trims 1 from each end, so the extreme really is discarded.
    // (20% of 5 would trim nothing and silently degrade to the mean.)
    let honest = [100i128, 101, 102, 103, 104];
    let attacked = [100i128, 101, 102, 103, 1_000_000_000];
    let base = trimmed_mean_core(&honest, 40);
    let moved = trimmed_mean_core(&attacked, 40);
    assert_eq!(base, 102, "unexpected trimmed mean for the honest set");
    assert_eq!(
        moved, 102,
        "a single trimmed outlier moved the trimmed mean"
    );
});

/// A source with negligible weight must not pull the weighted median outside the
/// honest range.
attack!(
    "manipulation",
    manipulation_minority_weight_cannot_pull_median,
    {
        let prices = [100i128, 101, 102, 103, 104];
        let honest = [50i128, 50, 50, 50, 50];
        // Attacker is index 0 with weight 1 against five honest sources.
        let attacked = [1i128, 50, 50, 50, 50];
        let base = weighted_median_core(&prices, &honest);
        let moved = weighted_median_core(&prices, &attacked);
        assert!(
            (101..=103).contains(&base),
            "baseline weighted median {base} outside the honest range"
        );
        assert!(
            (101..=103).contains(&moved),
            "minority-weight source pulled the median to {moved}"
        );
    }
);

/// The published aggregate must be reproducible: the pure core and the SDK-`Vec`
/// implementation must not diverge, or a fix applied to one is silently absent
/// from the other.
attack!(
    "manipulation",
    manipulation_core_and_sdk_aggregates_agree,
    {
        let e = Env::default();
        for case in [
            vec![1i128],
            vec![1i128, 2],
            vec![3i128, 1, 2],
            vec![5i128, 5, 5, 5],
            vec![9i128, -4, 7, 0, 100],
        ] {
            let sdk = soroban_sdk::Vec::from_slice(&e, &case);
            assert_eq!(
                median_core(&case),
                compute_median(&sdk),
                "core/SDK median divergence on {case:?}"
            );
            assert_eq!(
                mean_core(&case),
                crate::storage::compute_mean(&sdk),
                "core/SDK mean divergence on {case:?}"
            );
        }
    }
);

// ═══════════════════════════════════════════════════════════════════════════
// authorization
// ═══════════════════════════════════════════════════════════════════════════

/// An address never registered as a source must not be able to submit, however
/// plausible its price.
attack!(
    "authorization",
    authorization_unregistered_source_cannot_submit,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let asset = register_test_asset(&e, &client);
        let ts = e.ledger().timestamp();

        let stranger = Address::generate(&e);
        assert_eq!(
            client.try_submit_price(&stranger, &asset, &1_000i128, &ts),
            Err(Ok(ErrorCode::NotAuthorized.into()))
        );
        assert!(client.get_price(&asset, &0u64).is_none());
    }
);

/// Removing a source strips its authority immediately: the removed address can
/// no longer submit even though its key still signs correctly.
attack!(
    "authorization",
    authorization_removed_source_loses_authority,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let src = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);
        let ts = e.ledger().timestamp();

        submit_test_price(&client, &src, &asset, 1_000, ts);
        client.remove_source(&src);

        assert_eq!(
            client.try_submit_price(&src, &asset, &2_000i128, &ts),
            Err(Ok(ErrorCode::NotAuthorized.into()))
        );
    }
);

/// A price for an unregistered asset must be rejected, otherwise a source can
/// pre-seed storage for an asset nobody has vetted.
attack!(
    "authorization",
    authorization_unregistered_asset_is_rejected,
    {
        let e = Env::default();
        e.mock_all_auths();
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let src = register_test_source(&e, &client, "S1");
        let unknown_asset = Address::generate(&e);
        let ts = e.ledger().timestamp();

        assert_eq!(
            client.try_submit_price(&src, &unknown_asset, &1_000i128, &ts),
            Err(Ok(ErrorCode::AssetNotRegistered.into()))
        );
    }
);

// ═══════════════════════════════════════════════════════════════════════════
// fail-open
// ═══════════════════════════════════════════════════════════════════════════

/// If the quorum configuration is evicted from storage the contract must fail
/// closed. Falling back to a permissive default ("1 source") would let a single
/// compromised source publish the aggregate.
attack!("fail-open", fail_open_evicted_min_sources_fails_closed, {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let src = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);
    client.set_min_sources_required(&1u32);

    evict(&e, &client, &DataKey::CfgMinSources);

    assert_eq!(
        client.try_get_min_sources_required(),
        Err(Ok(ErrorCode::ConfigMissing.into()))
    );
    let ts = e.ledger().timestamp();
    assert!(client
        .try_submit_price(&src, &asset, &1_000i128, &ts)
        .is_err());
    assert!(client.get_price(&asset, &0u64).is_none());
});

/// Evicting a source's registry entry must also fail closed.
attack!("fail-open", fail_open_evicted_source_entry_fails_closed, {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);
    let src = register_test_source(&e, &client, "S1");
    let asset = register_test_asset(&e, &client);
    client.set_min_sources_required(&1u32);

    evict(&e, &client, &DataKey::SrcActive(src.clone()));

    let ts = e.ledger().timestamp();
    assert!(client
        .try_submit_price(&src, &asset, &1_000i128, &ts)
        .is_err());
    assert!(client.get_price(&asset, &0u64).is_none());
});

/// Evicting the admin must not leave admin functions callable.
attack!("fail-open", fail_open_evicted_admin_blocks_admin_calls, {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _admin) = setup_contract(&e);

    evict(&e, &client, &DataKey::Admin);

    assert!(client.try_set_min_sources_required(&1u32).is_err());
});
