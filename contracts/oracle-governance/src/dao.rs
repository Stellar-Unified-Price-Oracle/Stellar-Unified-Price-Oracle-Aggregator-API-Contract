//! DAO proposal lifecycle and vote tallying (#780).
use soroban_sdk::{contracttype, panic_with_error, Address, Env, Symbol};

use crate::{GovernanceError, GovernanceKey};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub action: Symbol,
    pub end_ledger: u32,
    pub votes_for: u64,
    pub votes_against: u64,
}

pub fn create_proposal(
    env: &Env,
    proposer: Address,
    proposal_id: u64,
    action: Symbol,
    end_ledger: u32,
) {
    proposer.require_auth();
    if end_ledger <= env.ledger().sequence() {
        panic_with_error!(env, GovernanceError::VotingClosed);
    }
    let proposal = Proposal {
        id: proposal_id,
        proposer,
        action,
        end_ledger,
        votes_for: 0,
        votes_against: 0,
    };
    env.storage()
        .persistent()
        .set(&GovernanceKey::Proposal(proposal_id), &proposal);
}

pub fn add_votes(env: &Env, proposal_id: u64, support: bool, weight: u64) {
    let key = GovernanceKey::Proposal(proposal_id);
    let mut proposal: Proposal = match env.storage().persistent().get(&key) {
        Some(found) => found,
        None => panic_with_error!(env, GovernanceError::ProposalNotFound),
    };
    if env.ledger().sequence() > proposal.end_ledger {
        panic_with_error!(env, GovernanceError::VotingClosed);
    }
    if support {
        proposal.votes_for = proposal.votes_for.saturating_add(weight);
    } else {
        proposal.votes_against = proposal.votes_against.saturating_add(weight);
    }
    env.storage().persistent().set(&key, &proposal);
}
