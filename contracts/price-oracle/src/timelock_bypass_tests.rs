//! # #455 — Timelock bypass and queue-manipulation attack suite
//!
//! The route map and findings are documented in
//! `docs/security/timelock-bypass-audit.md`.

use soroban_sdk::{testutils::Ledger, Bytes, Env};

use crate::test_helpers::*;
use crate::types::{BatchOperation, ErrorCode, OperationPriority};
use crate::PriceOracleContractClient;

const NORMAL: u32 = OperationPriority::Normal as u32;
const LONG_TERM: u32 = OperationPriority::LongTerm as u32;

fn advance(e: &Env, ledgers: u32) {
    let seq = e.ledger().sequence();
    e.ledger().with_mut(|l| l.sequence_number = seq + ledgers);
}

fn setup(e: &Env) -> PriceOracleContractClient<'_> {
    e.mock_all_auths();
    setup_contract(e).0
}

fn batch(e: &Env, ops: &[(u32, &[u8])]) -> soroban_sdk::Vec<BatchOperation> {
    let mut v = soroban_sdk::Vec::new(e);
    for (op_type, data) in ops {
        v.push_back(BatchOperation {
            op_type: *op_type,
            data: Bytes::from_slice(e, data),
        });
    }
    v
}

/// Attack (2): shortening the tier delay after queueing must not shorten the
/// wait of an already-queued operation.
#[test]
fn delay_shortening_does_not_affect_queued_op() {
    let e = Env::default();
    let client = setup(&e);
    let id = client.propose_operation_with_priority(&3u32, &Bytes::new(&e), &LONG_TERM);
    client.set_priority_delay(&LONG_TERM, &0u32);
    advance(&e, 1);
    let r = client.try_execute_operation(&id);
    assert_eq!(r, Err(Ok(code(ErrorCode::PriorityTimelockNotReady))));
    advance(&e, 99);
    client.execute_operation(&id);
}

/// Lengthening the tier delay still applies to queued operations.
#[test]
fn delay_lengthening_applies_to_queued_op() {
    let e = Env::default();
    let client = setup(&e);
    let id = client.propose_operation_with_priority(&3u32, &Bytes::new(&e), &NORMAL);
    client.set_priority_delay(&NORMAL, &50u32);
    advance(&e, 10);
    assert!(client.try_execute_operation(&id).is_err());
    advance(&e, 40);
    client.execute_operation(&id);
}

/// Attack (3): a mixed batch waits for its longest element (Upgrade → LongTerm).
#[test]
fn mixed_batch_enforces_longest_element_delay() {
    let e = Env::default();
    let client = setup(&e);
    let ops = batch(&e, &[(3, &[0, 0, 0, 50]), (0, &[7; 32])]);
    let id = client.propose_batch(&ops);
    advance(&e, 10);
    let r = client.try_execute_batch(&id);
    assert_eq!(r, Err(Ok(code(ErrorCode::TimelockNotReady))));
}

/// A short-delay-only batch keeps its own (short) delay.
#[test]
fn short_batch_executes_after_its_own_delay() {
    let e = Env::default();
    let client = setup(&e);
    let id = client.propose_batch(&batch(&e, &[(3, &[0, 0, 0, 50])]));
    advance(&e, 9);
    assert!(client.try_execute_batch(&id).is_err());
    advance(&e, 1);
    client.execute_batch(&id);
}

/// Shortening delays after a batch is queued does not shorten its wait.
#[test]
fn delay_shortening_does_not_affect_queued_batch() {
    let e = Env::default();
    let client = setup(&e);
    let id = client.propose_batch(&batch(&e, &[(3, &[0, 0, 0, 50]), (0, &[7; 32])]));
    client.set_priority_delay(&LONG_TERM, &0u32);
    advance(&e, 10);
    assert_eq!(
        client.try_execute_batch(&id),
        Err(Ok(code(ErrorCode::TimelockNotReady)))
    );
}

/// Attack (5): re-queueing yields a new id with a fresh clock; the original
/// cannot be executed once cancelled, and the new one waits the full delay.
#[test]
fn requeue_resets_clock_under_new_id() {
    let e = Env::default();
    let client = setup(&e);
    let first = client.propose_operation_with_priority(&3u32, &Bytes::new(&e), &NORMAL);
    advance(&e, 9);
    client.cancel_operation(&first);
    let second = client.propose_operation_with_priority(&3u32, &Bytes::new(&e), &NORMAL);
    assert_ne!(first, second);
    advance(&e, 1);
    assert_eq!(
        client.try_execute_operation(&first),
        Err(Ok(code(ErrorCode::OperationNotFound)))
    );
    assert!(client.try_execute_operation(&second).is_err());
    advance(&e, 9);
    client.execute_operation(&second);
}

/// Executed operations cannot be replayed.
#[test]
fn executed_operation_cannot_be_replayed() {
    let e = Env::default();
    let client = setup(&e);
    let id = client.propose_operation_with_priority(&3u32, &Bytes::new(&e), &NORMAL);
    advance(&e, 10);
    client.execute_operation(&id);
    assert_eq!(
        client.try_execute_operation(&id),
        Err(Ok(code(ErrorCode::OperationNotFound)))
    );
}

/// Attack (4): cancellation is admin-only.
#[test]
fn non_admin_cannot_cancel_queued_operation() {
    let e = Env::default();
    let client = setup(&e);
    let id = client.propose_operation_with_priority(&3u32, &Bytes::new(&e), &NORMAL);
    e.set_auths(&[]);
    assert!(client.try_cancel_operation(&id).is_err());
    assert!(client.try_cancel_batch(&id).is_err());
}

fn code(c: ErrorCode) -> soroban_sdk::Error {
    soroban_sdk::Error::from_contract_error(c as u32)
}
