use soroban_sdk::{panic_with_error, Bytes, Env};

use crate::events::{OperationCancelledEvent, OperationExecutedEvent, OperationProposedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, OperationType, PendingOperation};

pub fn propose_operation(
    env: &Env,
    op_type: OperationType,
    data: &Bytes,
) -> u32 {
    let admin = get_admin(env);
    admin.require_auth();

    let current_ledger = env.ledger().sequence();
    let op_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::PendingOpCount)
        .unwrap_or(0);
    let op_id = op_count + 1;

    let pending_op = PendingOperation {
        id: op_id,
        op_type: op_type.clone(),
        proposed_by: admin.clone(),
        proposed_ledger: current_ledger,
        data: data.clone(),
    };

    env.storage()
        .persistent()
        .set(&DataKey::PendingOp(op_id), &pending_op);
    env.storage()
        .persistent()
        .set(&DataKey::PendingOpCount, &op_id);

    let op_type_num = match op_type {
        OperationType::Upgrade => 0,
        OperationType::SetAdmin => 1,
        OperationType::SetMinSources => 2,
        OperationType::SetMaxHistory => 3,
        OperationType::SetResolution => 4,
        OperationType::SetDecimals => 5,
        OperationType::SetDescription => 6,
        OperationType::SetTimestampThreshold => 7,
    };

    OperationProposedEvent {
        operation_id: op_id,
        op_type: op_type_num,
        proposed_by: admin,
        proposed_ledger: current_ledger,
    }
    .publish(env);

    op_id
}

pub fn execute_operation(env: &Env, op_id: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let pending_op: PendingOperation = env
        .storage()
        .persistent()
        .get(&DataKey::PendingOp(op_id))
        .ok_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
        .unwrap();

    let timelock_duration: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::TimelockDuration)
        .unwrap_or(10);
    let current_ledger = env.ledger().sequence();
    let elapsed = current_ledger - pending_op.proposed_ledger;

    if elapsed < timelock_duration {
        panic_with_error!(env, ErrorCode::TimelockNotReady);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::PendingOp(op_id));

    let op_type_num = match pending_op.op_type {
        OperationType::Upgrade => 0,
        OperationType::SetAdmin => 1,
        OperationType::SetMinSources => 2,
        OperationType::SetMaxHistory => 3,
        OperationType::SetResolution => 4,
        OperationType::SetDecimals => 5,
        OperationType::SetDescription => 6,
        OperationType::SetTimestampThreshold => 7,
    };

    OperationExecutedEvent {
        operation_id: op_id,
        op_type: op_type_num,
        executed_by: admin,
    }
    .publish(env);
}

pub fn cancel_operation(env: &Env, op_id: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let pending_op: PendingOperation = env
        .storage()
        .persistent()
        .get(&DataKey::PendingOp(op_id))
        .ok_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
        .unwrap();

    env.storage()
        .persistent()
        .remove(&DataKey::PendingOp(op_id));

    let op_type_num = match pending_op.op_type {
        OperationType::Upgrade => 0,
        OperationType::SetAdmin => 1,
        OperationType::SetMinSources => 2,
        OperationType::SetMaxHistory => 3,
        OperationType::SetResolution => 4,
        OperationType::SetDecimals => 5,
        OperationType::SetDescription => 6,
        OperationType::SetTimestampThreshold => 7,
    };

    OperationCancelledEvent {
        operation_id: op_id,
        op_type: op_type_num,
        cancelled_by: admin,
    }
    .publish(env);
}

pub fn set_timelock_duration(env: &Env, duration: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    env.storage()
        .persistent()
        .set(&DataKey::TimelockDuration, &duration);
}

pub fn get_timelock_duration(env: &Env) -> u32 {
    let key = DataKey::TimelockDuration;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(10)
}
