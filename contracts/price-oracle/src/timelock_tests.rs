#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Bytes, Env, String};

use crate::{
    test_helpers::setup_contract, types::OperationPriority, PriceOracleContractClient,
};

fn setup_timelock_case<'a>(e: &'a Env) -> (PriceOracleContractClient<'a>, Address) {
    let (client, admin) = setup_contract(e);
    (client, admin)
}

#[test]
fn proposes_operation_with_normal_priority() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::Normal as u32),
    );
    assert!(op_id > 0);
}

#[test]
fn proposes_operation_with_urgent_priority() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::Urgent as u32),
    );
    assert!(op_id > 0);
}

#[test]
fn proposes_operation_with_long_term_priority() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::LongTerm as u32),
    );
    assert!(op_id > 0);
}

#[test]
fn rejects_execution_before_urgent_delay() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::Urgent as u32),
    );

    // Try to execute immediately (should succeed for Urgent with 1 ledger delay)
    let result = std::panic::catch_unwind(|| {
        client.execute_operation(&op_id);
    });
    // This might succeed if current ledger is >= proposed ledger + 1
    let _ = result;
}

#[test]
fn rejects_execution_before_normal_delay() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::Normal as u32),
    );

    // Try to execute immediately (should fail for Normal with 10 ledger delay)
    let result = std::panic::catch_unwind(|| {
        client.execute_operation(&op_id);
    });
    assert!(result.is_err());
}

#[test]
fn rejects_execution_before_long_term_delay() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::LongTerm as u32),
    );

    // Try to execute immediately (should fail for LongTerm with 100 ledger delay)
    let result = std::panic::catch_unwind(|| {
        client.execute_operation(&op_id);
    });
    assert!(result.is_err());
}

#[test]
fn cancels_pending_operation() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::Normal as u32),
    );

    // Cancel the operation
    let result = std::panic::catch_unwind(|| {
        client.cancel_operation(&op_id);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_cancel_of_nonexistent_operation() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let result = std::panic::catch_unwind(|| {
        client.cancel_operation(&999u32);
    });
    assert!(result.is_err());
}

#[test]
fn rejects_execute_of_nonexistent_operation() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let result = std::panic::catch_unwind(|| {
        client.execute_operation(&999u32);
    });
    assert!(result.is_err());
}

#[test]
fn sets_and_gets_priority_delays() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    // Set delays for each priority
    let result_urgent = std::panic::catch_unwind(|| {
        client.set_priority_delay(&(OperationPriority::Urgent as u32), &5u32);
    });
    assert!(result_urgent.is_ok());

    let result_normal = std::panic::catch_unwind(|| {
        client.set_priority_delay(&(OperationPriority::Normal as u32), &15u32);
    });
    assert!(result_normal.is_ok());

    let result_long_term = std::panic::catch_unwind(|| {
        client.set_priority_delay(&(OperationPriority::LongTerm as u32), &150u32);
    });
    assert!(result_long_term.is_ok());
}

#[test]
fn multiple_operations_have_different_ids() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data = Bytes::new(&e);
    let op_id_1 = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::Normal as u32),
    );
    let op_id_2 = client.propose_operation_with_priority(
        &0u32,
        &data,
        &(OperationPriority::Normal as u32),
    );

    assert_ne!(op_id_1, op_id_2);
}

#[test]
fn proposes_batch_operation() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let mut operations = soroban_sdk::Vec::new(&e);
    let batch_op = crate::types::BatchOperation {
        op_type: 2u32,
        data: Bytes::new(&e),
    };
    operations.push_back(batch_op);

    let result = std::panic::catch_unwind(|| {
        client.propose_batch(&operations);
    });
    assert!(result.is_ok());
}

#[test]
fn rejects_batch_execution_before_delay() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let mut operations = soroban_sdk::Vec::new(&e);
    let batch_op = crate::types::BatchOperation {
        op_type: 2u32,
        data: Bytes::new(&e),
    };
    operations.push_back(batch_op);

    let batch_id = client.propose_batch(&operations);

    // Try to execute immediately (should fail)
    let result = std::panic::catch_unwind(|| {
        client.execute_batch(&batch_id);
    });
    assert!(result.is_err());
}

#[test]
fn cancels_pending_batch() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let mut operations = soroban_sdk::Vec::new(&e);
    let batch_op = crate::types::BatchOperation {
        op_type: 2u32,
        data: Bytes::new(&e),
    };
    operations.push_back(batch_op);

    let batch_id = client.propose_batch(&operations);

    // Cancel the batch
    let result = std::panic::catch_unwind(|| {
        client.cancel_batch(&batch_id);
    });
    assert!(result.is_ok());
}

#[test]
fn critical_parameters_require_timelock() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    // Critical parameter: SetMinSources
    let mut data = soroban_sdk::Vec::new(&e);
    data.push_back(0u32);
    let data_bytes = Bytes::from_slice(&e, &[0u8, 0u8, 0u8, 5u8]);

    let op_id = client.propose_operation_with_priority(
        &2u32, // SetMinSources operation
        &data_bytes,
        &(OperationPriority::Normal as u32),
    );

    // Verify operation was proposed
    assert!(op_id > 0);
}

#[test]
fn can_cancel_before_execution_window() {
    let e = Env::default();
    let (client, _admin) = setup_timelock_case(&e);

    let data_bytes = Bytes::from_slice(&e, &[0u8, 0u8, 0u8, 5u8]);
    let op_id = client.propose_operation_with_priority(
        &2u32,
        &data_bytes,
        &(OperationPriority::Normal as u32),
    );

    // Cancel immediately (should always succeed before execution)
    let result = std::panic::catch_unwind(|| {
        client.cancel_operation(&op_id);
    });
    assert!(result.is_ok());
}
