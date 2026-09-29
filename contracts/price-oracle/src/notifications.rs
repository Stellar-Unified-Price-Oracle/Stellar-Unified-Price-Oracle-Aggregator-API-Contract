//! Admin notification preference system (#243)
//!
//! Lets the admin register off-chain notification targets (webhook URLs, email
//! addresses, etc) per event type. This module only stores and exposes the
//! preferences — an off-chain relayer service watches contract events and
//! dispatches the actual notifications to the registered targets.

use soroban_sdk::{panic_with_error, Env, String, Vec};

use crate::events::{NotifPrefSetEvent, NotifPrefsClearedEvent};
use crate::storage::get_admin;
use crate::types::{DataKey, ErrorCode, NotificationPreference};

const MAX_STRING_LEN: u32 = 256;

/// Registers a notification preference for a given event type.
///
/// Idempotent: registering the same `(event_type, channel, target)` twice does
/// not create a duplicate entry.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::NotificationConfigInvalid`] — `channel` or `target` exceeds 256 chars.
pub fn set_notification_preference(env: &Env, event_type: u32, channel: String, target: String) {
    let admin = get_admin(env);
    admin.require_auth();

    if channel.len() > MAX_STRING_LEN || target.len() > MAX_STRING_LEN {
        panic_with_error!(env, ErrorCode::NotificationConfigInvalid);
    }

    let key = DataKey::NotificationPrefs(event_type);
    let mut prefs: Vec<NotificationPreference> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));

    let already_exists = prefs
        .iter()
        .any(|p| p.channel == channel && p.target == target);

    if !already_exists {
        prefs.push_back(NotificationPreference {
            event_type,
            channel: channel.clone(),
            target: target.clone(),
        });
        env.storage().persistent().set(&key, &prefs);

        let types_key = DataKey::NotificationEventTypes;
        let mut types: Vec<u32> = env
            .storage()
            .persistent()
            .get(&types_key)
            .unwrap_or(Vec::new(env));
        if !types.contains(event_type) {
            types.push_back(event_type);
            env.storage().persistent().set(&types_key, &types);
        }
    }

    NotifPrefSetEvent {
        event_type,
        channel,
        target,
    }
    .publish(env);
}

/// Returns all notification preferences registered for a specific event type.
pub fn list_notification_preferences(env: &Env, event_type: u32) -> Vec<NotificationPreference> {
    env.storage()
        .persistent()
        .get(&DataKey::NotificationPrefs(event_type))
        .unwrap_or(Vec::new(env))
}

/// Returns every event-type discriminant that currently has at least one preference set.
pub fn list_notification_event_types(env: &Env) -> Vec<u32> {
    env.storage()
        .persistent()
        .get(&DataKey::NotificationEventTypes)
        .unwrap_or(Vec::new(env))
}

/// Clears all notification preferences registered for a given event type.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
pub fn clear_notification_preferences(env: &Env, event_type: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .remove(&DataKey::NotificationPrefs(event_type));

    let types_key = DataKey::NotificationEventTypes;
    let types: Vec<u32> = env
        .storage()
        .persistent()
        .get(&types_key)
        .unwrap_or(Vec::new(env));
    let mut new_types = Vec::new(env);
    for t in types.iter() {
        if t != event_type {
            new_types.push_back(t);
        }
    }
    env.storage().persistent().set(&types_key, &new_types);

    NotifPrefsClearedEvent { event_type }.publish(env);
}
