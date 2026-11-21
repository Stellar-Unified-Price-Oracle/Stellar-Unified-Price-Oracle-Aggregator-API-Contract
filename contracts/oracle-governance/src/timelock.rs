//! Timelock scheduling and maturity enforcement for governance operations (#780).
use soroban_sdk::{contracttype, panic_with_error, Address, BytesN, Env, Symbol};

use crate::{GovernanceError, GovernanceKey};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TimelockOperation {
    pub op_id: u64,
    pub target: Address,
    pub action: Symbol,
    pub payload_hash: BytesN<32>,
    pub ready_ledger: u32,
    pub executed: bool,
}

pub fn set_delay(env: &Env, delay_ledgers: u32) {
    env.storage()
        .persistent()
        .set(&GovernanceKey::TimelockDelay, &delay_ledgers);
}

pub fn get_delay(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get::<_, u32>(&GovernanceKey::TimelockDelay)
        .unwrap_or(0)
}

pub fn schedule(
    env: &Env,
    op_id: u64,
    target: Address,
    action: Symbol,
    payload_hash: BytesN<32>,
) -> u32 {
    let delay = get_delay(env);
    let ready_ledger = env.ledger().sequence().saturating_add(delay);
    let op = TimelockOperation {
        op_id,
        target,
        action,
        payload_hash,
        ready_ledger,
        executed: false,
    };
    env.storage()
        .persistent()
        .set(&GovernanceKey::TimelockOp(op_id), &op);
    ready_ledger
}

pub fn mark_executed(env: &Env, op_id: u64) {
    let key = GovernanceKey::TimelockOp(op_id);
    let mut op: TimelockOperation = match env.storage().persistent().get(&key) {
        Some(found) => found,
        None => panic_with_error!(env, GovernanceError::OperationNotFound),
    };
    if op.executed {
        panic_with_error!(env, GovernanceError::AlreadyExecuted);
    }
    if env.ledger().sequence() < op.ready_ledger {
        panic_with_error!(env, GovernanceError::TimelockNotMatured);
    }
    op.executed = true;
    env.storage().persistent().set(&key, &op);
}
