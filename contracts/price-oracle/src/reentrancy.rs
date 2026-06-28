use soroban_sdk::{panic_with_error, Env};

use crate::types::{DataKey, ErrorCode};

/// Marks the contract as entered. Panics with `Reentrant` if already entered.
pub fn enter(env: &Env) {
    if env
        .storage()
        .temporary()
        .get::<_, bool>(&DataKey::ReentrancyGuard)
        .unwrap_or(false)
    {
        panic_with_error!(env, ErrorCode::Reentrant);
    }
    env.storage()
        .temporary()
        .set(&DataKey::ReentrancyGuard, &true);
}

/// Clears the reentrancy guard after the function body completes.
pub fn exit(env: &Env) {
    env.storage().temporary().remove(&DataKey::ReentrancyGuard);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{contract, contractimpl, testutils::Address as _, Address, Env};

    #[contract]
    struct TestContract;

    #[contractimpl]
    impl TestContract {
        pub fn test_enter_exit(_env: Env) {}
        pub fn test_double_enter(env: Env) {
            enter(&env);
            enter(&env); // should panic
        }
    }

    #[test]
    fn test_guard_enter_exit_normal() {
        let env = Env::default();
        let id = env.register(TestContract, ());
        // Use as_contract to access storage within a contract context
        env.as_contract(&id, || {
            enter(&env);
            exit(&env);
            // No panic means success
        });
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #16)")]
    fn test_guard_reentrant_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register(TestContract, ());
        // Manually set the guard flag and then call enter to trigger Reentrant
        env.as_contract(&id, || {
            enter(&env); // sets flag
            enter(&env); // panics with Reentrant
        });
    }

    #[test]
    fn test_guard_cleared_after_exit() {
        let env = Env::default();
        let id = env.register(TestContract, ());
        env.as_contract(&id, || {
            enter(&env);
            exit(&env);
            // After exit, entering again should not panic
            enter(&env);
            exit(&env);
        });
    }
}
