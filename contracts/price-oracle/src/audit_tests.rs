#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events},
    Env, String,
};

use crate::test_helpers::*;

/// Returns the number of contract events emitted in the most recent invocation.
/// In Soroban test environments, `events().all()` reflects the current
/// invocation's events — prior invocations' events are replaced on each call.
fn event_count(e: &Env) -> usize {
    e.events().all().events().len()
}

#[test]
fn test_admin_action_on_initialize() {
    let e = Env::default();
    e.mock_all_auths();
    let admin = soroban_sdk::Address::generate(&e);
    let client = create_contract(&e);

    client.initialize(&admin, &1u32, &10u32, &18u32, &String::from_str(&e, "Test"));

    // emit_initialized + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "initialize should emit at least 2 events (init event + AdminActionEvent)"
    );
}

#[test]
fn test_admin_action_on_set_admin() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let new_admin = soroban_sdk::Address::generate(&e);

    client.set_admin(&new_admin);

    // AdminChangedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_admin should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_add_source() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let source = soroban_sdk::Address::generate(&e);

    client.add_source(&source, &String::from_str(&e, "TestSource"));

    // SourceAddedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "add_source should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_remove_source() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let source = register_test_source(&e, &client, "TestSource");

    client.remove_source(&source);

    // SourceRemovedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "remove_source should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_register_asset() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = soroban_sdk::Address::generate(&e);

    client.register_asset(&asset);

    // AssetRegisteredEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "register_asset should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_unregister_asset() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    client.unregister_asset(&asset);

    // AssetUnregisteredEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "unregister_asset should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_min_sources() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    register_test_source(&e, &client, "S1");
    register_test_source(&e, &client, "S2");

    client.set_min_sources_required(&2u32);

    // MinSourcesChangedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_min_sources_required should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_max_history() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_max_history_length(&50u32);

    // MaxHistoryChangedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_max_history_length should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_resolution() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_resolution(&60u32);

    // ResolutionChangedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_resolution should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_decimals() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_decimals(&8u32);

    // DecimalsChangedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_decimals should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_description() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_description(&String::from_str(&e, "New Description"));

    // DescriptionChangedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_description should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_timestamp_threshold() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_timestamp_threshold(&600u64);

    // timestamp_threshold_changed + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_timestamp_threshold should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_max_price_deviation() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_max_price_deviation(&1000u32);

    // max_price_deviation_changed + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_max_price_deviation should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_set_heartbeat_interval() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_heartbeat_interval(&7200u64);

    // HeartbeatIntervalChangedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "set_heartbeat_interval should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_on_pause() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.pause();

    // ContractPausedEvent + AdminActionEvent = 2
    assert!(event_count(&e) >= 2, "pause should emit at least 2 events");
}

#[test]
fn test_admin_action_on_unpause() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.pause();

    client.unpause();

    // ContractUnpausedEvent + AdminActionEvent = 2
    assert!(
        event_count(&e) >= 2,
        "unpause should emit at least 2 events"
    );
}

#[test]
fn test_admin_action_emits_two_events_per_call() {
    // set_resolution: ResolutionChangedEvent (contractevent macro) + AdminActionEvent = 2
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_resolution(&60u32);

    assert_eq!(
        event_count(&e),
        2,
        "set_resolution should emit exactly 2 events"
    );
}

#[test]
fn test_admin_action_emits_two_events_for_set_admin() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let new_admin = soroban_sdk::Address::generate(&e);

    client.set_admin(&new_admin);

    assert_eq!(event_count(&e), 2, "set_admin should emit exactly 2 events");
}

#[test]
fn test_admin_action_emits_two_events_for_register_asset() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = soroban_sdk::Address::generate(&e);

    client.register_asset(&asset);

    assert_eq!(
        event_count(&e),
        2,
        "register_asset should emit exactly 2 events"
    );
}
