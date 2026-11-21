//! Multisig signer registry and M-of-N approval tracking (#780).
use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::{GovernanceError, GovernanceKey};

pub fn configure(env: &Env, signers: &Vec<Address>, quorum: u32) {
    env.storage()
        .persistent()
        .set(&GovernanceKey::MultisigSigners, signers);
    env.storage()
        .persistent()
        .set(&GovernanceKey::MultisigQuorum, &quorum);
}

pub fn get_quorum(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get::<_, u32>(&GovernanceKey::MultisigQuorum)
        .unwrap_or(0)
}

pub fn is_signer(env: &Env, candidate: &Address) -> bool {
    let signers: Vec<Address> = env
        .storage()
        .persistent()
        .get(&GovernanceKey::MultisigSigners)
        .unwrap_or_else(|| Vec::new(env));
    signers.contains(candidate)
}

pub fn approve(env: &Env, signer: Address, op_id: u64) -> u32 {
    signer.require_auth();
    if !is_signer(env, &signer) {
        panic_with_error!(env, GovernanceError::NotAuthorized);
    }
    let key = GovernanceKey::MultisigApprovals(op_id);
    let mut approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| Vec::new(env));
    if !approvals.contains(&signer) {
        approvals.push_back(signer);
        env.storage().persistent().set(&key, &approvals);
    }
    approvals.len()
}

pub fn has_quorum(env: &Env, op_id: u64) -> bool {
    let quorum = get_quorum(env);
    if quorum == 0 {
        return false;
    }
    let approvals: Vec<Address> = env
        .storage()
        .persistent()
        .get(&GovernanceKey::MultisigApprovals(op_id))
        .unwrap_or_else(|| Vec::new(env));
    approvals.len() >= quorum
}
