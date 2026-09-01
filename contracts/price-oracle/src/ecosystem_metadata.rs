use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String, Vec};

use crate::events::{emit_admin_action, FeedMetadataRegisteredEvent, FeedMetadataUpdatedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, EcosystemMetadata, ErrorCode, FeedMetadata};

const MAX_FEED_DESCRIPTION_LENGTH: u32 = 256;

/// Registers the oracle contract in the Stellar ecosystem metadata registry.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
pub fn register_ecosystem_metadata(env: &Env, metadata: EcosystemMetadata) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .set(&DataKey::EcosystemMetadata, &metadata);
    env.storage().persistent().extend_ttl(
        &DataKey::EcosystemMetadata,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    emit_admin_action(env, symbol_short!("reg_meta"), admin, Bytes::new(env));
}

/// Updates the ecosystem metadata.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
pub fn update_ecosystem_metadata(env: &Env, metadata: EcosystemMetadata) {
    let admin = get_admin(env);
    admin.require_auth();

    env.storage()
        .persistent()
        .set(&DataKey::EcosystemMetadata, &metadata);
    env.storage().persistent().extend_ttl(
        &DataKey::EcosystemMetadata,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    emit_admin_action(env, symbol_short!("upd_meta"), admin, Bytes::new(env));
}

/// Returns the ecosystem metadata, or `None` if not registered.
pub fn get_ecosystem_metadata(env: &Env) -> Option<EcosystemMetadata> {
    let key = DataKey::EcosystemMetadata;
    env.storage().persistent().get(&key)
}

/// Registers a new price feed in the ecosystem metadata directory.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — feed description exceeds `MAX_FEED_DESCRIPTION_LENGTH`.
pub fn register_feed_metadata(env: &Env, feed: FeedMetadata) {
    let admin = get_admin(env);
    admin.require_auth();

    if feed.description.len() > MAX_FEED_DESCRIPTION_LENGTH {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let mut metadata = get_ecosystem_metadata(env).unwrap_or_else(|| EcosystemMetadata {
        contract_id: env.current_contract_address(),
        name: String::from_str(env, "Stellar Unified Price Oracle"),
        description: String::from_str(env, ""),
        version: String::from_str(env, "1.0.0"),
        feeds: Vec::new(env),
        registered_at: env.ledger().timestamp(),
    });

    let asset_key = format!("{}", feed.asset);
    let mut found = false;
    for i in 0..metadata.feeds.len() {
        let existing = metadata.feeds.get_unchecked(i);
        if existing.asset == feed.asset {
            metadata.feeds.set(i, &feed);
            found = true;
            break;
        }
    }
    if !found {
        metadata.feeds.push_back(feed.clone());
    }

    env.storage()
        .persistent()
        .set(&DataKey::EcosystemMetadata, &metadata);
    env.storage().persistent().extend_ttl(
        &DataKey::EcosystemMetadata,
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    FeedMetadataRegisteredEvent {
        asset: feed.asset.clone(),
        symbol: feed.symbol.clone(),
        description: feed.description.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("reg_feed"), admin, Bytes::new(env));
}

/// Returns all registered feed metadata.
pub fn list_feed_metadata(env: &Env) -> Vec<FeedMetadata> {
    let metadata = get_ecosystem_metadata(env);
    match metadata {
        Some(m) => m.feeds,
        None => Vec::new(env),
    }
}

/// Returns feed metadata for a specific asset, or `None`.
pub fn get_feed_metadata(env: &Env, asset: Address) -> Option<FeedMetadata> {
    let metadata = get_ecosystem_metadata(env)?;
    for i in 0..metadata.feeds.len() {
        let feed = metadata.feeds.get_unchecked(i);
        if feed.asset == asset {
            return Some(feed);
        }
    }
    None
}
