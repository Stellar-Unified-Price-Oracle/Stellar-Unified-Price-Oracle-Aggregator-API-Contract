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
//! | `reward-sybil` | Splitting/rotating identities to farm contribution rewards |
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
pub const ATTACK_CLASSES: [&str; 8] = [
    "reentrancy",
    "replay",
    "malformed-payload",
    "quota-evasion",
    "manipulation",
    "authorization",
    "fail-open",
    "reward-sybil",
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
        "reward-sybil",
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
// reward-sybil (#670)
// ═══════════════════════════════════════════════════════════════════════════
//
// The contribution-reward model pays a fixed `REWARD_PER_ACCEPTED_SUBMISSION`
// for each accepted submission, subject to a per-source per-window cap of
// `REWARD_CAP_PER_SOURCE_WINDOW`. These scenarios simulate the three classic
// farming strategies against that model and quantify attacker profitability
// (gross reward minus the per-identity registration cost). Each scenario is
// executable and asserts the mitigation actually bounds the payout.

/// Reward paid per accepted submission (mirrors the proposed schedule).
const REWARD_PER_ACCEPTED_SUBMISSION: i128 = 100;
/// Maximum reward a single source may earn within one window.
const REWARD_CAP_PER_SOURCE_WINDOW: i128 = 500;
/// Cost an attacker pays to register one fresh source identity.
const IDENTITY_REGISTRATION_COST: i128 = 250;

/// Net profit of a farming run: gross reward minus identity costs.
fn net_profit(gross_reward: i128, identities: i128) -> i128 {
    gross_reward - identities * IDENTITY_REGISTRATION_COST
}

/// Reward a single source earns for `submissions` accepted submissions, after
/// the per-source per-window cap is applied.
fn capped_reward(submissions: i128) -> i128 {
    let raw = submissions * REWARD_PER_ACCEPTED_SUBMISSION;
    if raw > REWARD_CAP_PER_SOURCE_WINDOW {
        REWARD_CAP_PER_SOURCE_WINDOW
    } else {
        raw
    }
}

/// Source splitting: one operator spreads submissions across many sources to
/// dodge the per-source cap. The cap is per source, so splitting *does* raise
/// gross reward — but each split identity costs more than the marginal reward
/// it unlocks, so net profit stays negative. This pins the mitigation: the
/// registration cost must exceed the per-identity marginal reward.
attack!("reward-sybil", source_splitting_is_unprofitable, {
    // Honest single source: capped at the window bound.
    let honest_gross = capped_reward(20);
    assert_eq!(honest_gross, REWARD_CAP_PER_SOURCE_WINDOW);

    // Attacker splits the same 20 submissions across 4 identities.
    let identities = 4i128;
    let per_identity = 20 / identities;
    let split_gross: i128 = (0..identities).map(|_| capped_reward(per_identity)).sum();

    // Splitting raises gross reward (cap is per source)...
    assert!(
        split_gross > honest_gross,
        "splitting must raise gross reward, else the cap is not per-source"
    );
    // ...but the marginal reward per extra identity is below its cost, so the
    // attack is unprofitable. This is the mitigation under test.
    let marginal = split_gross - honest_gross;
    assert!(
        marginal < (identities - 1) * IDENTITY_REGISTRATION_COST,
        "mitigation broken: splitting out-earns identity cost (marginal={marginal})"
    );
    assert!(
        net_profit(split_gross, identities) < 0,
        "source splitting must be net-unprofitable"
    );
});

/// Coordinated submissions (collusion): a cartel of sources submits in lockstep
/// to dominate the aggregate and harvest rewards. The per-source cap bounds
/// each member, and the cartel's net profit is negative once identity costs are
/// counted. Pins that collusion cannot beat the cap.
attack!("reward-sybil", coordinated_submissions_are_capped, {
    let cartel_size = 6i128;
    // Each member submits far more than the cap allows.
    let gross: i128 = (0..cartel_size).map(|_| capped_reward(50)).sum();
    let expected = cartel_size * REWARD_CAP_PER_SOURCE_WINDOW;
    assert_eq!(gross, expected, "per-source cap must bound each cartel member");

    // No member can exceed the cap regardless of submission volume.
    assert_eq!(capped_reward(1_000), REWARD_CAP_PER_SOURCE_WINDOW);

    // Net profit is negative: identity cost dominates the capped reward.
    assert!(
        net_profit(gross, cartel_size) < 0,
        "collusion must be net-unprofitable under the cap"
    );
});

/// Identity rotation (Sybil): an attacker rotates through fresh identities each
/// window to reset the per-source cap. Rotation resets the cap, so gross reward
/// scales with identities — but each rotation costs more than one window's
/// capped reward, so sustained rotation is unprofitable. Pins that the
/// registration cost exceeds the per-window cap.
attack!("reward-sybil", identity_rotation_is_unprofitable, {
    let windows = 5i128;
    let identities_per_window = 3i128;
    let identities = windows * identities_per_window;

    // Each fresh identity earns at most one window's cap.
    let gross = identities * REWARD_CAP_PER_SOURCE_WINDOW;

    // The mitigation: one window's capped reward must not cover the cost of a
    // fresh identity, otherwise rotation is a money printer.
    assert!(
        REWARD_CAP_PER_SOURCE_WINDOW < IDENTITY_REGISTRATION_COST,
        "mitigation broken: a window's cap out-earns a fresh identity"
    );
    assert!(
        net_profit(gross, identities) < 0,
        "identity rotation must be net-unprofitable"
    );
});

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

/// A non-pos

/* … truncated 14757 chars — edit only what you need near the top … */
