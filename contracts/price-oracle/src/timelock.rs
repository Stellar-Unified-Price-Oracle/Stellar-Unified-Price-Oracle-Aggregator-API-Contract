use soroban_sdk::{panic_with_error, Bytes, Env};

use crate::events::{
    OperationCancelledEvent, OperationExecutedEvent, OperationProposedEvent,
    PriorityDelayChangedEvent,
};
use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, OperationPriority, OperationType, PendingOperation};

// ---------------------------------------------------------------------------
// Priority delay helpers
// ---------------------------------------------------------------------------

/// Default delay in ledgers for each priority tier.
const DEFAULT_URGENT_DELAY: u32 = 1;
const DEFAULT_NORMAL_DELAY: u32 = 10;
const DEFAULT_LONG_TERM_DELAY: u32 = 100;

/// Returns the required delay (in ledgers) for the given priority tier.
pub fn get_priority_delay(env: &Env, priority: &OperationPriority) -> u32 {
    match priority {
        OperationPriority::Urgent => {
            let key = DataKey::TlUrgentDelay;
            if env.storage().persistent().has(&key) {
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
                env.storage()
                    .persistent()
                    .get(&key)
                    .unwrap_or(DEFAULT_URGENT_DELAY)
            } else {
                DEFAULT_URGENT_DELAY
            }
        }
        OperationPriority::Normal => {
            // Normal reuses the existing CfgTimelockDuration for backward-compat,
            // with TlNormalDelay as an explicit override.
            let key = DataKey::TlNormalDelay;
            if env.storage().persistent().has(&key) {
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
                env.storage()
                    .persistent()
                    .get(&key)
                    .unwrap_or(DEFAULT_NORMAL_DELAY)
            } else {
                env.storage()
                    .persistent()
                    .get(&DataKey::CfgTimelockDuration)
                    .unwrap_or(DEFAULT_NORMAL_DELAY)
            }
        }
        OperationPriority::LongTerm => {
            let key = DataKey::TlLongTermDelay;
            if env.storage().persistent().has(&key) {
                env.storage()
                    .persistent()
                    .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
                env.storage()
                    .persistent()
                    .get(&key)
                    .unwrap_or(DEFAULT_LONG_TERM_DELAY)
            } else {
                DEFAULT_LONG_TERM_DELAY
            }
        }
    }
}

/// Sets the delay for a given priority tier.  Admin-only.
pub fn set_priority_delay(env: &Env, priority: OperationPriority, delay: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let key = match priority {
        OperationPriority::Urgent => DataKey::TlUrgentDelay,
        OperationPriority::Normal => DataKey::TlNormalDelay,
        OperationPriority::LongTerm => DataKey::TlLongTermDelay,
    };

    let priority_num = match priority {
        OperationPriority::Urgent => 0u32,
        OperationPriority::Normal => 1u32,
        OperationPriority::LongTerm => 2u32,
    };

    env.storage().persistent().set(&key, &delay);

    PriorityDelayChangedEvent {
        priority: priority_num,
        new_delay: delay,
        changed_by: admin,
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Internal helper: op_type → u32
// ---------------------------------------------------------------------------

fn op_type_to_num(op_type: &OperationType) -> u32 {
    match op_type {
        OperationType::Upgrade => 0,
        OperationType::SetAdmin => 1,
        OperationType::SetMinSources => 2,
        OperationType::SetMaxHistory => 3,
        OperationType::SetResolution => 4,
        OperationType::SetDecimals => 5,
        OperationType::SetDescription => 6,
        OperationType::SetTimestampThreshold => 7,
    }
}

// ---------------------------------------------------------------------------
// Original propose_operation — defaults to Normal priority
// ---------------------------------------------------------------------------

pub fn propose_operation(env: &Env, op_type: OperationType, data: &Bytes) -> u32 {
    propose_operation_with_priority(env, op_type, data, OperationPriority::Normal)
}

// ---------------------------------------------------------------------------
// Priority-aware propose
// ---------------------------------------------------------------------------

/// Proposes a timelock operation with an explicit priority tier.
///
/// The required delay before execution is determined by `priority`:
/// * [`OperationPriority::Urgent`]   — 1 ledger (default; configurable)
/// * [`OperationPriority::Normal`]   — 10 ledgers (inherits `CfgTimelockDuration`; configurable)
/// * [`OperationPriority::LongTerm`] — 100 ledgers (default; configurable)
pub fn propose_operation_with_priority(
    env: &Env,
    op_type: OperationType,
    data: &Bytes,
    priority: OperationPriority,
) -> u32 {
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
        priority: priority.clone(),
    };

    env.storage()
        .persistent()
        .set(&DataKey::TlPendingOp(op_id), &pending_op);
    env.storage()
        .persistent()
        .set(&DataKey::TlPendingOpCount, &op_id);

    OperationProposedEvent {
        operation_id: op_id,
        op_type: op_type_to_num(&op_type),
        proposed_by: admin,
        proposed_ledger: current_ledger,
    }
    .publish(env);

    op_id
}

// ---------------------------------------------------------------------------
// Execute
// ---------------------------------------------------------------------------

pub fn execute_operation(env: &Env, op_id: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    let pending_op: PendingOperation = env
        .storage()
        .persistent()
        .get(&DataKey::TlPendingOp(op_id))
        .ok_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
        .unwrap();

    // Use priority-aware delay
    let required_delay = get_priority_delay(env, &pending_op.priority);
    let current_ledger = env.ledger().sequence();
    let elapsed = current_ledger - pending_op.proposed_ledger;

    if elapsed < required_delay {
        panic_with_error!(env, ErrorCode::PriorityTimelockNotReady);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::TlPendingOp(op_id));

    OperationExecutedEvent {
        operation_id: op_id,
        op_type: op_type_to_num(&pending_op.op_type),
        executed_by: admin,
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Cancel
// ---------------------------------------------------------------------------

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

    OperationCancelledEvent {
        operation_id: op_id,
        op_type: op_type_to_num(&pending_op.op_type),
        cancelled_by: admin,
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Legacy duration getter / setter (kept for backward compat)
// ---------------------------------------------------------------------------

pub fn set_timelock_duration(env: &Env, duration: u32) {
    let admin = get_admin(env);
    admin.require_auth();
    crate::config_history::snapshot_before_change(env, &admin);
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

// ---------------------------------------------------------------------------
// Batch operations (#68)
// ---------------------------------------------------------------------------

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
    let admin = get_admin(env);
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
                crate::config_history::snapshot_before_change(env, &admin);
                env.storage()
                    .persistent()
                    .set(&DataKey::CfgMinSources, &val);
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
                crate::config_history::snapshot_before_change(env, &admin);
                env.storage()
                    .persistent()
                    .set(&DataKey::CfgMaxHistory, &val);
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
                crate::config_history::snapshot_before_change(env, &admin);
                env.storage()
                    .persistent()
                    .set(&DataKey::CfgResolution, &val);
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
                crate::config_history::snapshot_before_change(env, &admin);
                env.storage().persistent().set(&DataKey::CfgDecimals, &val);
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
                crate::config_history::snapshot_before_change(env, &admin);
                env.storage()
                    .persistent()
                    .set(&DataKey::CfgTimestampThreshold, &val);
            }
        }
        _ => {
            panic_with_error!(env, ErrorCode::OperationNotFound);
        }
    }
}
