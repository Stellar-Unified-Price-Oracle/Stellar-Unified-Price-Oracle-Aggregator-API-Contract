use crate::types::{DataKey, ErrorCode, Operation, OperationStatus};
use soroban_sdk::{panic_with_error, Env, String, Vec};

pub fn create_operation(env: &Env, op_id: String, depends_on: Vec<String>) {
    let key = DataKey::Operation(op_id.clone());
    if env.storage().persistent().has(&key) {
        panic_with_error!(env, ErrorCode::OperationAlreadyExists);
    }
    let op = Operation {
        id: op_id.clone(),
        status: OperationStatus::Pending,
        depends_on: depends_on.clone(),
    };
    env.storage().persistent().set(&key, &op);

    // register this op as dependent for each dependency
    for i in 0..depends_on.len() {
        let dep = depends_on.get_unchecked(i);
        let dkey = DataKey::OperationDependents(dep.clone());
        let mut deps: Vec<String> = env
            .storage()
            .persistent()
            .get(&dkey)
            .unwrap_or(Vec::new(env));
        deps.push_back(op_id.clone());
        env.storage().persistent().set(&dkey, &deps);
    }
}

fn read_operation(env: &Env, op_id: &String) -> Operation {
    let key = DataKey::Operation(op_id.clone());
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::OperationNotFound))
}

pub fn get_operation_dependencies(env: &Env, op_id: String) -> Vec<String> {
    let op = read_operation(env, &op_id);
    op.depends_on
}

pub fn get_operation_status(env: &Env, op_id: String) -> OperationStatus {
    let op = read_operation(env, &op_id);
    op.status
}

pub fn execute_operation(env: &Env, op_id: String) {
    let mut op = read_operation(env, &op_id);
    if op.status != OperationStatus::Pending {
        panic_with_error!(env, ErrorCode::InvalidOperationState);
    }
    // ensure dependencies executed
    for i in 0..op.depends_on.len() {
        let dep_id = op.depends_on.get_unchecked(i);
        let dep = read_operation(env, &dep_id);
        if dep.status != OperationStatus::Executed {
            panic_with_error!(env, ErrorCode::DependencyNotMet);
        }
    }
    op.status = OperationStatus::Executed;
    env.storage()
        .persistent()
        .set(&DataKey::Operation(op_id.clone()), &op);
}

pub fn cancel_operation(env: &Env, op_id: String) {
    let mut op = read_operation(env, &op_id);
    if op.status == OperationStatus::Cancelled {
        return;
    }
    op.status = OperationStatus::Cancelled;
    env.storage()
        .persistent()
        .set(&DataKey::Operation(op_id.clone()), &op);

    // auto-cancel dependents
    let dkey = DataKey::OperationDependents(op_id.clone());
    let dependents: Vec<String> = env
        .storage()
        .persistent()
        .get(&dkey)
        .unwrap_or(Vec::new(env));
    for i in 0..dependents.len() {
        let dep = dependents.get_unchecked(i);
        // recursive cancel
        cancel_operation(env, dep.clone());
    }
}
