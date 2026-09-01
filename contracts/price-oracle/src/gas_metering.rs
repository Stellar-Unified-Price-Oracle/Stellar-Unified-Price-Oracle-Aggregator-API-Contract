use soroban_sdk::{Address, Env, String};

use crate::types::{DataKey, GasRecord};

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
