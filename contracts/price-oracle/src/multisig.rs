//! # N-of-M Multi-Sig Governance & Ordered Timelock (#178)
//!
//! Extends the existing single-admin timelock with a multi-signature approval layer.
//! Operations require `required_approvals` out of the registered governor set before
//! the timelock execution timer begins.  Proposals are stored in a linked-list pattern
//! to enforce sequential execution ordering.

use soroban_sdk::{panic_with_error, Address, Bytes, Env, Vec};

use crate::storage::{get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, MultiSigOperation, OperationType};

// ─────────────────────────────────────────────────────────────────────────────
// Storage helpers
// ─────────────────────────────────────────────────────────────────────────────

fn read_governors(env: &Env) -> Vec<Address> {
    let key = DataKey::MsGovernors;
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

fn write_governors(env: &Env, governors: &Vec<Address>) {
    let key = DataKey::MsGovernors;
    env.storage().persistent().set(&key, governors);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn read_required_approvals(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::MsRequiredApprovals)
        .unwrap_or(1u32)
}

fn read_op(env: &Env, op_id: u32) -> MultiSigOperation {
    env.storage()
        .persistent()
        .get(&DataKey::MsOp(op_id))
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
}

fn write_op(env: &Env, op: &MultiSigOperation) {
    let key = DataKey::MsOp(op.id);
    env.storage().persistent().set(&key, op);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn remove_op(env: &Env, op_id: u32) {
    env.storage().persistent().remove(&DataKey::MsOp(op_id));
}

/// Returns the head of the ordered proposal queue (linked list head pointer).
fn read_queue_head(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::MsQueueHead)
        .unwrap_or(0u32)
}

fn write_queue_head(env: &Env, head: u32) {
    env.storage().persistent().set(&DataKey::MsQueueHead, &head);
}

/// Returns the tail of the ordered proposal queue.
fn read_queue_tail(env: &Env) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::MsQueueTail)
        .unwrap_or(0u32)
}

fn write_queue_tail(env: &Env, tail: u32) {
    env.storage().persistent().set(&DataKey::MsQueueTail, &tail);
}

/// Op counter (monotonically increasing).
fn next_op_id(env: &Env) -> u32 {
    let key = DataKey::MsOpCount;
    let count: u32 = env.storage().persistent().get(&key).unwrap_or(0);
    let next = count + 1;
    env.storage().persistent().set(&key, &next);
    next
}

// ─────────────────────────────────────────────────────────────────────────────
// Governor management
// ─────────────────────────────────────────────────────────────────────────────

/// Sets the full governor set and required approvals threshold (admin only).
pub fn set_governors(env: &Env, governors: Vec<Address>, required: u32) {
    let admin = get_admin(env);
    admin.require_auth();

    if required == 0 || required > governors.len() {
        panic_with_error!(env, ErrorCode::InvalidConfiguration);
    }

    write_governors(env, &governors);
    env.storage()
        .persistent()
        .set(&DataKey::MsRequiredApprovals, &required);

    crate::events::MsGovernorsUpdatedEvent {
        admin: admin.clone(),
        governor_count: governors.len(),
        required_approvals: required,
    }
    .publish(env);
}

/// Returns the current governor list.
pub fn get_governors(env: &Env) -> Vec<Address> {
    read_governors(env)
}

/// Returns the required approvals threshold.
pub fn get_required_approvals(env: &Env) -> u32 {
    read_required_approvals(env)
}

// ─────────────────────────────────────────────────────────────────────────────
// Proposal lifecycle
// ─────────────────────────────────────────────────────────────────────────────

/// Proposes a multi-sig governance operation.
///
/// Any registered governor may propose. The operation enters `approvals = []`
/// state. The timelock clock does NOT start until the N-th approval.
///
/// Returns the new operation ID.
pub fn propose_ms_operation(
    env: &Env,
    proposer: Address,
    op_type: OperationType,
    data: Bytes,
) -> u32 {
    proposer.require_auth();

    // Proposer must be a governor
    let governors = read_governors(env);
    if !governors.contains(&proposer) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let op_id = next_op_id(env);
    let required = read_required_approvals(env);

    // Append to linked-list queue
    let tail = read_queue_tail(env);

    let op = MultiSigOperation {
        id: op_id,
        op_type: op_type.clone(),
        proposed_by: proposer.clone(),
        proposed_ledger: env.ledger().sequence(),
        data,
        approvals: Vec::new(env),
        required_approvals: required,
        // Timelock not started yet; 0 means "not ready"
        timelock_start_ledger: 0,
        // Linked-list: this node has no successor yet
        next_op_id: 0,
    };

    write_op(env, &op);

    // Update linked list
    if tail == 0 {
        // Queue was empty
        write_queue_head(env, op_id);
        write_queue_tail(env, op_id);
    } else {
        // Append to tail's `next_op_id`
        let mut tail_op = read_op(env, tail);
        tail_op.next_op_id = op_id;
        write_op(env, &tail_op);
        write_queue_tail(env, op_id);
    }

    let op_type_num = op_type_to_u32(&op.op_type);
    crate::events::MsOperationProposedEvent {
        op_id,
        op_type: op_type_num,
        proposed_by: proposer,
        proposed_ledger: op.proposed_ledger,
    }
    .publish(env);

    op_id
}

/// A governor approves the given operation.
///
/// Once the N-th required approval is submitted, the timelock clock starts.
pub fn approve_operation(env: &Env, governor: Address, op_id: u32) {
    governor.require_auth();

    let governors = read_governors(env);
    if !governors.contains(&governor) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let mut op = read_op(env, op_id);

    // Prevent double-approval
    if op.approvals.contains(&governor) {
        panic_with_error!(env, ErrorCode::AlreadyApproved);
    }

    op.approvals.push_back(governor.clone());

    // Check if quorum just reached
    if op.approvals.len() >= op.required_approvals && op.timelock_start_ledger == 0 {
        op.timelock_start_ledger = env.ledger().sequence();
        crate::events::MsQuorumReachedEvent {
            op_id,
            approval_count: op.approvals.len(),
        }
        .publish(env);
    }

    write_op(env, &op);

    crate::events::MsOperationApprovedEvent {
        op_id,
        approver: governor.clone(),
    }
    .publish(env);
}

/// A governor retracts their previous approval.
///
/// If the timelock had started (quorum was previously met), retracting below
/// quorum resets `timelock_start_ledger` back to 0.
pub fn retract_approval(env: &Env, governor: Address, op_id: u32) {
    governor.require_auth();

    let governors = read_governors(env);
    if !governors.contains(&governor) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    let mut op = read_op(env, op_id);

    // Find and remove the governor's approval
    let mut found = false;
    let mut new_approvals: Vec<Address> = Vec::new(env);
    for i in 0..op.approvals.len() {
        let addr = op.approvals.get_unchecked(i);
        if addr == governor {
            found = true;
        } else {
            new_approvals.push_back(addr);
        }
    }
    if !found {
        panic_with_error!(env, ErrorCode::ApprovalNotFound);
    }
    op.approvals = new_approvals;

    // If approval count drops below quorum, pause the timelock
    if op.approvals.len() < op.required_approvals {
        op.timelock_start_ledger = 0;
    }

    write_op(env, &op);

    crate::events::MsOperationRetractedEvent {
        op_id,
        retracted_by: governor.clone(),
    }
    .publish(env);
}

/// Executes an operation that has reached quorum AND whose timelock has elapsed.
///
/// Enforces sequential execution order: only the current queue HEAD may be executed
/// when the queue is non-empty.
pub fn execute_ms_operation(env: &Env, executor: Address, op_id: u32) {
    executor.require_auth();

    // Executor must be admin or a governor
    let admin = get_admin(env);
    let governors = read_governors(env);
    if executor != admin && !governors.contains(&executor) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    // Ordered execution: this must be the queue head
    let head = read_queue_head(env);
    if head != op_id {
        panic_with_error!(env, ErrorCode::MsNotQueueHead);
    }

    let op = read_op(env, op_id);

    // Must have reached quorum
    if op.approvals.len() < op.required_approvals {
        panic_with_error!(env, ErrorCode::MsQuorumNotReached);
    }

    // Timelock must have started and elapsed
    if op.timelock_start_ledger == 0 {
        panic_with_error!(env, ErrorCode::TimelockNotReady);
    }

    let timelock_duration: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::CfgTimelockDuration)
        .unwrap_or(10);
    let elapsed = env.ledger().sequence() - op.timelock_start_ledger;
    if elapsed < timelock_duration {
        panic_with_error!(env, ErrorCode::TimelockNotReady);
    }

    // Advance queue head
    write_queue_head(env, op.next_op_id);
    if op.next_op_id == 0 {
        write_queue_tail(env, 0);
    }
    remove_op(env, op_id);

    let op_type_num = op_type_to_u32(&op.op_type);
    crate::events::MsOperationExecutedEvent {
        op_id,
        op_type: op_type_num,
        executed_by: executor,
    }
    .publish(env);
}

/// Cancels a pending multi-sig operation (admin or any governor may cancel).
pub fn cancel_ms_operation(env: &Env, canceller: Address, op_id: u32) {
    canceller.require_auth();

    let admin = get_admin(env);
    let governors = read_governors(env);
    if canceller != admin && !governors.contains(&canceller) {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    // Must exist
    let op = read_op(env, op_id);

    // Patch linked list: find predecessor
    let head = read_queue_head(env);
    if head == op_id {
        write_queue_head(env, op.next_op_id);
        if op.next_op_id == 0 {
            write_queue_tail(env, 0);
        }
    } else {
        // Walk to find predecessor
        let mut cursor = head;
        while cursor != 0 {
            let cur_op = read_op(env, cursor);
            if cur_op.next_op_id == op_id {
                let mut patched = cur_op.clone();
                patched.next_op_id = op.next_op_id;
                write_op(env, &patched);
                if op.next_op_id == 0 {
                    write_queue_tail(env, cursor);
                }
                break;
            }
            cursor = cur_op.next_op_id;
        }
    }

    remove_op(env, op_id);

    crate::events::MsOperationCancelledEvent {
        op_id,
        cancelled_by: canceller,
    }
    .publish(env);
}

/// Returns a pending multi-sig operation by ID.
pub fn get_ms_operation(env: &Env, op_id: u32) -> MultiSigOperation {
    read_op(env, op_id)
}

/// Returns the ID of the current queue head (0 if empty).
pub fn get_ms_queue_head(env: &Env) -> u32 {
    read_queue_head(env)
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn op_type_to_u32(op_type: &OperationType) -> u32 {
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
