#![cfg(test)]

use soroban_sdk::{testutils::Events, Env, String};

use crate::test_helpers::*;

/// Returns the number of contract events emitted in the most recent invocation.
fn event_count(e: &Env) -> usize {
    e.events().all().events().len()
}

#[test]
fn test_snapshot_before_first_change() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    assert_eq!(client.get_config_history(&10u32).len(), 0);

    let before_resolution = client.get_resolution();
    client.set_resolution(&60u32);

    let history = client.get_config_history(&10u32);
    assert_eq!(history.len(), 1);
    let snap = history.get_unchecked(0);
    assert_eq!(snap.version, 1);
    assert_eq!(snap.resolution, before_resolution);
    assert_eq!(client.get_resolution(), 60u32);
}

#[test]
fn test_get_config_history_newest_first_and_count() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    client.set_resolution(&10u32);
    client.set_resolution(&20u32);
    client.set_resolution(&30u32);

    assert_eq!(client.get_config_history(&0u32).len(), 0);

    let all = client.get_config_history(&10u32);
    assert_eq!(all.len(), 3);
    assert_eq!(all.get_unchecked(0).version, 3);
    assert_eq!(all.get_unchecked(1).version, 2);
    assert_eq!(all.get_unchecked(2).version, 1);
    assert_eq!(all.get_unchecked(0).resolution, 20);
    assert_eq!(all.get_unchecked(1).resolution, 10);
    assert_eq!(all.get_unchecked(2).resolution, 0);

    let limited = client.get_config_history(&2u32);
    assert_eq!(limited.len(), 2);
    assert_eq!(limited.get_unchecked(0).version, 3);
    assert_eq!(limited.get_unchecked(1).version, 2);
}

#[test]
fn test_rollback_restores_exact_previous_state() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    // Capture initial live values via first mutation snapshot.
    client.set_min_sources_required(&1u32);
    client.set_max_history_length(&50u32);
    client.set_resolution(&15u32);
    client.set_decimals(&12u32);
    client.set_description(&String::from_str(&e, "Rollback Target"));
    client.set_aggregation_method(&1u32);
    client.set_timestamp_threshold(&600u64);
    client.set_max_price_deviation(&250u32);
    client.set_heartbeat_interval(&7200u64);
    client.set_max_history_per_asset(&500u32);
    client.set_max_events_per_call(&10u32);
    client.set_max_aggregation_sources(&7u32);
    client.set_aggregation_cooldown(&25u32);
    client.set_min_submission_interval(&5u32);
    client.set_interpolation_enabled(&false);
    client.set_max_sources(&40u32);
    client.set_query_rate_limit(&55u32);
    client.set_max_assets(&80u32);
    client.set_timelock_duration(&20u32);
    client.pause();

    let target_history = client.get_config_history(&1u32);
    let target = target_history.get_unchecked(0);
    let target_version = target.version;

    // Mutate away from the target state.
    client.unpause();
    client.set_resolution(&99u32);
    client.set_decimals(&8u32);
    client.set_description(&String::from_str(&e, "Mutated"));
    client.set_aggregation_method(&2u32);
    client.set_timestamp_threshold(&90u64);
    client.set_max_price_deviation(&999u32);
    client.set_heartbeat_interval(&100u64);
    client.set_max_history_per_asset(&1u32);
    client.set_max_events_per_call(&1u32);
    client.set_max_aggregation_sources(&1u32);
    client.set_aggregation_cooldown(&1u32);
    client.set_min_submission_interval(&1u32);
    client.set_interpolation_enabled(&true);
    client.set_max_sources(&1u32);
    client.set_query_rate_limit(&1u32);
    client.set_max_assets(&1u32);
    client.set_timelock_duration(&1u32);
    client.set_max_history_length(&11u32);

    client.rollback_config(&target_version);

    assert_eq!(
        client.get_min_sources_required(),
        target.min_sources_required
    );
    assert_eq!(client.get_max_history_length(), target.max_history_length);
    assert_eq!(client.get_resolution(), target.resolution);
    assert_eq!(client.get_decimals(), target.decimals);
    assert_eq!(client.get_description(), target.description);
    assert_eq!(client.get_aggregation_method(), target.aggregation_method);
    assert_eq!(client.get_timestamp_threshold(), target.timestamp_threshold);
    assert_eq!(client.get_max_price_deviation(), target.max_price_deviation);
    assert_eq!(client.get_heartbeat_interval(), target.heartbeat_interval);
    assert_eq!(
        client.get_max_history_per_asset(),
        target.max_history_per_asset
    );
    assert_eq!(client.get_max_events_per_call(), target.max_events_per_call);
    assert_eq!(
        client.get_max_aggregation_sources(),
        target.max_aggregation_sources
    );
    assert_eq!(
        client.get_aggregation_cooldown(),
        target.aggregation_cooldown
    );
    assert_eq!(
        client.get_min_submission_interval(),
        target.min_submission_interval
    );
    assert_eq!(
        client.get_interpolation_enabled(),
        target.interpolation_enabled
    );
    assert_eq!(client.get_max_sources(), target.max_sources);
    assert_eq!(client.get_query_rate_limit(), target.query_rate_limit);
    assert_eq!(client.get_max_assets(), target.max_assets);
    assert_eq!(client.is_paused(), target.paused);
    assert_eq!(client.get_timelock_duration(), target.timelock_duration);
}

#[test]
fn test_rollback_snapshots_current_first() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    client.set_resolution(&10u32); // v1 = init (0)
    client.set_resolution(&20u32); // v2 = 10

    let before_rollback = client.get_resolution();
    assert_eq!(before_rollback, 20u32);

    client.rollback_config(&1u32); // restore init resolution 0; also snapshot current as v3

    assert_eq!(client.get_resolution(), 0u32);
    let history = client.get_config_history(&5u32);
    assert!(history.len() >= 3);
    let newest = history.get_unchecked(0);
    assert_eq!(newest.resolution, before_rollback);
    assert_eq!(newest.version, 3);
}

#[test]
fn test_canonical_threshold_and_deviation_roundtrip() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    client.set_timestamp_threshold(&450u64);
    client.set_max_price_deviation(&333u32);
    assert_eq!(client.get_timestamp_threshold(), 450u64);
    assert_eq!(client.get_max_price_deviation(), 333u32);

    let history = client.get_config_history(&1u32);
    let snap = history.get_unchecked(0);
    // Newest snapshot was taken before the deviation write, so threshold is already 450.
    assert_eq!(snap.timestamp_threshold, 450u64);

    client.rollback_config(&snap.version);
    assert_eq!(client.get_timestamp_threshold(), 450u64);
    // Pre-deviation snapshot restores default deviation.
    assert_eq!(client.get_max_price_deviation(), 500u32);
}

#[test]
fn test_config_history_prunes_beyond_100() {
    let e = Env::default();
    e.cost_estimate().disable_resource_limits();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    for i in 1..=101u32 {
        client.set_resolution(&i);
    }

    let history = client.get_config_history(&200u32);
    assert_eq!(history.len(), 100);
    assert_eq!(history.get_unchecked(0).version, 101);
    assert_eq!(history.get_unchecked(99).version, 2);
}

#[test]
#[should_panic(expected = "Error(Contract, #132)")]
fn test_rollback_unknown_version_panics() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);
    client.set_resolution(&5u32);
    client.rollback_config(&999u32);
}

#[test]
#[should_panic(expected = "Error(Contract, #132)")]
fn test_rollback_pruned_version_panics() {
    let e = Env::default();
    e.cost_estimate().disable_resource_limits();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    for i in 1..=101u32 {
        client.set_resolution(&i);
    }
    // Version 1 was pruned.
    client.rollback_config(&1u32);
}

#[test]
#[should_panic]
fn test_rollback_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    client.set_resolution(&5u32);
    clear_auth(&e);
    client.rollback_config(&1u32);
}

#[test]
fn test_snapshot_and_rollback_events() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    client.set_resolution(&42u32);
    // ConfigSnapshotTakenEvent + ResolutionChangedEvent + AdminActionEvent = 3
    assert!(
        event_count(&e) >= 3,
        "set_resolution should also emit a config snapshot event"
    );

    client.rollback_config(&1u32);
    // ConfigSnapshotTakenEvent + ConfigRolledBackEvent + AdminActionEvent = 3
    assert!(
        event_count(&e) >= 3,
        "rollback_config should emit snapshot + rollback + admin action events"
    );
}

#[test]
fn test_pause_is_included_in_snapshot() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    assert!(!client.is_paused());
    client.pause();
    assert!(client.is_paused());

    let history = client.get_config_history(&1u32);
    assert!(!history.get_unchecked(0).paused);

    client.unpause();
    client.rollback_config(&1u32);
    assert!(!client.is_paused());
}

#[test]
fn test_multiple_parameter_types_snapshot() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, _) = setup_contract(&e);

    client.set_decimals(&10u32);
    client.set_heartbeat_interval(&1800u64);
    client.set_interpolation_enabled(&false);

    let history = client.get_config_history(&3u32);
    assert_eq!(history.len(), 3);
    assert_eq!(history.get_unchecked(0).version, 3);
    assert_eq!(history.get_unchecked(0).heartbeat_interval, 1800);
    assert_eq!(history.get_unchecked(1).decimals, 10);
}
