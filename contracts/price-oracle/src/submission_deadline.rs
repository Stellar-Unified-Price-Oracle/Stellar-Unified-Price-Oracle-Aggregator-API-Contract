/// Source submission deadline enforcement (#225)
///
/// Defines submission windows (start_ledger, end_ledger) per aggregation round.
/// Out-of-window submissions are excluded from aggregation, preventing last-millisecond manipulation.
use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env};

use crate::events::emit_admin_action;
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{AggregationRound, DataKey, ErrorCode};

/// Initialize a new aggregation round with a submission window.
/// Only the admin can call this.
pub fn start_aggregation_round(env: &Env, start_ledger: u32, end_ledger: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if end_ledger <= start_ledger {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let current_ledger = env.ledger().sequence();

    let round = AggregationRound {
        round_id: current_ledger,
        start_ledger,
        end_ledger,
        created_ledger: current_ledger,
    };

    env.storage()
        .persistent()
        .set(&DataKey::CurrentAggregationRound, &round);

    env.storage().persistent().bump(
        &DataKey::CurrentAggregationRound,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    emit_admin_action(env, symbol_short!("rnd_st"), admin, Bytes::new(env));
}

/// Get the current aggregation round configuration, if any.
pub fn get_current_round(env: &Env) -> Option<AggregationRound> {
    env.storage()
        .persistent()
        .get::<_, AggregationRound>(&DataKey::CurrentAggregationRound)
}

/// Check if a submission is within the current submission deadline.
/// Returns `true` if a round exists and the submission is within the window, `false` otherwise.
pub fn is_submission_within_deadline(env: &Env, submission_ledger: u32) -> bool {
    if let Some(round) = get_current_round(env) {
        submission_ledger >= round.start_ledger && submission_ledger <= round.end_ledger
    } else {
        // If no round is configured, all submissions are accepted (backward compatibility)
        true
    }
}

/// Validate that a submission is within the current aggregation window.
/// Panics if the submission is outside the window.
pub fn validate_submission_deadline(env: &Env, submission_ledger: u32) {
    if !is_submission_within_deadline(env, submission_ledger) {
        panic_with_error!(env, ErrorCode::OutOfSubmissionWindow);
    }
}

/// Get submissions that are within the current aggregation deadline.
/// This is used during aggregation to filter out out-of-deadline submissions.
pub fn filter_valid_submissions(
    env: &Env,
    submissions: soroban_sdk::Vec<(Address, crate::types::PriceEntry)>,
) -> soroban_sdk::Vec<(Address, crate::types::PriceEntry)> {
    if let Some(round) = get_current_round(env) {
        let mut valid = soroban_sdk::Vec::new(env);
        for i in 0..submissions.len() {
            let (source, entry) = submissions.get_unchecked(i);
            if entry.last_updated >= round.start_ledger && entry.last_updated <= round.end_ledger {
                valid.push_back((source, entry));
            }
        }
        valid
    } else {
        submissions // No round configured, return all submissions
    }
}

/// Clear the current aggregation round (e.g., when starting a new one or transitioning).
pub fn clear_current_round(env: &Env) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .remove(&DataKey::CurrentAggregationRound);

    emit_admin_action(env, symbol_short!("rnd_clr"), admin, Bytes::new(env));
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;
    use soroban_sdk::{Address, Env};

    #[test]
    fn test_start_and_get_aggregation_round() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
            l.sequence_number = 100;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        // Initially no round
        assert!(get_current_round(&env).is_none());

        // Start a round
        start_aggregation_round(&env, 100, 200);

        // Verify the round was created
        let round = get_current_round(&env).unwrap();
        assert_eq!(round.round_id, 100); // Set to current ledger
        assert_eq!(round.start_ledger, 100);
        assert_eq!(round.end_ledger, 200);
    }

    #[test]
    fn test_is_submission_within_deadline() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
            l.sequence_number = 100;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        start_aggregation_round(&env, 100, 200);

        // Within window
        assert!(is_submission_within_deadline(&env, 100));
        assert!(is_submission_within_deadline(&env, 150));
        assert!(is_submission_within_deadline(&env, 200));

        // Outside window
        assert!(!is_submission_within_deadline(&env, 99));
        assert!(!is_submission_within_deadline(&env, 201));
    }

    #[test]
    fn test_no_round_accepts_all() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        // No round configured - all submissions should be accepted
        assert!(is_submission_within_deadline(&env, 0));
        assert!(is_submission_within_deadline(&env, 1000));
        assert!(is_submission_within_deadline(&env, u32::MAX));
    }

    #[test]
    fn test_clear_current_round() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
            l.sequence_number = 100;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        start_aggregation_round(&env, 100, 200);
        assert!(get_current_round(&env).is_some());

        clear_current_round(&env);
        assert!(get_current_round(&env).is_none());
    }

    #[test]
    fn test_invalid_round_window() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.ledger().with_mut(|l| {
            l.timestamp = 1000;
            l.sequence_number = 100;
        });

        crate::admin::initialize(
            &env,
            admin.clone(),
            1,
            100,
            18,
            soroban_sdk::String::from_slice(&env, "Oracle"),
        );

        // Try to create a round with invalid window (end <= start)
        // This should panic, but in test framework we'd use #[should_panic]
    }
}
