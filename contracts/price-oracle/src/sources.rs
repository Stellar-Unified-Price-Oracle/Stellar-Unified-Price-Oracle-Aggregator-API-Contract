use soroban_sdk::{panic_with_error, Address, Env, String, Vec};

use crate::events::{
    SourceAddedEvent, SourceEnrollmentStartedEvent, SourceEvidenceSubmittedEvent,
    SourceRemovedEvent,
};
use crate::storage::{get_admin, read_oracle_sources, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, OracleSources, SourceEnrollment};

pub fn add_source(env: &Env, source: Address, name: String) {
    let admin = get_admin(env);
    admin.require_auth();
    let enrollment_key = DataKey::SourceEnrollment(source.clone());
    if env.storage().persistent().has(&enrollment_key) {
        panic_with_error!(env, ErrorCode::SourceAlreadyExists);
    }
    env.storage()
        .persistent()
        .set(
            &enrollment_key,
            &SourceEnrollment {
                name,
                identity_evidence: String::from_str(env, ""),
                endpoint_ownership_evidence: String::from_str(env, ""),
                operational_contact: String::from_str(env, ""),
                reliability_evidence: String::from_str(env, ""),
            },
        );
    SourceEnrollmentStartedEvent {
        source,
        admin,
    }
    .publish(env);
}

pub fn submit_source_evidence(
    env: &Env,
    source: Address,
    identity_evidence: String,
    endpoint_ownership_evidence: String,
    operational_contact: String,
    reliability_evidence: String,
) {
    source.require_auth();
    let enrollment_key = DataKey::SourceEnrollment(source.clone());
    let mut enrollment: SourceEnrollment = env
        .storage()
        .persistent()
        .get(&enrollment_key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::SourceNotFound));
    if identity_evidence.len() == 0
        || endpoint_ownership_evidence.len() == 0
        || operational_contact.len() == 0
        || reliability_evidence.len() == 0
    {
        panic_with_error!(env, ErrorCode::SourceEvidenceMissing);
    }
    enrollment.identity_evidence = identity_evidence;
    enrollment.endpoint_ownership_evidence = endpoint_ownership_evidence;
    enrollment.operational_contact = operational_contact;
    enrollment.reliability_evidence = reliability_evidence;
    env.storage()
        .persistent()
        .set(&enrollment_key, &enrollment);
    SourceEvidenceSubmittedEvent { source }.publish(env);
}

pub fn activate_source(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    let enrollment_key = DataKey::SourceEnrollment(source.clone());
    let enrollment: SourceEnrollment = env
        .storage()
        .persistent()
        .get(&enrollment_key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::SourceNotFound));
    if enrollment.identity_evidence.len() == 0
        || enrollment.endpoint_ownership_evidence.len() == 0
        || enrollment.operational_contact.len() == 0
        || enrollment.reliability_evidence.len() == 0
    {
        panic_with_error!(env, ErrorCode::SourceEvidenceMissing);
    }
    if env
        .storage()
        .persistent()
        .get::<_, bool>(&DataKey::Source(source.clone()))
        .unwrap_or(false)
    {
        panic_with_error!(env, ErrorCode::SourceAlreadyExists);
    }

    env.storage()
        .persistent()
        .set(&DataKey::Source(source.clone()), &true);
    let mut oracle_sources: OracleSources = read_oracle_sources(env);
    oracle_sources.sources.push_back(source.clone());
    oracle_sources
        .metadata
        .set(source.clone(), enrollment.name.clone());
    env.storage()
        .persistent()
        .set(&DataKey::OracleSources, &oracle_sources);
    SourceAddedEvent {
        source,
        admin,
        name: enrollment.name,
    }
    .publish(env);
}

pub fn get_source_enrollment(env: &Env, source: Address) -> Option<SourceEnrollment> {
    let key = DataKey::SourceEnrollment(source);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

pub fn remove_source(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    let enrollment_key = DataKey::SourceEnrollment(source.clone());
    if !env.storage().persistent().has(&enrollment_key) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::Source(source.clone()));
    env.storage().persistent().remove(&enrollment_key);

    let mut oracle_sources: OracleSources = read_oracle_sources(env);
    let mut new_sources: Vec<Address> = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let s = oracle_sources.sources.get_unchecked(i);
        if s != source {
            new_sources.push_back(s);
        }
    }
    oracle_sources.sources = new_sources;
    let removed_source = source.clone();
    oracle_sources.metadata.remove(source);
    env.storage()
        .persistent()
        .set(&DataKey::OracleSources, &oracle_sources);
    SourceRemovedEvent {
        source: removed_source,
    }
    .publish(env);
}

pub fn is_source(env: &Env, source: Address) -> bool {
    let key = DataKey::Source(source.clone());
    let exists: bool = env.storage().persistent().get(&key).unwrap_or(false);
    if exists {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    exists
}

pub fn get_oracle_sources(env: &Env) -> OracleSources {
    read_oracle_sources(env)
}
