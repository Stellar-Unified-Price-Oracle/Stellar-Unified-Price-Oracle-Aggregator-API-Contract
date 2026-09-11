use soroban_sdk::{Address, Env, String};

use crate::types::{DataKey, GasRecord};

/// CPU instruction cost consumed so far.
///
/// The budget API is only available in test/testutils builds, so this returns
/// `0` when compiling the contract for WASM.
pub fn cpu_usage(env: &Env) -> u64 {
    #[cfg(any(test, feature = "testutils"))]
    {
        env.budget().cpu_instruction_cost()
    }
    #[cfg(not(any(test, feature = "testutils")))]
    {
        let _ = env;
        0
    }
}

/// Memory cost consumed so far.
///
/// The budget API is only available in test/testutils builds, so this returns
/// `0` when compiling the contract for WASM.
pub fn mem_usage(env: &Env) -> u64 {
    #[cfg(any(test, feature = "testutils"))]
    {
        env.budget().memory_bytes_cost()
    }
    #[cfg(not(any(test, feature = "testutils")))]
    {
        let _ = env;
        0
    }
}

pub fn write_last_gas(env: &Env, method: String, cpu: u64, mem: u64) {
    let record = GasRecord {
        method,
        cpu_instructions: cpu,
        memory_bytes: mem,
        ledger: env.ledger().sequence(),
        timestamp: env.ledger().timestamp(),
    };
    env.storage()
        .persistent()
        .set(&DataKey::LastGasRecord, &record);
}

pub fn read_last_gas(env: &Env) -> Option<GasRecord> {
    env.storage().persistent().get(&DataKey::LastGasRecord)
}
