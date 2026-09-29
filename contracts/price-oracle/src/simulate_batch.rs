//! # Batch Dry-Run Simulation
//!
//! `simulate_batch` evaluates a list of [`BatchOperation`]s and returns a
//! [`BatchSimulationResult`] that describes what *would* happen if the batch
//! were executed — without committing any state changes.
//!
//! ## What it checks
//!
//! For each operation the simulator:
//!
//! 1. Validates the `op_type` discriminant.
//! 2. Checks that the encoded `data` payload is long enough for the operation.
//! 3. Decodes the would-be parameter value and flags extreme or security-relevant
//!    settings with a [`SimulationWarning`].
//! 4. Marks the operation as `would_succeed = true` when no hard error is found.
//!
//! The result also includes aggregate counters so callers can quickly decide
//! whether to submit the batch.

use soroban_sdk::{Env, String, Vec};

use crate::types::{
    BatchOperation, BatchSimulationResult, OperationSimulationResult, SimulationWarning,
};

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Decode a big-endian u32 from the first 4 bytes of `data`.
/// Returns `None` if `data` is shorter than 4 bytes.
fn decode_u32(data: &soroban_sdk::Bytes) -> Option<u32> {
    if data.len() < 4 {
        return None;
    }
    let mut arr = [0u8; 4];
    for i in 0..4u32 {
        arr[i as usize] = data.get_unchecked(i);
    }
    Some(u32::from_be_bytes(arr))
}

/// Simulate a single `BatchOperation` and return its result.
fn simulate_single(env: &Env, index: u32, op: &BatchOperation) -> OperationSimulationResult {
    match op.op_type {
        0 => {
            // Upgrade — data must be ≥32 bytes (WASM hash).
            if op.data.len() < 32 {
                return OperationSimulationResult {
                    index,
                    op_type: 0,
                    description: String::from_str(env, "Upgrade: data too short for WASM hash"),
                    would_succeed: false,
                    warning: SimulationWarning::InvalidData,
                };
            }
            OperationSimulationResult {
                index,
                op_type: 0,
                description: String::from_str(env, "Upgrade: would update contract WASM"),
                would_succeed: true,
                warning: SimulationWarning::None,
            }
        }
        1 => {
            // SetAdmin — no payload validation needed (address encoded externally).
            OperationSimulationResult {
                index,
                op_type: 1,
                description: String::from_str(env, "SetAdmin: would transfer admin rights"),
                would_succeed: true,
                warning: SimulationWarning::None,
            }
        }
        2 => {
            // SetMinSources — warn if < 2 (weakens security).
            match decode_u32(&op.data) {
                None => OperationSimulationResult {
                    index,
                    op_type: 2,
                    description: String::from_str(env, "SetMinSources: data too short"),
                    would_succeed: false,
                    warning: SimulationWarning::InvalidData,
                },
                Some(val) => {
                    let warning = if val < 2 {
                        SimulationWarning::LowMinSources
                    } else {
                        SimulationWarning::None
                    };
                    OperationSimulationResult {
                        index,
                        op_type: 2,
                        description: String::from_str(
                            env,
                            "SetMinSources: would update min sources",
                        ),
                        would_succeed: true,
                        warning,
                    }
                }
            }
        }
        3 => {
            // SetMaxHistory — warn if very large.
            match decode_u32(&op.data) {
                None => OperationSimulationResult {
                    index,
                    op_type: 3,
                    description: String::from_str(env, "SetMaxHistory: data too short"),
                    would_succeed: false,
                    warning: SimulationWarning::InvalidData,
                },
                Some(val) => {
                    let warning = if val > 10_000 {
                        SimulationWarning::LargeHistory
                    } else {
                        SimulationWarning::None
                    };
                    OperationSimulationResult {
                        index,
                        op_type: 3,
                        description: String::from_str(
                            env,
                            "SetMaxHistory: would update max history length",
                        ),
                        would_succeed: true,
                        warning,
                    }
                }
            }
        }
        4 => {
            // SetResolution
            match decode_u32(&op.data) {
                None => OperationSimulationResult {
                    index,
                    op_type: 4,
                    description: String::from_str(env, "SetResolution: data too short"),
                    would_succeed: false,
                    warning: SimulationWarning::InvalidData,
                },
                Some(val) => {
                    let warning = if val > 86_400 {
                        // > 1 day in seconds is extreme
                        SimulationWarning::ExtremeValue
                    } else {
                        SimulationWarning::None
                    };
                    OperationSimulationResult {
                        index,
                        op_type: 4,
                        description: String::from_str(
                            env,
                            "SetResolution: would update resolution window",
                        ),
                        would_succeed: true,
                        warning,
                    }
                }
            }
        }
        5 => {
            // SetDecimals
            match decode_u32(&op.data) {
                None => OperationSimulationResult {
                    index,
                    op_type: 5,
                    description: String::from_str(env, "SetDecimals: data too short"),
                    would_succeed: false,
                    warning: SimulationWarning::InvalidData,
                },
                Some(val) => {
                    let warning = if val > 38 {
                        SimulationWarning::ExtremeValue
                    } else {
                        SimulationWarning::None
                    };
                    OperationSimulationResult {
                        index,
                        op_type: 5,
                        description: String::from_str(
                            env,
                            "SetDecimals: would update decimal precision",
                        ),
                        would_succeed: true,
                        warning,
                    }
                }
            }
        }
        6 => {
            // SetDescription — no hard validation (arbitrary bytes used as description)
            OperationSimulationResult {
                index,
                op_type: 6,
                description: String::from_str(
                    env,
                    "SetDescription: would update contract description",
                ),
                would_succeed: true,
                warning: SimulationWarning::None,
            }
        }
        7 => {
            // SetTimestampThreshold — needs ≥8 bytes (u64)
            if op.data.len() < 8 {
                return OperationSimulationResult {
                    index,
                    op_type: 7,
                    description: String::from_str(env, "SetTimestampThreshold: data too short"),
                    would_succeed: false,
                    warning: SimulationWarning::InvalidData,
                };
            }
            OperationSimulationResult {
                index,
                op_type: 7,
                description: String::from_str(
                    env,
                    "SetTimestampThreshold: would update timestamp threshold",
                ),
                would_succeed: true,
                warning: SimulationWarning::None,
            }
        }
        _ => OperationSimulationResult {
            index,
            op_type: op.op_type,
            description: String::from_str(env, "Unknown op_type: would fail at execution"),
            would_succeed: false,
            warning: SimulationWarning::UnknownOpType,
        },
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Simulates `operations` and returns a [`BatchSimulationResult`] without
/// committing any state changes.
pub fn simulate_batch(env: &Env, operations: Vec<BatchOperation>) -> BatchSimulationResult {
    let total_ops = operations.len();
    let mut results: Vec<OperationSimulationResult> = Vec::new(env);
    let mut would_succeed_count: u32 = 0;
    let mut warning_count: u32 = 0;

    for i in 0..total_ops {
        let op = operations.get_unchecked(i);
        let result = simulate_single(env, i, &op);

        if result.would_succeed {
            would_succeed_count += 1;
        }
        let has_warning = !matches!(result.warning, SimulationWarning::None);
        if has_warning {
            warning_count += 1;
        }
        results.push_back(result);
    }

    let all_succeed = would_succeed_count == total_ops;

    BatchSimulationResult {
        results,
        total_ops,
        would_succeed_count,
        warning_count,
        all_succeed,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Bytes, Env};

    use crate::test_helpers::setup_contract;

    fn make_ops(e: &Env, tuples: &[(u32, &[u8])]) -> Vec<BatchOperation> {
        let mut v: Vec<BatchOperation> = Vec::new(e);
        for (op_type, raw) in tuples {
            let mut b = Bytes::new(e);
            for byte in *raw {
                b.push_back(*byte);
            }
            v.push_back(BatchOperation {
                op_type: *op_type,
                data: b,
            });
        }
        v
    }

    #[test]
    fn test_simulate_empty_batch() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        let ops: Vec<BatchOperation> = Vec::new(&e);
        let result = client.simulate_batch(&ops);
        assert_eq!(result.total_ops, 0);
        assert_eq!(result.would_succeed_count, 0);
        assert!(result.all_succeed);
    }

    #[test]
    fn test_simulate_valid_set_min_sources() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        // op_type=2, value=3 (big-endian u32)
        let ops = make_ops(&e, &[(2u32, &[0, 0, 0, 3])]);
        let result = client.simulate_batch(&ops);
        assert_eq!(result.total_ops, 1);
        assert_eq!(result.would_succeed_count, 1);
        assert_eq!(result.warning_count, 0);
        assert!(result.all_succeed);
    }

    #[test]
    fn test_simulate_warns_low_min_sources() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        // op_type=2, value=1 → LowMinSources warning
        let ops = make_ops(&e, &[(2u32, &[0, 0, 0, 1])]);
        let result = client.simulate_batch(&ops);
        assert_eq!(result.total_ops, 1);
        assert_eq!(result.would_succeed_count, 1); // succeeds but warns
        assert_eq!(result.warning_count, 1);
        assert!(result.all_succeed);
    }

    #[test]
    fn test_simulate_fails_invalid_data() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        // op_type=2, data too short (only 2 bytes)
        let ops = make_ops(&e, &[(2u32, &[0, 0])]);
        let result = client.simulate_batch(&ops);
        assert_eq!(result.total_ops, 1);
        assert_eq!(result.would_succeed_count, 0);
        assert!(!result.all_succeed);
    }

    #[test]
    fn test_simulate_unknown_op_type() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        let ops = make_ops(&e, &[(99u32, &[])]);
        let result = client.simulate_batch(&ops);
        assert_eq!(result.would_succeed_count, 0);
        assert!(!result.all_succeed);
        assert_eq!(result.warning_count, 1);
    }

    #[test]
    fn test_simulate_mixed_batch() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        // op 0: valid SetMinSources(5), op 1: invalid (no data), op 2: valid SetDecimals(18)
        let ops = make_ops(
            &e,
            &[
                (2u32, &[0, 0, 0, 5]),
                (2u32, &[]), // too short
                (5u32, &[0, 0, 0, 18]),
            ],
        );
        let result = client.simulate_batch(&ops);
        assert_eq!(result.total_ops, 3);
        assert_eq!(result.would_succeed_count, 2);
        assert!(!result.all_succeed);
    }

    #[test]
    fn test_simulate_upgrade_needs_32_bytes() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        // op_type=0, exactly 32 zero bytes
        let ops = make_ops(&e, &[(0u32, &[0u8; 32])]);
        let result = client.simulate_batch(&ops);
        assert_eq!(result.would_succeed_count, 1);
        assert!(result.all_succeed);
    }

    #[test]
    fn test_simulate_upgrade_short_data_fails() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        let ops = make_ops(&e, &[(0u32, &[0u8; 10])]);
        let result = client.simulate_batch(&ops);
        assert_eq!(result.would_succeed_count, 0);
        assert!(!result.all_succeed);
    }

    /// Verifies that simulate_batch does NOT actually change contract state.
    #[test]
    fn test_simulate_does_not_mutate_state() {
        let e = Env::default();
        let (client, _) = setup_contract(&e);
        let before = client.get_min_sources_required();

        // Simulate setting min sources to 99
        let ops = make_ops(&e, &[(2u32, &[0, 0, 0, 99])]);
        let result = client.simulate_batch(&ops);
        assert!(result.all_succeed);

        // Actual state must be unchanged
        let after = client.get_min_sources_required();
        assert_eq!(before, after);
    }
}
