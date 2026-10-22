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
    // Snapshot the delay so a later `set_priority_delay` cannot shorten the
    // wait of an operation that is already queued (#455).
    env.storage().persistent().set(
        &DataKey::TlOpRequiredDelay(op_id),
        &get_priority_delay(env, &priority),
    );

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

    // Use priority-aware delay; never shorter than the delay snapshotted at proposal.
    let snapshot_key = DataKey::TlOpRequiredDelay(op_id);
    let snapshot: u32 = env.storage().persistent().get(&snapshot_key).unwrap_or(0);
    let required_delay = get_priority_delay(env, &pending_op.priority).max(snapshot);
    let current_ledger = env.ledger().sequence();
    let elapsed = current_ledger - pending_op.proposed_ledger;

    if elapsed < required_delay {
        panic_with_error!(env, ErrorCode::PriorityTimelockNotReady);
    }

    env.storage()
        .persistent()
        .remove(&DataKey::TlPendingOp(op_id));
    env.storage().persistent().remove(&snapshot_key);

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

/// Cancels a pending timelock operation before its execution point.
///
/// Operator procedure (#655):
/// * Any pending operation may be cancelled by the admin at any time while it
///   is still queued (i.e. before `execute_operation` succeeds).  Cancellation
///   is the primary safety lever when a queued action is found to be unsafe
///   during review.
/// * Cancellation is idempotent-safe: cancelling an unknown or already
///   executed/expired operation fails with `OperationNotFound` rather than
///   silently succeeding, so operators get a clear signal.
/// * On success the pending record and its snapshotted delay are removed and
///   an `OperationCancelledEvent` is emitted for downstream monitoring.
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
    env.storage()
        .persistent()
        .remove(&DataKey::TlOpRequiredDelay(op_id));

    OperationCancelledEvent {
        operation_id: op_id,
        op_type: op_type_to_num(&pending_op.op_type),
        cancelled_by: admin,
    }
    .publish(env);
}

// ---------------------------------------------------------------------------
// Expiry
// ---------------------------------------------------------------------------

/// Returns the ledger at which a pending operation expires if not executed.
///
/// Expiry is derived from the snapshotted required delay: an operation that
/// has not been executed within `required_delay` ledgers past its proposal
/// point is considered stale and must be re-proposed.  This bounds the window
/// in which a forgotten or abandoned action can be executed later.
pub fn operation_expiry_ledger(env: &Env, op_id: u32) -> Option<u32> {
    let pending_op: PendingOperation = env
        .storage()
        .persistent()
        .get(&DataKey::TlPendingOp(op_id))?;
    let snapshot: u32 = env
        .storage()
        .persistent()
        .get(&DataKey::TlOpRequiredDelay(op_id))
        .unwrap_or(0);
    let required_delay = get_priority_delay(env, &pending_op.priority).max(snapshot);
    Some(pending_op.proposed_ledger.saturating_add(required_delay))
}

/// Returns `true` if the pending operation has passed its expiry ledger.
pub fn is_operation_expired(env: &Env, op_id: u32) -> bool {
    match operation_expiry_ledger(env, op_id) {
        Some(expiry) => env.ledger().sequence() > expiry,
        None => false,
    }
}
