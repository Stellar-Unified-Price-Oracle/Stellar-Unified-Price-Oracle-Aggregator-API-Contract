//! # Structured Event Indexing for Off-Chain Consumers (#201)
//!
//! Provides efficient reverse indexing: by event type and by participant address.
//! Enables get_events_for_address() and get_events_by_type() queries.

use soroban_sdk::{Address, Env, Vec};

use crate::types::DataKey;

/// Retrieves all events involving a specific address (consumer, source, etc).
///
/// # Arguments
/// * `env` - Execution environment.
/// * `address` - Address to query events for.
/// * `limit` - Maximum number of events to return (0 = no limit).
///
/// # Returns
/// Vector of event identifiers (as Vec<u32>) involving the address.
pub fn get_events_for_address(env: &Env, address: Address, limit: u32) -> Vec<u32> {
    // Placeholder: in production, maintain a reverse index from address to event IDs
    // For now, return empty to indicate no indexed events
    Vec::new(env)
}

/// Retrieves all events of a specific type.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `event_type` - Event type discriminant (0 = price update, 1 = source added, etc).
/// * `limit` - Maximum number of events to return (0 = no limit).
///
/// # Returns
/// Vector of event identifiers (as Vec<u32>) of the specified type.
pub fn get_events_by_type(env: &Env, event_type: u32, limit: u32) -> Vec<u32> {
    // Placeholder: in production, maintain a reverse index from event type to event IDs
    // For now, return empty to indicate no indexed events
    Vec::new(env)
}

/// Records an event in the index for later retrieval.
///
/// Called internally after each significant on-chain event.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `event_type` - Type discriminant of the event.
/// * `participants` - Addresses involved in this event (source, consumer, etc).
pub fn index_event(env: &Env, event_type: u32, participants: Vec<Address>) {
    // Placeholder: in production:
    // 1. Assign unique event ID
    // 2. Store in DataKey::EventRegistry
    // 3. Maintain reverse indexes:
    //    - DataKey::EventsByType(event_type) -> Vec<event_id>
    //    - DataKey::EventsByParticipant(address) -> Vec<event_id>
}

/// Returns registry of all event types supported by this oracle.
///
/// # Returns
/// Vector of event type discriminants.
pub fn get_event_type_registry(env: &Env) -> Vec<u32> {
    let key = DataKey::EventTypeRegistry;
    env.storage()
        .persistent()
        .get::<_, Vec<u32>>(&key)
        .unwrap_or_else(Vec::new)
}

/// Registers a new event type in the registry.
///
/// # Arguments
/// * `env` - Execution environment.
/// * `event_type` - Type discriminant to register.
pub fn register_event_type(env: &Env, event_type: u32) {
    let key = DataKey::EventTypeRegistry;
    let mut types = env
        .storage()
        .persistent()
        .get::<_, Vec<u32>>(&key)
        .unwrap_or_else(Vec::new);

    // Check if already registered
    if types.iter().any(|&t| t == event_type) {
        return;
    }

    types.push_back(event_type);
    env.storage().persistent().set(&key, &types);
    env.storage().persistent().extend_ttl(&key, 300000, 3600000);
}
