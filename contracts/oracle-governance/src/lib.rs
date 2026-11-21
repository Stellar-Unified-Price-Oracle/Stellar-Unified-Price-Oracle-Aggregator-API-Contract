//! Standalone `oracle-governance` contract holding extracted governance modules (#780):
//! multisig, timelock, DAO proposals, vote delegation, and quadratic voting.
#![no_std]

pub mod dao;
pub mod delegation;
pub mod multisig;
pub mod quadratic_voting;
pub mod timelock;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, Address, BytesN, Env,
    Symbol, Vec,
};

#[contracterror]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GovernanceError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAuthorized = 3,
    InvalidQuorum = 4,
    OperationNotFound = 5,
    TimelockNotMatured = 6,
    AlreadyExecuted = 7,
    ProposalNotFound = 8,
    VotingClosed = 9,
    InvalidWeight = 10,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GovernanceKey {
    Admin,
    MultisigSigners,
    MultisigQuorum,
    MultisigApprovals(u64),
    TimelockDelay,
    TimelockOp(u64),
    Proposal(u64),
    Delegate(Address),
    QuadraticCredits(Address, u64),
}

#[contract]
pub struct OracleGovernance;

pub(crate) fn require_admin(env: &Env) -> Address {
    match env
        .storage()
        .persistent()
        .get::<_, Address>(&GovernanceKey::Admin)
    {
        Some(admin) => {
            admin.require_auth();
            admin
        }
        None => panic_with_error!(env, GovernanceError::NotInitialized),
    }
}

#[contractimpl]
impl OracleGovernance {
    /// Initialize the governance contract with an administrator, multisig signer set,
    /// quorum threshold, and timelock delay in ledgers.
    pub fn initialize(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
        quorum: u32,
        timelock_delay_ledgers: u32,
    ) {
        if env.storage().persistent().has(&GovernanceKey::Admin) {
            panic_with_error!(&env, GovernanceError::AlreadyInitialized);
        }
        admin.require_auth();
        if quorum == 0 || quorum > signers.len() {
            panic_with_error!(&env, GovernanceError::InvalidQuorum);
        }
        env.storage()
            .persistent()
            .set(&GovernanceKey::Admin, &admin);
        multisig::configure(&env, &signers, quorum);
        timelock::set_delay(&env, timelock_delay_ledgers);
    }

    /// Return the configured multisig quorum threshold.
    pub fn get_quorum(env: Env) -> u32 {
        multisig::get_quorum(&env)
    }

    /// Approve a pending governance operation ID as one of the multisig signers.
    pub fn approve_multisig(env: Env, signer: Address, op_id: u64) -> u32 {
        multisig::approve(&env, signer, op_id)
    }

    /// Schedule a timelocked operation payload hash after multisig quorum is met.
    pub fn schedule_timelock(
        env: Env,
        op_id: u64,
        target: Address,
        action: Symbol,
        payload_hash: BytesN<32>,
    ) -> u32 {
        let _ = require_admin(&env);
        if !multisig::has_quorum(&env, op_id) {
            panic_with_error!(&env, GovernanceError::NotAuthorized);
        }
        timelock::schedule(&env, op_id, target, action, payload_hash)
    }

    /// Mark a matured timelocked operation as executed.
    pub fn execute_timelock(env: Env, op_id: u64) {
        let _ = require_admin(&env);
        timelock::mark_executed(&env, op_id);
    }

    /// Create a DAO proposal with a voting deadline ledger.
    pub fn create_proposal(
        env: Env,
        proposer: Address,
        proposal_id: u64,
        action: Symbol,
        end_ledger: u32,
    ) {
        dao::create_proposal(&env, proposer, proposal_id, action, end_ledger);
    }

    /// Delegate a voter's governance weight to `delegatee`.
    pub fn delegate_vote(env: Env, voter: Address, delegatee: Address) {
        delegation::set_delegate(&env, voter, delegatee);
    }

    /// Cast a quadratic vote on `proposal_id` using `voice_credits` (effective votes = floor(sqrt(credits))).
    pub fn cast_quadratic_vote(
        env: Env,
        voter: Address,
        proposal_id: u64,
        support: bool,
        voice_credits: u64,
    ) -> u64 {
        let effective = quadratic_voting::record_credits(&env, voter, proposal_id, voice_credits);
        dao::add_votes(&env, proposal_id, support, effective);
        effective
    }
}
