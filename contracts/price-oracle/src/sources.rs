use soroban_sdk::{panic_with_error, symbol_short, Address, Bytes, Env, String, Vec};

use crate::events::{
    emit_admin_action, RemovalCooldownChangedEvent, SourceActiveAgainEvent, SourceAddedEvent,
    SourceHeartbeatEvent, SourceInactiveEvent, SourceMarkedForRemovalEvent,
    SourceRemovalCancelledEvent, SourceRemovedEvent,
    SourceWarningEvent, SourceProbationEvent, SourceDisqualifiedEvent, SourceDemeritsResetEvent,
    DemeritConfigChangedEvent, InvalidSubmissionEvent,
    SourceGovConfigChangedEvent, SourceProposalCreatedEvent, SourceProposalApprovedEvent, SourceProposalExecutedEvent,
    SourceGeoUpdatedEvent,
    SourceBondConfigChangedEvent, SourceBondDepositedEvent, SourceBondForfeitedEvent, SourceBondReturnedEvent,
    SourceAssetAddedEvent, SourceAssetRemovedEvent, SourceKeyRotatedEvent,
    SourceVerificationSetEvent,
};
use crate::storage::{
    get_admin, is_source_inactive as check_source_inactive, mark_source_active,
    mark_source_inactive, read_oracle_sources, LEDGER_BUMP, LEDGER_THRESHOLD,
};
use crate::types::{
    DataKey, ErrorCode, OracleSources, DisqualificationStatus, SourceDemeritState, DemeritConfig,
    SourceGovernance, SourceProposal, SourceGeoMetadata, DecentralizationReport,
    SourceVerification,
};




const MAX_SOURCE_NAME_LENGTH: u32 = 64;
const SOURCE_ROTATION_COOLDOWN: u32 = 100;

fn register_source_internal(env: &Env, source: Address, name: String) {
    if name.is_empty() {
        panic_with_error!(env, ErrorCode::SourceNameEmpty);
    }
    if name.len() > MAX_SOURCE_NAME_LENGTH {
        panic_with_error!(env, ErrorCode::SourceNameTooLong);
    }
    if env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceAlreadyExists);
    }

    let oracle_sources: OracleSources = read_oracle_sources(env);
    let max_sources = crate::admin::get_max_sources(env);
    if max_sources > 0 && oracle_sources.sources.len() >= max_sources {
        panic_with_error!(env, ErrorCode::MaxSourcesReached);
    }

    env.storage()
        .persistent()
        .set(&DataKey::SrcActive(source.clone()), &true);

    let mut oracle_sources: OracleSources = oracle_sources;
    oracle_sources.sources.push_back(source.clone());
    let source_name = name.clone();
    oracle_sources.metadata.set(source.clone(), name);
    env.storage()
        .persistent()
        .set(&DataKey::SrcRegistry, &oracle_sources);
    SourceAddedEvent {
        source: source.clone(),
        admin: get_admin(env),
        name: source_name,
    }
    .publish(env);
}

fn contains_address(items: &Vec<Address>, needle: &Address) -> bool {
    for i in 0..items.len() {
        if items.get_unchecked(i) == *needle {
            return true;
        }
    }
    false
}

pub fn add_source(env: &Env, source: Address, name: String) {
    let admin = get_admin(env);
    admin.require_auth();

    if let Some(gov) = get_source_governance(env) {
        if gov.threshold > 0 {
            panic_with_error!(env, ErrorCode::NotAuthorized);
        }
    }

    register_source_internal(env, source, name);
    emit_admin_action(env, symbol_short!("add_src"), admin, Bytes::new(env));
}

pub fn add_source_with_assets(env: &Env, source: Address, name: String, assets: Vec<Address>) {
    let admin = get_admin(env);
    admin.require_auth();
    for i in 0..assets.len() {
        crate::storage::check_registered_asset(env, &assets.get_unchecked(i));
    }
    register_source_internal(env, source.clone(), name);
    env.storage()
        .persistent()
        .set(&DataKey::SourceAssets(source.clone()), &assets);
    for i in 0..assets.len() {
        SourceAssetAddedEvent {
            source: source.clone(),
            asset: assets.get_unchecked(i),
        }
        .publish(env);
    }
    emit_admin_action(env, symbol_short!("add_srca"), admin, Bytes::new(env));
}


pub fn remove_source(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    let is_inactive = is_source_inactive(env, source.clone());
    let deposited = get_source_deposited_bond(env, source.clone());
    if deposited > 0 {
        if !is_inactive {
            if let Some(token_addr) = crate::reputation::get_stake_token_contract(env) {
                let client = soroban_sdk::token::Client::new(env, &token_addr);
                let _ = client.try_transfer(&env.current_contract_address(), &source, &deposited);
                SourceBondReturnedEvent {
                    source: source.clone(),
                    amount: deposited,
                }
                .publish(env);
            }
        } else {
            forfeit_source_bond_internal(env, source.clone());
        }
        env.storage().persistent().remove(&DataKey::SourceBond(source.clone()));
    }

    env.storage()
        .persistent()
        .remove(&DataKey::SrcActive(source.clone()));

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
    oracle_sources.verification.remove(removed_source.clone());
    env.storage()
        .persistent()
        .set(&DataKey::SrcRegistry, &oracle_sources);
    env.storage()
        .persistent()
        .remove(&DataKey::SourceAssets(removed_source.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::SourceVerification(removed_source.clone()));
    SourceRemovedEvent {
        source: removed_source,
        admin: admin.clone(),
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("rem_src"), admin, Bytes::new(env));
}


pub fn is_source(env: &Env, source: Address) -> bool {
    let key = DataKey::SrcActive(source.clone());
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

pub fn get_source_assets(env: &Env, source: Address) -> Vec<Address> {
    if !is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    let key = DataKey::SourceAssets(source);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

pub fn add_source_asset(env: &Env, source: Address, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    crate::storage::check_registered_asset(env, &asset);
    let key = DataKey::SourceAssets(source.clone());
    let mut assets: Vec<Address> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    if !contains_address(&assets, &asset) {
        assets.push_back(asset.clone());
        env.storage().persistent().set(&key, &assets);
        SourceAssetAddedEvent {
            source: source.clone(),
            asset,
        }
        .publish(env);
    }
    emit_admin_action(env, symbol_short!("add_sass"), admin, Bytes::new(env));
}

pub fn remove_source_asset(env: &Env, source: Address, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    let key = DataKey::SourceAssets(source.clone());
    let assets: Vec<Address> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env));
    let mut next: Vec<Address> = Vec::new(env);
    let mut removed = false;
    for i in 0..assets.len() {
        let current = assets.get_unchecked(i);
        if current == asset {
            removed = true;
        } else {
            next.push_back(current);
        }
    }
    env.storage().persistent().set(&key, &next);
    if removed {
        SourceAssetRemovedEvent {
            source: source.clone(),
            asset,
        }
        .publish(env);
    }
    emit_admin_action(env, symbol_short!("rem_sass"), admin, Bytes::new(env));
}

pub fn set_source_verification(
    env: &Env,
    source: Address,
    verified: bool,
    verification_method: String,
    verifier: Address,
) {
    let admin = get_admin(env);
    admin.require_auth();
    if !is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    let verification = SourceVerification {
        verified,
        verification_method: verification_method.clone(),
        verifier: verifier.clone(),
    };
    env.storage()
        .persistent()
        .set(&DataKey::SourceVerification(source.clone()), &verification);
    let mut oracle_sources = read_oracle_sources(env);
    oracle_sources
        .verification
        .set(source.clone(), verification);
    env.storage()
        .persistent()
        .set(&DataKey::SrcRegistry, &oracle_sources);
    SourceVerificationSetEvent {
        source,
        verified,
        verification_method,
        verifier,
    }
    .publish(env);
    emit_admin_action(env, symbol_short!("set_ver"), admin, Bytes::new(env));
}

pub fn get_source_verification(env: &Env, source: Address) -> Option<SourceVerification> {
    let key = DataKey::SourceVerification(source);
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key)
}

pub fn rotate_source_key(env: &Env, source: Address, new_address: Address) {
    source.require_auth();
    new_address.require_auth();
    if !is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    if is_source(env, new_address.clone()) {
        panic_with_error!(env, ErrorCode::SourceAlreadyExists);
    }
    let current_ledger = env.ledger().sequence();
    let last_rotation: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::SourceRotationLedger(source.clone()))
        .unwrap_or(0);
    if last_rotation > 0
        && current_ledger.saturating_sub(last_rotation) < SOURCE_ROTATION_COOLDOWN
    {
        panic_with_error!(env, ErrorCode::CooldownNotElapsed);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::SrcActive(source.clone()));
    env.storage()
        .persistent()
        .set(&DataKey::SrcActive(new_address.clone()), &true);

    let mut oracle_sources = read_oracle_sources(env);
    for i in 0..oracle_sources.sources.len() {
        if oracle_sources.sources.get_unchecked(i) == source {
            oracle_sources.sources.set(i, new_address.clone());
            break;
        }
    }
    if let Some(name) = oracle_sources.metadata.get(source.clone()) {
        oracle_sources.metadata.remove(source.clone());
        oracle_sources.metadata.set(new_address.clone(), name);
    }
    if let Some(verification) = oracle_sources.verification.get(source.clone()) {
        oracle_sources.verification.remove(source.clone());
        oracle_sources
            .verification
            .set(new_address.clone(), verification.clone());
        env.storage()
            .persistent()
            .remove(&DataKey::SourceVerification(source.clone()));
        env.storage()
            .persistent()
            .set(&DataKey::SourceVerification(new_address.clone()), &verification);
    }
    env.storage()
        .persistent()
        .set(&DataKey::SrcRegistry, &oracle_sources);

    if let Some(assets) = env
        .storage()
        .persistent()
        .get::<_, Vec<Address>>(&DataKey::SourceAssets(source.clone()))
    {
        env.storage()
            .persistent()
            .remove(&DataKey::SourceAssets(source.clone()));
        env.storage()
            .persistent()
            .set(&DataKey::SourceAssets(new_address.clone()), &assets);
    }

    let registered_assets = crate::storage::read_registered_assets(env);
    for i in 0..registered_assets.len() {
        let asset = registered_assets.get_unchecked(i);
        let old_key = DataKey::Submission(asset.clone(), source.clone());
        if let Some(mut entry) = env.storage().persistent().get::<_, crate::types::PriceEntry>(&old_key) {
            entry.source = new_address.clone();
            env.storage().persistent().remove(&old_key);
            env.storage()
                .persistent()
                .set(&DataKey::Submission(asset.clone(), new_address.clone()), &entry);
        }
    }

    env.storage().persistent().set(
        &DataKey::SourceRotationLedger(new_address.clone()),
        &current_ledger,
    );
    SourceKeyRotatedEvent {
        old_source: source,
        new_source: new_address,
        ledger: current_ledger,
    }
    .publish(env);
}

pub fn submit_heartbeat(env: &Env, source: Address) {
    source.require_auth();
    if !is_source(env, source.clone()) {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    let timestamp = env.ledger().timestamp();
    let current_ledger = env.ledger().sequence();

    // Snapshot the old health status for the HealthChanged event.
    let old_health = get_source_health(env, source.clone());
    let old_status = old_health as u32;

    env.storage()
        .persistent()
        .set(&DataKey::SrcHeartbeat(source.clone()), &timestamp);

    let was_inactive = check_source_inactive(env, &source);
    if was_inactive {
        // Reactivation requires BOTH a heartbeat AND a price submission in the same or
        // adjacent ledger. Check whether the source has submitted a price after the last
        // reactivation attempt.
        let price_submitted_key = DataKey::SrcPriceSubmitAfterReactivation(source.clone());
        let has_price: bool = env
            .storage()
            .persistent()
            .get(&price_submitted_key)
            .unwrap_or(false);

        if has_price {
            // Full reactivation: both conditions met.
            mark_source_active(env, &source);
            reset_missed_heartbeats(env, &source);
            env.storage().persistent().remove(&price_submitted_key);
            env.storage()
                .persistent()
                .remove(&DataKey::SrcInactiveSinceLedger(source.clone()));

            crate::events::SourceHealthChangedEvent {
                source: source.clone(),
                old_status,
                new_status: 0, // Healthy
                missed_heartbeats: 0,
            }
            .publish(env);

            SourceActiveAgainEvent {
                source: source.clone(),
                timestamp,
            }
            .publish(env);
        }
        // If no price has been submitted yet, the heartbeat is recorded but the source
        // stays inactive — caller must also submit a price before reactivation completes.
    } else {
        // Source was active: reset missed count and record heartbeat.
        reset_missed_heartbeats(env, &source);

        let new_status = get_source_health(env, source.clone()) as u32;
        if old_status != new_status {
            crate::events::SourceHealthChangedEvent {
                source: source.clone(),
                old_status,
                new_status,
                missed_heartbeats: 0,
            }
            .publish(env);
        }
    }

    // Update the last-heartbeat ledger for adaptive interval computation.
    env.storage().persistent().set(
        &DataKey::SrcLastPriceLedger(source.clone()),
        &current_ledger,
    );

    SourceHeartbeatEvent {
        source: source.clone(),
        timestamp,
    }
    .publish(env);
}

pub fn is_source_inactive(env: &Env, source: Address) -> bool {
    // Fast path: already explicitly marked inactive.
    let is_marked = check_source_inactive(env, &source);
    if is_marked {
        return true;
    }

    // Check adaptive heartbeat timeout.
    let key = DataKey::SrcHeartbeat(source.clone());
    let last_heartbeat: Option<u64> = env.storage().persistent().get(&key);

    let base_interval = crate::admin::get_heartbeat_interval(env);
    let current_time = env.ledger().timestamp();
    let current_ledger = env.ledger().sequence();
    let window = crate::sources::get_heartbeat_window(env);

    if let Some(hb_time) = last_heartbeat {
        let missed = get_missed_heartbeats(env, &source);
        let adaptive = compute_adaptive_interval(base_interval, missed, window);

        if current_time > hb_time.saturating_add(adaptive) {
            let new_missed = increment_missed_heartbeats(env, &source);

            if new_missed >= MISS_THRESHOLD {
                // Cross the inactivity threshold.
                let old_status = if new_missed == MISS_THRESHOLD {
                    1u32
                } else {
                    2u32
                };
                mark_source_inactive(env, &source);
                forfeit_source_bond_internal(env, source.clone());


                // Record when inactivity started (only on first trip).
                let inactive_since_key = DataKey::SrcInactiveSinceLedger(source.clone());
                if !env.storage().persistent().has(&inactive_since_key) {
                    env.storage()
                        .persistent()
                        .set(&inactive_since_key, &current_ledger);
                }

                crate::events::SourceHealthChangedEvent {
                    source: source.clone(),
                    old_status,
                    new_status: 2, // Inactive
                    missed_heartbeats: new_missed,
                }
                .publish(env);

                SourceInactiveEvent {
                    source: source.clone(),
                    last_heartbeat: hb_time,
                }
                .publish(env);
                return true;
            }

            // Below threshold: Degraded.
            crate::events::SourceHealthChangedEvent {
                source: source.clone(),
                old_status: if new_missed > 1 { 1u32 } else { 0u32 },
                new_status: 1, // Degraded
                missed_heartbeats: new_missed,
            }
            .publish(env);
        }
    } else {
        // No heartbeat on record.
        if current_time > base_interval {
            let new_missed = increment_missed_heartbeats(env, &source);
            if new_missed >= MISS_THRESHOLD {
                mark_source_inactive(env, &source);
                let inactive_since_key = DataKey::SrcInactiveSinceLedger(source.clone());
                if !env.storage().persistent().has(&inactive_since_key) {
                    env.storage()
                        .persistent()
                        .set(&inactive_since_key, &current_ledger);
                }
                SourceInactiveEvent {
                    source: source.clone(),
                    last_heartbeat: 0,
                }
                .publish(env);
                return true;
            }
        }
    }

    false
}

pub fn get_inactive_sources(env: &Env) -> u32 {
    let oracle_sources = read_oracle_sources(env);
    let mut count: u32 = 0;

    for i in 0..oracle_sources.sources.len() {
        let source = oracle_sources.sources.get_unchecked(i);
        if is_source_inactive(env, source) {
            count += 1;
        }
    }

    count
}

pub fn get_demerit_config(env: &Env) -> DemeritConfig {
    env.storage()
        .persistent()
        .get(&DataKey::DemeritConfig)
        .unwrap_or(DemeritConfig {
            warning_threshold: 2,
            probation_threshold: 5,
            disqualified_threshold: 10,
            cooldown_ledgers: 100,
        })
}

pub fn set_demerit_config(env: &Env, config: DemeritConfig) {
    let admin = get_admin(env);
    admin.require_auth();

    if config.warning_threshold > config.probation_threshold
        || config.probation_threshold > config.disqualified_threshold
    {
        panic_with_error!(env, ErrorCode::InvalidDemeritThreshold);
    }

    env.storage()
        .persistent()
        .set(&DataKey::DemeritConfig, &config);

    DemeritConfigChangedEvent {
        admin,
        warning_threshold: config.warning_threshold,
        probation_threshold: config.probation_threshold,
        disqualified_threshold: config.disqualified_threshold,
        cooldown_ledgers: config.cooldown_ledgers,
    }
    .publish(env);
}

pub fn get_source_demerits(env: &Env, source: Address) -> SourceDemeritState {
    let key = DataKey::SourceDemerits(source.clone());
    let mut state: SourceDemeritState = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or(SourceDemeritState {
            demerits: 0,
            status: DisqualificationStatus::Active,
            status_updated_ledger: 0,
        });

    if state.status == DisqualificationStatus::Disqualified {
        let config = get_demerit_config(env);
        let current_ledger = env.ledger().sequence();
        if current_ledger >= state.status_updated_ledger.saturating_add(config.cooldown_ledgers) {
            state.demerits = 0;
            state.status = DisqualificationStatus::Active;
            state.status_updated_ledger = current_ledger;
            env.storage().persistent().set(&key, &state);
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
        }
    }

    state
}

pub fn is_source_suspended(env: &Env, source: Address) -> bool {
    let state = get_source_demerits(env, source);
    state.status == DisqualificationStatus::Disqualified
}

pub fn record_invalid_submission(env: &Env, source: Address) {
    let key = DataKey::SourceDemerits(source.clone());
    let mut state = get_source_demerits(env, source.clone());

    state.demerits = state.demerits.saturating_add(1);
    let config = get_demerit_config(env);
    let current_ledger = env.ledger().sequence();

    let old_status = state.status;

    if state.demerits >= config.disqualified_threshold {
        state.status = DisqualificationStatus::Disqualified;
        state.status_updated_ledger = current_ledger;
    } else if state.demerits >= config.probation_threshold {
        state.status = DisqualificationStatus::Probation;
        state.status_updated_ledger = current_ledger;
    } else if state.demerits >= config.warning_threshold {
        state.status = DisqualificationStatus::Warning;
        state.status_updated_ledger = current_ledger;
    }

    env.storage().persistent().set(&key, &state);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    InvalidSubmissionEvent {
        source: source.clone(),
        demerits: state.demerits,
    }
    .publish(env);

    if state.status != old_status {
        match state.status {
            DisqualificationStatus::Warning => {
                SourceWarningEvent {
                    source: source.clone(),
                    demerits: state.demerits,
                }
                .publish(env);
            }
            DisqualificationStatus::Probation => {
                SourceProbationEvent {
                    source: source.clone(),
                    demerits: state.demerits,
                }
                .publish(env);
            }
            DisqualificationStatus::Disqualified => {
                SourceDisqualifiedEvent {
                    source: source.clone(),
                    demerits: state.demerits,
                    status_updated_ledger: current_ledger,
                }
                .publish(env);
            }
            _ => {}
        }
    }
}

pub fn reset_source_demerits(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();

    if !env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    let key = DataKey::SourceDemerits(source.clone());
    let current_ledger = env.ledger().sequence();
    let state = SourceDemeritState {
        demerits: 0,
        status: DisqualificationStatus::Active,
        status_updated_ledger: current_ledger,
    };

    env.storage().persistent().set(&key, &state);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SourceDemeritsResetEvent {
        source,
        admin,
    }
    .publish(env);
}



pub fn get_source_last_heartbeat(env: &Env, source: Address) -> u64 {
    let key = DataKey::SrcHeartbeat(source);
    env.storage().persistent().get(&key).unwrap_or(0u64)
}

// --- #66: Phased source removal ---

const DEFAULT_REMOVAL_COOLDOWN: u32 = 100; // ledgers

pub fn set_removal_cooldown(env: &Env, ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::RemovalCooldown, &ledgers);
    RemovalCooldownChangedEvent { value: ledgers }.publish(env);
}

pub fn get_removal_cooldown(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::RemovalCooldown)
        .unwrap_or(DEFAULT_REMOVAL_COOLDOWN)
}

pub fn mark_source_for_removal(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !env
        .storage()
        .persistent()
        .has(&DataKey::Source(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }
    let cooldown = get_removal_cooldown(env);
    let current_ledger = env.ledger().sequence();
    let eligible_at = current_ledger + cooldown;
    env.storage()
        .persistent()
        .set(&DataKey::SourcePendingRemoval(source.clone()), &eligible_at);
    SourceMarkedForRemovalEvent {
        source: source.clone(),
        admin: admin.clone(),
        eligible_at_ledger: eligible_at,
    }
    .publish(env);
}

pub fn cancel_source_removal(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    if !env
        .storage()
        .persistent()
        .has(&DataKey::SourcePendingRemoval(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotPendingRemoval);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::SourcePendingRemoval(source.clone()));
    SourceRemovalCancelledEvent {
        source: source.clone(),
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn finalize_source_removal(env: &Env, source: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    let eligible_at: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::SourcePendingRemoval(source.clone()))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::SourceNotPendingRemoval));
    if env.ledger().sequence() < eligible_at {
        panic_with_error!(env, ErrorCode::CooldownNotElapsed);
    }
    env.storage()
        .persistent()
        .remove(&DataKey::SourcePendingRemoval(source.clone()));
    // Perform the actual removal (same logic as remove_source)
    env.storage()
        .persistent()
        .remove(&DataKey::Source(source.clone()));
    let mut oracle_sources: OracleSources = read_oracle_sources(env);
    let mut new_sources: Vec<Address> = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let s = oracle_sources.sources.get_unchecked(i);
        if s != source {
            new_sources.push_back(s);
        }
    }
    oracle_sources.sources = new_sources;
    oracle_sources.metadata.remove(source.clone());
    env.storage()
        .persistent()
        .set(&DataKey::SrcRegistry, &oracle_sources);
    SourceRemovedEvent {
        source: source.clone(),
        admin: admin.clone(),
    }
    .publish(env);
}

pub fn is_source_pending_removal(env: &Env, source: Address) -> bool {
    env.storage()
        .persistent()
        .has(&DataKey::SourcePendingRemoval(source))
}

// --- #65: Source reputation ---

const DEFAULT_DECAY_FACTOR: u32 = 10; // out of 100, higher = faster decay towards 50
const INITIAL_REPUTATION: i128 = 50;

pub fn set_reputation_decay_factor(env: &Env, factor: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::ReputationDecayFactor, &factor);
    crate::events::ReputationDecayChangedEvent { value: factor }.publish(env);
}

pub fn get_reputation_decay_factor(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::ReputationDecayFactor)
        .unwrap_or(DEFAULT_DECAY_FACTOR)
}

pub fn get_source_reputation(env: &Env, source: Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::SourceReputation(source))
        .unwrap_or(INITIAL_REPUTATION)
}

/// Called after aggregation to update a source's reputation based on deviation from median.
/// `source_price`: the price submitted by this source
/// `median_price`: the aggregated median for the asset
///
/// Not currently wired into `aggregate_asset` — kept for a future reputation-scoring pass.
#[allow(dead_code)]
pub fn update_source_reputation(
    env: &Env,
    source: &Address,
    source_price: i128,
    median_price: i128,
) {
    if median_price == 0 {
        return;
    }
    let old_score = get_source_reputation(env, source.clone());
    let decay = get_reputation_decay_factor(env) as i128;

    // Deviation in basis points (0 = perfect, 10000 = 100% off)
    let deviation_bps = ((source_price - median_price).abs() * 10_000) / median_price;

    // Accuracy score: 100 if exact, decreasing linearly, floored at 0
    // 100 bps (~1%) deviation → still near perfect; 5000 bps (50%) → score 0
    let accuracy: i128 = if deviation_bps >= 5000 {
        0
    } else {
        100 - (deviation_bps * 100 / 5000)
    };

    // Weighted moving average: new = old * (100 - decay)/100 + accuracy * decay/100
    let new_score = (old_score * (100 - decay) + accuracy * decay) / 100;
    let new_score = new_score.clamp(0, 100);

    env.storage()
        .persistent()
        .set(&DataKey::SourceReputation(source.clone()), &new_score);

    crate::events::SourceReputationUpdatedEvent {
        source: source.clone(),
        old_score,
        new_score,
    }
    .publish(env);
}

// =============================================================================
// #186 — Adaptive Heartbeat / Liveness Detection
// =============================================================================

/// Default: after 64 ledgers of inactivity, auto-remove the source.
const DEFAULT_MAX_INACTIVE_LEDGERS: u32 = 64;
/// Default heartbeat window size — used to smooth the adaptive interval.
const DEFAULT_HEARTBEAT_WINDOW: u32 = 10;
/// Consecutive missed heartbeats before a source is marked inactive.
const MISS_THRESHOLD: u32 = 3;

// --- Configuration accessors ---

/// Sets the maximum number of ledgers a source may remain inactive before automatic removal.
pub fn set_max_inactive_ledgers(env: &Env, ledgers: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if ledgers == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgMaxInactiveLedgers, &ledgers);
    crate::events::InactiveLedgersChangedEvent { value: ledgers }.publish(env);
}

/// Returns the configured max-inactive-ledgers threshold (default 64).
pub fn get_max_inactive_ledgers(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgMaxInactiveLedgers)
        .unwrap_or(DEFAULT_MAX_INACTIVE_LEDGERS)
}

/// Sets the heartbeat window size used in the adaptive-interval formula.
pub fn set_heartbeat_window(env: &Env, window: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    if window == 0 {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }
    env.storage()
        .persistent()
        .set(&DataKey::CfgHeartbeatWindow, &window);
    crate::events::HeartbeatWindowChangedEvent { value: window }.publish(env);
}

/// Returns the configured heartbeat window size (default 10).
pub fn get_heartbeat_window(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgHeartbeatWindow)
        .unwrap_or(DEFAULT_HEARTBEAT_WINDOW)
}

// --- Adaptive interval computation ---

/// Computes the adaptive heartbeat deadline interval for a source.
///
/// Formula: `base_interval * (window + missed) / window`
/// - Minimum returned value is `base_interval` (when missed == 0).
/// - Caps the multiplier at `3×` to prevent unbounded growth.
///
/// # Arguments
/// * `base_interval` — the contract-wide heartbeat interval in seconds.
/// * `missed` — consecutive missed heartbeat count for this source.
/// * `window` — the smoothing window size from config.
pub fn compute_adaptive_interval(base_interval: u64, missed: u32, window: u32) -> u64 {
    let window = if window == 0 { 1 } else { window };
    // multiplier = (window + missed) / window, capped at 3
    let numerator = (window as u64).saturating_add(missed as u64);
    let multiplier = numerator / (window as u64);
    let multiplier = multiplier.min(3);
    let multiplier = if multiplier == 0 { 1 } else { multiplier };
    base_interval.saturating_mul(multiplier)
}

// --- Missed-heartbeat tracking ---

/// Returns the consecutive missed-heartbeat count for a source.
pub fn get_missed_heartbeats(env: &Env, source: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::SrcMissedHeartbeats(source.clone()))
        .unwrap_or(0)
}

/// Increments the missed-heartbeat count by one, returning the new value.
fn increment_missed_heartbeats(env: &Env, source: &Address) -> u32 {
    let count = get_missed_heartbeats(env, source).saturating_add(1);
    env.storage()
        .persistent()
        .set(&DataKey::SrcMissedHeartbeats(source.clone()), &count);
    count
}

/// Resets the missed-heartbeat count to zero.
fn reset_missed_heartbeats(env: &Env, source: &Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::SrcMissedHeartbeats(source.clone()));
}

// --- Health status ---

/// Returns the current `SourceHealthStatus` for a source.
///
/// Does NOT mutate state — safe to call from read-only contexts.
pub fn get_source_health(env: &Env, source: Address) -> crate::types::SourceHealthStatus {
    use crate::types::SourceHealthStatus;

    // If the source isn't registered at all, treat as AutoRemoved.
    if !env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        return SourceHealthStatus::AutoRemoved;
    }

    // If explicitly marked inactive, check whether it should be Inactive or Degraded.
    let is_inactive = check_source_inactive(env, &source);
    if is_inactive {
        return SourceHealthStatus::Inactive;
    }

    let missed = get_missed_heartbeats(env, &source);
    if missed >= MISS_THRESHOLD {
        SourceHealthStatus::Inactive
    } else if missed > 0 {
        SourceHealthStatus::Degraded
    } else {
        SourceHealthStatus::Healthy
    }
}

// --- Reactivation guard ---

/// Records that a source has submitted a price (used for reactivation logic).
pub fn record_price_submitted(env: &Env, source: &Address, ledger: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::SrcLastPriceLedger(source.clone()), &ledger);
    // Mark that a price has been submitted after the most recent reactivation.
    env.storage().persistent().set(
        &DataKey::SrcPriceSubmitAfterReactivation(source.clone()),
        &true,
    );
}

/// Returns the ledger of the most recent price submission from a source.
pub fn get_last_price_ledger(env: &Env, source: Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::SrcLastPriceLedger(source))
        .unwrap_or(0)
}

// --- Auto-removal ---

/// Checks every registered source for extended inactivity and removes those that
/// have been inactive for more than `max_inactive_ledgers` without reactivating.
///
/// **Race-condition guard**: if removing a source would leave fewer active sources
/// than `min_sources_required`, the removal is skipped for that source (the oracle
/// must remain usable).
///
/// Returns the number of sources that were auto-removed.
pub fn check_and_prune_inactive_sources(env: &Env) -> u32 {
    let max_inactive = get_max_inactive_ledgers(env);
    let current_ledger = env.ledger().sequence();
    let min_required = crate::admin::get_min_sources_required(env);

    let oracle_sources = read_oracle_sources(env);
    let total_sources = oracle_sources.sources.len();
    let mut removed_count: u32 = 0;

    // First pass: collect candidates.
    let mut candidates: Vec<Address> = Vec::new(env);
    for i in 0..total_sources {
        let src = oracle_sources.sources.get_unchecked(i);
        if !check_source_inactive(env, &src) {
            continue;
        }
        let inactive_since_key = DataKey::SrcInactiveSinceLedger(src.clone());
        if let Some(inactive_since) = env
            .storage()
            .persistent()
            .get::<_, u32>(&inactive_since_key)
        {
            if current_ledger.saturating_sub(inactive_since) >= max_inactive {
                candidates.push_back(src);
            }
        }
    }

    // Second pass: remove, but keep at least min_required active sources.
    for i in 0..candidates.len() {
        let src = candidates.get_unchecked(i);

        // Count currently active sources before removing.
        let current_sources = read_oracle_sources(env);
        let active_count = current_sources
            .sources
            .iter()
            .filter(|s| !check_source_inactive(env, s))
            .count() as u32;

        if active_count <= min_required {
            // Removing this would break the oracle — skip.
            break;
        }

        let missed = get_missed_heartbeats(env, &src);
        let inactive_since: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::SrcInactiveSinceLedger(src.clone()))
            .unwrap_or(0);

        // Emit health change before removal.
        crate::events::SourceHealthChangedEvent {
            source: src.clone(),
            old_status: 2, // Inactive
            new_status: 3, // AutoRemoved
            missed_heartbeats: missed,
        }
        .publish(env);

        // Perform the actual removal.
        _remove_source_internal(env, src.clone());

        crate::events::SourceAutoRemovedEvent {
            source: src.clone(),
            inactive_since_ledger: inactive_since,
            removed_at_ledger: current_ledger,
            missed_heartbeats: missed,
        }
        .publish(env);

        removed_count += 1;
    }

    removed_count
}

/// Internal source removal helper shared by `remove_source` and auto-removal.
/// Does NOT check admin auth — callers must ensure authorization.
fn _remove_source_internal(env: &Env, source: Address) {
    env.storage()
        .persistent()
        .remove(&DataKey::SrcActive(source.clone()));

    let mut oracle_sources: OracleSources = read_oracle_sources(env);
    let mut new_sources: Vec<Address> = Vec::new(env);
    for i in 0..oracle_sources.sources.len() {
        let s = oracle_sources.sources.get_unchecked(i);
        if s != source {
            new_sources.push_back(s);
        }
    }
    oracle_sources.sources = new_sources;
    oracle_sources.metadata.remove(source.clone());
    env.storage()
        .persistent()
        .set(&DataKey::SrcRegistry, &oracle_sources);

    // Clean up per-source state.
    env.storage()
        .persistent()
        .remove(&DataKey::SrcInactive(source.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::SrcMissedHeartbeats(source.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::SrcInactiveSinceLedger(source.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::SrcLastPriceLedger(source.clone()));
    env.storage()
        .persistent()
        .remove(&DataKey::SrcPriceSubmitAfterReactivation(source));
}

pub fn get_source_governance(env: &Env) -> Option<SourceGovernance> {
    env.storage()
        .persistent()
        .get(&DataKey::SourceGovConfig)
}

pub fn set_source_governance(env: &Env, approvers: Vec<Address>, threshold: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if threshold > approvers.len() {
        panic_with_error!(env, ErrorCode::InvalidGovernanceConfig);
    }

    if threshold == 0 && approvers.len() > 0 {
        panic_with_error!(env, ErrorCode::InvalidGovernanceConfig);
    }

    let gov = SourceGovernance {
        approvers: approvers.clone(),
        threshold,
    };

    env.storage()
        .persistent()
        .set(&DataKey::SourceGovConfig, &gov);

    SourceGovConfigChangedEvent {
        admin,
        threshold,
        approvers_count: approvers.len(),
    }
    .publish(env);
}

pub fn propose_source(env: &Env, proposer: Address, source: Address, name: String) -> u32 {
    proposer.require_auth();

    let gov = get_source_governance(env).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    });

    if gov.threshold == 0 {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let mut is_approver = false;
    for i in 0..gov.approvers.len() {
        if gov.approvers.get_unchecked(i) == proposer {
            is_approver = true;
            break;
        }
    }
    if !is_approver {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let count_key = DataKey::SourceProposalCount;
    let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
    let proposal_id = count.saturating_add(1);
    env.storage().persistent().set(&count_key, &proposal_id);

    let proposal = SourceProposal {
        id: proposal_id,
        source: source.clone(),
        name: name.clone(),
        approvals: Vec::new(env),
        executed: false,
    };

    let prop_key = DataKey::SourceProposal(proposal_id);
    env.storage().persistent().set(&prop_key, &proposal);
    env.storage()
        .persistent()
        .extend_ttl(&prop_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SourceProposalCreatedEvent {
        proposal_id,
        proposer,
        source,
        name,
    }
    .publish(env);

    proposal_id
}

pub fn approve_source(env: &Env, approver: Address, proposal_id: u32) {
    approver.require_auth();

    let gov = get_source_governance(env).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    });

    if gov.threshold == 0 {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let mut is_approver = false;
    for i in 0..gov.approvers.len() {
        if gov.approvers.get_unchecked(i) == approver {
            is_approver = true;
            break;
        }
    }
    if !is_approver {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let prop_key = DataKey::SourceProposal(proposal_id);
    let mut proposal: SourceProposal = env
        .storage()
        .persistent()
        .get(&prop_key)
        .unwrap_or_else(|| {
            panic_with_error!(env, ErrorCode::ProposalNotFound);
        });

    if proposal.executed {
        panic_with_error!(env, ErrorCode::ProposalAlreadyExecuted);
    }

    for i in 0..proposal.approvals.len() {
        if proposal.approvals.get_unchecked(i) == approver {
            panic_with_error!(env, ErrorCode::AlreadyApproved);
        }
    }

    proposal.approvals.push_back(approver.clone());

    SourceProposalApprovedEvent {
        proposal_id,
        approver,
    }
    .publish(env);

    if proposal.approvals.len() >= gov.threshold {
        proposal.executed = true;
        register_source_internal(env, proposal.source.clone(), proposal.name.clone());
        SourceProposalExecutedEvent {
            proposal_id,
            source: proposal.source.clone(),
        }
        .publish(env);
    }

    env.storage().persistent().set(&prop_key, &proposal);
    env.storage()
        .persistent()
        .extend_ttl(&prop_key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

pub fn get_source_proposal(env: &Env, proposal_id: u32) -> SourceProposal {
    let prop_key = DataKey::SourceProposal(proposal_id);
    env.storage()
        .persistent()
        .get(&prop_key)
        .unwrap_or_else(|| {
            panic_with_error!(env, ErrorCode::ProposalNotFound);
        })
}

pub fn get_source_geo(env: &Env, source: Address) -> Option<SourceGeoMetadata> {
    let key = DataKey::SourceGeo(source);
    env.storage().persistent().get(&key)
}

pub fn set_source_geo(env: &Env, source: Address, metadata: SourceGeoMetadata) {
    let admin = get_admin(env);
    admin.require_auth();

    if !env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    let key = DataKey::SourceGeo(source.clone());
    env.storage().persistent().set(&key, &metadata);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    SourceGeoUpdatedEvent {
        source,
        region: metadata.region,
        provider: metadata.provider,
        jurisdiction: metadata.jurisdiction,
    }
    .publish(env);
}

fn calculate_hhi(env: &Env, counts: soroban_sdk::Map<soroban_sdk::String, u32>, total: u32) -> u32 {
    if total == 0 {
        return 0;
    }
    let mut sum_squares = 0u64;
    let keys = counts.keys();
    for i in 0..keys.len() {
        let key = keys.get_unchecked(i);
        let count = counts.get_unchecked(key);
        sum_squares = sum_squares.saturating_add((count as u64) * (count as u64));
    }
    let hhi = sum_squares.saturating_mul(10000) / ((total as u64) * (total as u64));
    hhi as u32
}

pub fn get_decentralization_report(env: &Env) -> DecentralizationReport {
    let oracle_sources = read_oracle_sources(env);
    let total = oracle_sources.sources.len();
    if total == 0 {
        return DecentralizationReport {
            region_hhi: 0,
            provider_hhi: 0,
            jurisdiction_hhi: 0,
            overall_score: 0,
        };
    }

    let mut region_counts: soroban_sdk::Map<soroban_sdk::String, u32> = soroban_sdk::Map::new(env);
    let mut provider_counts: soroban_sdk::Map<soroban_sdk::String, u32> = soroban_sdk::Map::new(env);
    let mut jurisdiction_counts: soroban_sdk::Map<soroban_sdk::String, u32> = soroban_sdk::Map::new(env);

    let default_str = soroban_sdk::String::from_str(env, "unknown");

    for i in 0..total {
        let source = oracle_sources.sources.get_unchecked(i);
        let geo = get_source_geo(env, source);
        let (region, provider, jurisdiction) = match geo {
            Some(g) => (g.region, g.provider, g.jurisdiction),
            None => (default_str.clone(), default_str.clone(), default_str.clone()),
        };

        let rc = region_counts.get(region.clone()).unwrap_or(0);
        region_counts.set(region, rc + 1);

        let pc = provider_counts.get(provider.clone()).unwrap_or(0);
        provider_counts.set(provider, pc + 1);

        let jc = jurisdiction_counts.get(jurisdiction.clone()).unwrap_or(0);
        jurisdiction_counts.set(jurisdiction, jc + 1);
    }

    let region_hhi = calculate_hhi(env, region_counts, total);
    let provider_hhi = calculate_hhi(env, provider_counts, total);
    let jurisdiction_hhi = calculate_hhi(env, jurisdiction_counts, total);

    let avg_hhi = (region_hhi + provider_hhi + jurisdiction_hhi) / 3;
    let overall_score = 10000u32.saturating_sub(avg_hhi);

    DecentralizationReport {
        region_hhi,
        provider_hhi,
        jurisdiction_hhi,
        overall_score,
    }
}

pub fn set_source_bond(env: &Env, amount: i128) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::SourceBondAmount, &amount);

    SourceBondConfigChangedEvent { admin, amount }.publish(env);
}

pub fn get_source_bond(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::SourceBondAmount)
        .unwrap_or(0i128)
}

pub fn get_source_deposited_bond(env: &Env, source: Address) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::SourceBond(source))
        .unwrap_or(0i128)
}

pub fn deposit_source_bond(env: &Env, source: Address) {
    source.require_auth();

    if !env
        .storage()
        .persistent()
        .has(&DataKey::SrcActive(source.clone()))
    {
        panic_with_error!(env, ErrorCode::SourceNotFound);
    }

    let required = get_source_bond(env);
    if required <= 0 {
        return;
    }

    let current_deposited = get_source_deposited_bond(env, source.clone());
    if current_deposited >= required {
        return;
    }

    let deposit_amount = required - current_deposited;
    let token_contract = crate::reputation::get_stake_token_contract(env).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::StakeTokenNotConfigured);
    });

    let client = soroban_sdk::token::Client::new(env, &token_contract);
    client.transfer(&source, &env.current_contract_address(), &deposit_amount);

    let key = DataKey::SourceBond(source.clone());
    env.storage().persistent().set(&key, &required);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);

    if check_source_inactive(env, &source) {
        mark_source_active(env, &source);
    }

    SourceBondDepositedEvent {
        source,
        amount: deposit_amount,
    }
    .publish(env);
}

pub fn forfeit_source_bond_internal(env: &Env, source: Address) {
    let key = DataKey::SourceBond(source.clone());
    let deposited = get_source_deposited_bond(env, source.clone());
    if deposited > 0 {
        let treasury_key = DataKey::TreasuryBalance;
        let balance: i128 = env.storage().persistent().get(&treasury_key).unwrap_or(0);
        env.storage().persistent().set(&treasury_key, &(balance + deposited));

        env.storage().persistent().set(&key, &0i128);

        SourceBondForfeitedEvent {
            source,
            amount: deposited,
        }
        .publish(env);
    }
}


