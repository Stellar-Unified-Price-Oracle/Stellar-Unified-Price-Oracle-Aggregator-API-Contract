//! Vote delegation registry for governance participants (#780).
use soroban_sdk::{Address, Env};

use crate::GovernanceKey;

pub fn set_delegate(env: &Env, voter: Address, delegatee: Address) {
    voter.require_auth();
    env.storage()
        .persistent()
        .set(&GovernanceKey::Delegate(voter), &delegatee);
}

pub fn resolve_delegate(env: &Env, voter: &Address) -> Address {
    env.storage()
        .persistent()
        .get::<_, Address>(&GovernanceKey::Delegate(voter.clone()))
        .unwrap_or_else(|| voter.clone())
}
