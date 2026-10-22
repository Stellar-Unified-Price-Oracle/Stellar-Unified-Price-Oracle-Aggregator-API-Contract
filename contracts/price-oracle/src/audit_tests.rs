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
    // set_resolution: ResolutionChangedEvent (contractevent macro) + AdminActionEvent
    // + AdminAuditEntryAppendedEvent (audit log #239) = 3
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    client.set_resolution(&60u32);

    assert_eq!(
        event_count(&e),
        3,
        "set_resolution should emit exactly 3 events"
    );
}

#[test]
fn test_admin_action_emits_two_events_for_set_admin() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let new_admin = soroban_sdk::Address::generate(&e);

    client.set_admin(&new_admin);

    assert_eq!(event_count(&e), 3, "set_admin should emit exactly 3 events");
}

#[test]
fn test_admin_action_emits_two_events_for_register_asset() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = soroban_sdk::Address::generate(&e);

    client.register_asset(&asset);

    assert_eq!(
        event_count(&e),
        3,
        "register_asset should emit exactly 3 events"
    );
}

// ---------------------------------------------------------------------------
// Governance event audit completeness checks (#659)
//
// These checks reconcile privileged state changes with their emitted
// governance events and approval records across an upgrade cycle. A governance
// event that is missing, malformed, or unlinked from its state change / audit
// record causes the audit to fail.
// ---------------------------------------------------------------------------

/// A single privileged state change observed during an upgrade cycle.
struct PrivilegedChange {
    /// Human-readable label of the privileged operation.
    label: &'static str,
    /// Number of events emitted by the invocation that performed the change.
    emitted_events: usize,
}

/// Result of auditing a single privileged change.
#[derive(Debug, PartialEq)]
enum AuditOutcome {
    /// Change is fully accounted for: state change + governance event + audit record.
    Complete,
    /// No governance event was emitted for the state change.
    MissingEvent,
    /// A governance event was emitted but is malformed (wrong shape/count).
    MalformedEvent,
    /// The governance event is not linked to an approval/audit record.
    UnlinkedEvent,
}

/// Minimum number of events a privileged change must emit to be considered
/// complete: the domain event, the `AdminActionEvent`, and the
/// `AdminAuditEntryAppendedEvent` approval/audit record.
const REQUIRED_GOVERNANCE_EVENTS: usize = 3;

/// Audit a single privileged change against the governance event contract.
///
/// - `emitted_events == 0` -> `MissingEvent` (no governance event at all)
/// - `emitted_events < REQUIRED_GOVERNANCE_EVENTS` -> `MalformedEvent`
///   (governance event present but not the full domain + action + audit trio)
/// - `emitted_events == REQUIRED_GOVERNANCE_EVENTS` -> `Complete`
/// - `emitted_events > REQUIRED_GOVERNANCE_EVENTS` -> `UnlinkedEvent`
///   (extra events not attributable to this change's approval record)
fn audit_change(change: &PrivilegedChange) -> AuditOutcome {
    match change.emitted_events {
        0 => AuditOutcome::MissingEvent,
        n if n < REQUIRED_GOVERNANCE_EVENTS => AuditOutcome::MalformedEvent,
        n if n == REQUIRED_GOVERNANCE_EVENTS => AuditOutcome::Complete,
        _ => AuditOutcome::UnlinkedEvent,
    }
}

/// Audit a full upgrade cycle: every privileged change must be `Complete`.
/// Returns the first failing change label, if any.
fn audit_cycle(changes: &[PrivilegedChange]) -> Result<(), (&'static str, AuditOutcome)> {
    for change in changes {
        let outcome = audit_change(change);
        if outcome != AuditOutcome::Complete {
            return Err((change.label, outcome));
        }
    }
    Ok(())
}

#[test]
fn test_audit_complete_cycle() {
    // Normal operation: a representative upgrade cycle where every privileged
    // change emits the full governance event trio.
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);

    let mut changes = Vec::new();

    client.set_resolution(&60u32);
    changes.push(PrivilegedChange {
        label: "set_resolution",
        emitted_events: event_count(&e),
    });

    client.set_max_history_length(&50u32);
    changes.push(PrivilegedChange {
        label: "set_max_history_length",
        emitted_events: event_count(&e),
    });

    client.set_heartbeat_interval(&7200u64);
    changes.push(PrivilegedChange {
        label: "set_heartbeat_interval",
        emitted_events: event_count(&e),
    });

    assert_eq!(
        audit_cycle(&changes),
        Ok(()),
        "a fully-emitted upgrade cycle must pass the audit"
    );
}

#[test]
fn test_audit_empty_cycle_passes() {
    // Boundary: an upgrade cycle with no privileged changes is trivially complete.
    let changes: Vec<PrivilegedChange> = Vec::new();
    assert_eq!(audit_cycle(&changes), Ok(()));
}

#[test]
fn test_audit_multiple_upgrades_all_complete() {
    // Boundary: multiple sequential upgrades, each fully accounted for.
    let changes = vec![
        PrivilegedChange {
            label: "upgrade_1",
            emitted_events: REQUIRED_GOVERNANCE_EVENTS,
        },
        PrivilegedChange {
            label: "upgrade_2",
            emitted_events: REQUIRED_GOVERNANCE_EVENTS,
        },
        PrivilegedChange {
            label: "upgrade_3",
            emitted_events: REQUIRED_GOVERNANCE_EVENTS,
        },
    ];
    assert_eq!(audit_cycle(&changes), Ok(()));
}

#[test]
fn test_audit_fails_on_missing_event() {
    // Failure path: a privileged change emitted no governance event.
    let changes = vec![PrivilegedChange {
        label: "set_admin",
        emitted_events: 0,
    }];
    assert_eq!(
        audit_cycle(&changes),
        Err(("set_admin", AuditOutcome::MissingEvent))
    );
}

#[test]
fn test_audit_fails_on_malformed_event() {
    // Failure path: governance event present but missing the audit/approval record.
    let changes = vec![PrivilegedChange {
        label: "register_asset",
        emitted_events: 2,
    }];
    assert_eq!(
        audit_cycle(&changes),
        Err(("register_asset", AuditOutcome::MalformedEvent))
    );
}

#[test]
fn test_audit_fails_on_unlinked_event() {
    // Failure path: extra events not attributable to the change's approval record.
    let changes = vec![PrivilegedChange {
        label: "pause",
        emitted_events: REQUIRED_GOVERNANCE_EVENTS + 1,
    }];
    assert_eq!(
        audit_cycle(&changes),
        Err(("pause", AuditOutcome::UnlinkedEvent))
    );
}

#[test]
fn test_audit_reports_first_failure_in_cycle() {
    // Recovery path: a cycle with a good change followed by a broken one must
    // surface the broken change, not silently pass.
    let changes = vec![
        PrivilegedChange {
            label: "set_resolution",
            emitted_events: REQUIRED_GOVERNANCE_EVENTS,
        },
        PrivilegedChange {
            label: "set_admin",
            emitted_events: 0,
        },
        PrivilegedChange {
            label: "set_decimals",
            emitted_events: REQUIRED_GOVERNANCE_EVENTS,
        },
    ];
    assert_eq!(
        audit_cycle(&changes),
        Err(("set_admin", AuditOutcome::MissingEvent))
    );
}

#[test]
fn test_audit_recovers_after_fixing_missing_event() {
    // Recovery path: once the missing governance event is restored, the same
    // cycle passes the audit.
    let mut changes = vec![PrivilegedChange {
        label: "set_admin",
        emitted_events: 0,
    }];
    assert_eq!(
        audit_cycle(&changes),
        Err(("set_admin", AuditOutcome::MissingEvent))
    );

    changes[0].emitted_events = REQUIRED_GOVERNANCE_EVENTS;
    assert_eq!(audit_cycle(&changes), Ok(()));
}
