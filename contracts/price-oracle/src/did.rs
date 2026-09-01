use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String, Vec};

use crate::events::{
    emit_admin_action, DidRegisteredEvent, DidVerifiedEvent, SourceDidLinkedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, DidDocument, DidVerification, ErrorCode, SourceDidLink};

const MAX_DID_DOCUMENT_LENGTH: u32 = 4096;

/// Registers a new DID document under `did_address`.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
/// * [`ErrorCode::InvalidConfiguration`] — document exceeds `MAX_DID_DOCUMENT_LENGTH`.
pub fn register_did(env: &Env, did_address: Address, document: String) {
    let admin = get_admin(env);
    admin.require_auth();

    if document.len() > MAX_DID_DOCUMENT_LENGTH {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    let key = DataKey::DidDocument(did_address.clone());
    if env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::AlreadyInitialized);
    }

    env.storage().persistent().set(&key, &document);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    DidRegisteredEvent {
        did: did_address.clone(),
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("reg_did"), admin, Bytes::new(env));
}

/// Links an oracle source address to a DID address for identity verification.
///
/// # Panics
///
/// * [`ErrorCode::NotAuthorized`] — caller is not admin.
pub fn link_source_did(env: &Env, source: Address, did: Address, verified: bool) {
    let admin = get_admin(env);
    admin.require_auth();

    let link = SourceDidLink {
        source: source.clone(),
        did: did.clone(),
        verified,
        verified_at: env.ledger().timestamp(),
    };

    env.storage()
        .persistent()
        .set(&DataKey::SourceDid(source.clone()), &link);
    env.storage().persistent().extend_ttl(
        &DataKey::SourceDid(source.clone()),
        LEDGER_THRESHOLD,
        LEDGER_BUMP,
    );

    SourceDidLinkedEvent {
        source: source.clone(),
        did: did.clone(),
        verified,
    }
    .publish(env);
}

/// Verifies a DID document by checking its presence on-chain.
///
/// Returns `true` if the DID document exists, `false` otherwise.
pub fn verify_did(env: &Env, did_address: Address) -> bool {
    let key = DataKey::DidDocument(did_address.clone());
    let exists: bool = env.storage().persistent().has(&key);
    if exists {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    exists
}

/// Returns the DID document for a given DID address, or `None` if not registered.
pub fn get_did_document(env: &Env, did_address: Address) -> Option<String> {
    let key = DataKey::DidDocument(did_address);
    env.storage().persistent().get(&key)
}

/// Returns the DID link for a source, or `None` if not linked.
pub fn get_source_did(env: &Env, source: Address) -> Option<SourceDidLink> {
    let key = DataKey::SourceDid(source);
    env.storage().persistent().get(&key)
}

/// Returns all source-DID links stored on-chain.
pub fn get_all_source_dids(env: &Env) -> Vec<SourceDidLink> {
    let registry_key = DataKey::SrcRegistry;
    let oracle_sources: crate::types::OracleSources = env
        .storage()
        .persistent()
        .get(&registry_key)
        .unwrap_or_else(|| crate::types::OracleSources {
            sources: Vec::new(env),
            metadata: Map::new(env),
        });

    let mut links = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let source = oracle_sources.sources.get_unchecked(i);
        if let Some(link) = get_source_did(env, source.clone()) {
            links.push_back(link);
        }
    }
    links
}
