use soroban_sdk::{panic_with_error, Address, Bytes, BytesN, Env, Vec};

use crate::admin::{get_decimals, get_max_history_length, get_timestamp_threshold};
use crate::events::{
    ExternalDataResolvedEvent, PriceProposalCreatedEvent, PriceProposalDisputedEvent,
    PriceProposalResolvedEvent,
};
use crate::history::{remove_history_shard_entry, should_skip_on_write, write_history_shard};
use crate::storage::{check_registered_asset, check_source, get_admin};
use crate::types::{
    AggregatePrice, DataKey, ErrorCode, ExternalDataProof, OptimisticProposal,
    OptimisticProposalStatus, PriceHistoryEntry,
};

const DEFAULT_DISPUTE_WINDOW: u32 = 120;
const DEFAULT_MIN_BOND: i128 = 100_000_000;

fn read_dispute_window(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgOptimisticDisputeWindow)
        .unwrap_or(DEFAULT_DISPUTE_WINDOW)
}

fn read_min_bond(env: &Env) -> i128 {
    env.storage()
        .persistent()
        .get(&DataKey::CfgOptimisticMinBond)
        .unwrap_or(DEFAULT_MIN_BOND)
}

fn read_proposal_count(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::OptimisticProposalCount)
        .unwrap_or(0)
}

fn write_proposal_count(env: &Env, count: u32) {
    env.storage()
        .persistent()
        .set(&DataKey::OptimisticProposalCount, &count);
}

fn write_proposal(env: &Env, proposal: &OptimisticProposal) {
    env.storage()
        .persistent()
        .set(&DataKey::OptimisticProposal(proposal.id), proposal);
}

fn read_proposal(env: &Env, proposal_id: u32) -> Option<OptimisticProposal> {
    env.storage()
        .persistent()
        .get(&DataKey::OptimisticProposal(proposal_id))
}

fn write_price_snapshot(
    env: &Env,
    asset: &Address,
    price: i128,
    timestamp: u64,
    num_sources: u32,
    current_ledger: u32,
    decimals: u32,
) {
    let prev_version: u32 = env
        .storage()
        .persistent()
        .get::<_, AggregatePrice>(&DataKey::Aggregate(asset.clone()))
        .map(|a| {
            if price != a.price {
                a.version.saturating_add(1)
            } else {
                a.version
            }
        })
        .unwrap_or(0);
    let aggregate = AggregatePrice {
        price,
        timestamp,
        num_sources,
        decimals,
        is_override: false,
        version: prev_version,
    };
    env.storage()
        .persistent()
        .set(&DataKey::Aggregate(asset.clone()), &aggregate);
    env.storage().persistent().extend_ttl(
        &DataKey::Aggregate(asset.clone()),
        crate::storage::LEDGER_THRESHOLD,
        crate::storage::LEDGER_BUMP,
    );

    let history_entry = PriceHistoryEntry {
        price,
        timestamp,
        ledger: current_ledger,
        num_sources,
        is_interpolated: false,
    };

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let mut ledger_list: soroban_sdk::Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(soroban_sdk::Vec::new(env));
    if !should_skip_on_write(env, asset, price) {
        env.storage().temporary().set(
            &DataKey::PriceHistory(asset.clone(), current_ledger),
            &history_entry,
        );
        if ledger_list.len() == 0
            || ledger_list.get_unchecked(ledger_list.len() - 1) != current_ledger
        {
            ledger_list.push_back(current_ledger);
        }
        write_history_shard(env, asset, &history_entry);
    }

    let max_history = get_max_history_length(env);
    while ledger_list.len() > max_history {
        let oldest_ledger = ledger_list.get_unchecked(0);
        ledger_list.remove(0);
        env.storage()
            .temporary()
            .remove(&DataKey::PriceHistory(asset.clone(), oldest_ledger));
        remove_history_shard_entry(env, asset, oldest_ledger);
    }

    env.storage().persistent().set(&ledgers_key, &ledger_list);
}

fn finalize_if_expired(env: &Env, proposal: &OptimisticProposal) -> OptimisticProposal {
    if proposal.status != OptimisticProposalStatus::Pending as u32 {
        return proposal.clone();
    }
    if env.ledger().sequence() < proposal.expires_at_ledger {
        return proposal.clone();
    }

    let mut finalized = proposal.clone();
    finalized.status = OptimisticProposalStatus::Finalized as u32;
    finalized.resolved = true;
    finalized.resolution = 1;
    write_proposal(env, &finalized);
    write_price_snapshot(
        env,
        &finalized.asset,
        finalized.price,
        finalized.timestamp,
        1,
        env.ledger().sequence(),
        get_decimals(env),
    );
    finalized
}

pub fn propose_price(
    env: &Env,
    asset: Address,
    price: i128,
    timestamp: u64,
    bond_amount: i128,
) -> u32 {
    check_registered_asset(env, &asset);
    if price <= 0 {
        panic_with_error!(env, ErrorCode::InvalidPrice);
    }
    let threshold = get_timestamp_threshold(env);
    let ledger_time = env.ledger().timestamp();
    if timestamp > ledger_time.saturating_add(threshold) {
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }

    let min_bond = read_min_bond(env);
    if bond_amount < min_bond {
        panic_with_error!(env, ErrorCode::BondTooSmall);
    }

    let proposer = env.invoker();
    proposer.require_auth();

    let proposal_id = read_proposal_count(env) + 1;
    let dispute_window = read_dispute_window(env);
    let proposal = OptimisticProposal {
        id: proposal_id,
        asset: asset.clone(),
        proposer: proposer.clone(),
        price,
        timestamp,
        bond_amount,
        dispute_window,
        expires_at_ledger: env.ledger().sequence().saturating_add(dispute_window),
        status: OptimisticProposalStatus::Pending as u32,
        disputed: false,
        resolved: false,
        resolution: 0,
        disputer: None,
    };
    write_proposal_count(env, proposal_id);
    write_proposal(env, &proposal);

    PriceProposalCreatedEvent {
        asset: asset.clone(),
        proposer: proposer.clone(),
        proposal_id,
        price,
        timestamp,
        bond_amount,
        expires_at_ledger: proposal.expires_at_ledger,
    }
    .publish(env);
    proposal_id
}

pub fn dispute_proposal(env: &Env, proposal_id: u32) {
    let disputer = env.invoker();
    disputer.require_auth();

    let proposal = read_proposal(env, proposal_id).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::ProposalNotFound);
    });
    let mut proposal = finalize_if_expired(env, &proposal);

    if proposal.status != OptimisticProposalStatus::Pending as u32 {
        if proposal.status == OptimisticProposalStatus::Disputed as u32 {
            panic_with_error!(env, ErrorCode::ProposalAlreadyDisputed);
        }
        if proposal.status == OptimisticProposalStatus::Resolved as u32
            || proposal.status == OptimisticProposalStatus::Finalized as u32
        {
            panic_with_error!(env, ErrorCode::ProposalAlreadyResolved);
        }
    }

    if env.ledger().sequence() >= proposal.expires_at_ledger {
        panic_with_error!(env, ErrorCode::ProposalExpired);
    }

    let min_bond = read_min_bond(env);
    if proposal.bond_amount < min_bond {
        panic_with_error!(env, ErrorCode::BondTooSmall);
    }

    proposal.status = OptimisticProposalStatus::Disputed as u32;
    proposal.disputed = true;
    proposal.disputer = Some(disputer.clone());
    write_proposal(env, &proposal);

    PriceProposalDisputedEvent {
        proposal_id,
        disputer: disputer.clone(),
        bond_amount: proposal.bond_amount,
    }
    .publish(env);
}

pub fn resolve_dispute(env: &Env, proposal_id: u32, resolution: bool) {
    let admin = get_admin(env);
    admin.require_auth();

    let proposal = read_proposal(env, proposal_id).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::ProposalNotFound);
    });
    let mut proposal = finalize_if_expired(env, &proposal);

    if !proposal.disputed {
        panic_with_error!(env, ErrorCode::ProposalNotDisputed);
    }
    if proposal.status == OptimisticProposalStatus::Resolved as u32
        || proposal.status == OptimisticProposalStatus::Finalized as u32
    {
        panic_with_error!(env, ErrorCode::ProposalAlreadyResolved);
    }

    proposal.status = OptimisticProposalStatus::Resolved as u32;
    proposal.resolved = true;
    proposal.resolution = if resolution { 1 } else { 2 };

    if resolution {
        write_price_snapshot(
            env,
            &proposal.asset,
            proposal.price,
            proposal.timestamp,
            1,
            env.ledger().sequence(),
            get_decimals(env),
        );
    }

    write_proposal(env, &proposal);

    PriceProposalResolvedEvent {
        proposal_id,
        approved: resolution,
        finalized: resolution,
    }
    .publish(env);
}

pub fn get_proposal(env: &Env, proposal_id: u32) -> Option<OptimisticProposal> {
    let proposal = read_proposal(env, proposal_id)?;
    let updated = finalize_if_expired(env, &proposal);
    if updated.id != proposal.id {
        return None;
    }
    Some(updated)
}

pub fn resolve_via_external_data(
    env: &Env,
    proposal_id: u32,
    external_price: i128,
    proof: ExternalDataProof,
) {
    let admin = get_admin(env);
    admin.require_auth();

    let proposal = read_proposal(env, proposal_id).unwrap_or_else(|| {
        panic_with_error!(env, ErrorCode::ProposalNotFound);
    });
    let mut proposal = finalize_if_expired(env, &proposal);

    if !proposal.disputed {
        panic_with_error!(env, ErrorCode::ProposalNotDisputed);
    }
    if proposal.status == OptimisticProposalStatus::Resolved as u32
        || proposal.status == OptimisticProposalStatus::Finalized as u32
    {
        panic_with_error!(env, ErrorCode::ProposalAlreadyResolved);
    }

    check_source(env, &proof.source);
    let threshold = get_timestamp_threshold(env);
    let ledger_time = env.ledger().timestamp();
    if proof.timestamp > ledger_time.saturating_add(threshold) {
        panic_with_error!(env, ErrorCode::InvalidTimestamp);
    }
    if proof.signature.to_array().iter().all(|b| *b == 0) {
        panic_with_error!(env, ErrorCode::InvalidExternalProof);
    }

    let mut preimage = Bytes::new(env);
    preimage.append(&proof.source.to_xdr(env));
    let price_bytes = external_price.to_le_bytes();
    for b in price_bytes.iter() {
        preimage.push_back(b);
    }
    let ts_bytes = proof.timestamp.to_le_bytes();
    for b in ts_bytes.iter() {
        preimage.push_back(b);
    }
    let expected_hash: BytesN<32> = env.crypto().sha256(&preimage).into();
    if expected_hash != proof.data_hash {
        panic_with_error!(env, ErrorCode::InvalidExternalProof);
    }

    proposal.status = OptimisticProposalStatus::Resolved as u32;
    proposal.resolved = true;
    proposal.resolution = 1;

    write_price_snapshot(
        env,
        &proposal.asset,
        external_price,
        proof.timestamp,
        1,
        env.ledger().sequence(),
        get_decimals(env),
    );
    write_proposal(env, &proposal);

    ExternalDataResolvedEvent {
        proposal_id,
        external_price,
        resolver: admin,
    }
    .publish(env);
}

pub fn get_active_proposals(env: &Env) -> Vec<OptimisticProposal> {
    let count = read_proposal_count(env);
    let mut active: Vec<OptimisticProposal> = Vec::new(env);
    for id in 1..=count {
        if let Some(mut proposal) = read_proposal(env, id) {
            proposal = finalize_if_expired(env, &proposal);
            if proposal.status == OptimisticProposalStatus::Pending as u32
                || proposal.status == OptimisticProposalStatus::Disputed as u32
            {
                active.push_back(proposal);
            }
        }
    }
    active
}
