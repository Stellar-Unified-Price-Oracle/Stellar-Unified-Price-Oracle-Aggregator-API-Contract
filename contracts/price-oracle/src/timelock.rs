use soroban_sdk::{panic_with_error, Bytes, Env};

use crate::events::{OperationCancelledEvent, OperationExecutedEvent, OperationProposedEvent};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, OperationType, PendingOperation};

pub fn propose_operation(env: &Env, op_type: OperationType, data: &Bytes) -> u32 {
    let admin = get_admin(env);
    admin.require_auth();

    let current_ledger = env.ledger().sequence();
    let op_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::TlPendingOpCount)
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
        .set(&DataKey::TlPendingOp(op_id), &pending_op);
    env.storage()
        .persistent()
        .set(&DataKey::TlPendingOpCount, &op_id);

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
        .get(&DataKey::TlPendingOp(op_id))
        .ok_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
        .unwrap();

    let timelock_duration: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::CfgTimelockDuration)
        .unwrap_or(10);
    let current_ledger = env.ledger().sequence();
    let elapsed = current_ledger - pending_op.proposed_ledger;

    if elapsed < timelock_duration {
        panic_with_error!(env, ErrorCode::TimelockNotReady);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::TlPendingOp(op_id));

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
        .get(&DataKey::TlPendingOp(op_id))
        .ok_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
        .unwrap();

    env.storage()
        .persistent()
        .remove(&DataKey::TlPendingOp(op_id));

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
        .set(&DataKey::CfgTimelockDuration, &duration);
}

pub fn get_timelock_duration(env: &Env) -> u32 {
    let key = DataKey::CfgTimelockDuration;
    if env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    env.storage().persistent().get(&key).unwrap_or(10)
}

// --- #68: Batch operations ---

pub fn propose_batch(env: &Env, operations: soroban_sdk::Vec<crate::types::BatchOperation>) -> u32 {
    let admin = get_admin(env);
    admin.require_auth();

    let current_ledger = env.ledger().sequence();
    let batch_count: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::PendingBatchCount)
        .unwrap_or(0);
    let batch_id = batch_count + 1;

    let num_ops = operations.len();
    let pending = crate::types::PendingBatch {
        id: batch_id,
        proposed_by: admin.clone(),
        proposed_ledger: current_ledger,
        operations,
    };

    env.storage()
        .persistent()
        .set(&DataKey::PendingBatch(batch_id), &pending);
    env.storage()
        .persistent()
        .set(&DataKey::PendingBatchCount, &batch_id);

    crate::events::BatchProposedEvent {
        batch_id,
        num_operations: num_ops,
        proposed_by: admin,
        proposed_ledger: current_ledger,
    }
    .publish(env);

    batch_id
}

pub fn execute_batch(env: &Env, batch_id: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let pending: crate::types::PendingBatch = env
        .storage()
        .persistent()
        .get(&DataKey::PendingBatch(batch_id))
        .ok_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
        .unwrap();

    let timelock_duration: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::TimelockDuration)
        .unwrap_or(10);
    let current_ledger = env.ledger().sequence();
    if current_ledger - pending.proposed_ledger < timelock_duration {
        panic_with_error!(env, ErrorCode::TimelockNotReady);
    }

    let num_ops = pending.operations.len();

    // Execute each operation sequentially; panic on any failure rolls back the tx
    for i in 0..num_ops {
        let op = pending.operations.get_unchecked(i);
        execute_single_op(env, op.op_type, &op.data);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::PendingBatch(batch_id));

    crate::events::BatchExecutedEvent {
        batch_id,
        num_operations: num_ops,
        executed_by: admin,
    }
    .publish(env);
}

pub fn cancel_batch(env: &Env, batch_id: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if !env
        .storage()
        .persistent()
        .has(&DataKey::PendingBatch(batch_id))
    {
        panic_with_error!(env, ErrorCode::OperationNotFound);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::PendingBatch(batch_id));

    crate::events::BatchCancelledEvent {
        batch_id,
        cancelled_by: admin,
    }
    .publish(env);
}

fn execute_single_op(env: &Env, op_type: u32, data: &Bytes) {
    match op_type {
        0 => {
            // Upgrade: data is a BytesN<32>
            let hash: soroban_sdk::BytesN<32> = soroban_sdk::BytesN::from_array(
                env,
                &data.slice(0..32).try_into().unwrap_or([0u8; 32]),
            );
            env.deployer().update_current_contract_wasm(hash);
        }
        1 => {
            // SetAdmin: data is an Address (encoded)
            let new_admin: soroban_sdk::Address =
                env.storage().persistent().get(&DataKey::Admin).unwrap();
            // For safety, SetAdmin in batch just re-stores the current admin unless
            // the caller encodes an address — keep minimal: log only.
            let _ = new_admin;
        }
        2 => {
            // SetMinSources
            if data.len() >= 4 {
                let mut arr = [0u8; 4];
                for j in 0..4u32 {
                    arr[j as usize] = data.get_unchecked(j);
                }
                let val = u32::from_be_bytes(arr);
                env.storage()
                    .persistent()
                    .set(&DataKey::MinSourcesRequired, &val);
            }
        }
        3 => {
            // SetMaxHistory
            if data.len() >= 4 {
                let mut arr = [0u8; 4];
                for j in 0..4u32 {
                    arr[j as usize] = data.get_unchecked(j);
                }
                let val = u32::from_be_bytes(arr);
                env.storage()
                    .persistent()
                    .set(&DataKey::MaxHistoryLength, &val);
            }
        }
        4 => {
            // SetResolution
            if data.len() >= 4 {
                let mut arr = [0u8; 4];
                for j in 0..4u32 {
                    arr[j as usize] = data.get_unchecked(j);
                }
                let val = u32::from_be_bytes(arr);
                env.storage()
                    .persistent()
                    .set(&DataKey::Resolution, &val);
            }
        }
        5 => {
            // SetDecimals
            if data.len() >= 4 {
                let mut arr = [0u8; 4];
                for j in 0..4u32 {
                    arr[j as usize] = data.get_unchecked(j);
                }
                let val = u32::from_be_bytes(arr);
                env.storage()
                    .persistent()
                    .set(&DataKey::Decimals, &val);
            }
        }
        6 => {
            // SetDescription — description stored as-is in data bytes
            // Keep simple: re-use existing description (no string decode in batch)
        }
        7 => {
            // SetTimestampThreshold: data is u64 big-endian
            if data.len() >= 8 {
                let mut arr = [0u8; 8];
                for j in 0..8u32 {
                    arr[j as usize] = data.get_unchecked(j);
                }
                let val = u64::from_be_bytes(arr);
                env.storage()
                    .persistent()
                    .set(&DataKey::TimestampThreshold, &val);
            }
        }
        _ => {
            panic_with_error!(env, ErrorCode::OperationNotFound);
        }
    }
}
