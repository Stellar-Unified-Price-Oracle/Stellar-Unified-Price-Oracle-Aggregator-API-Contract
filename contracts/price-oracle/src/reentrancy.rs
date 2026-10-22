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
    #[should_panic(expected = "Error(Contract, #25)")]
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

    // -----------------------------------------------------------------
    // Adversarial harness (#446): a victim that performs an effect and
    // then calls out to an attacker-supplied contract, which re-enters.
    // -----------------------------------------------------------------

    use soroban_sdk::{symbol_short, IntoVal, Symbol, Val, Vec as SVec};

    const COUNT: Symbol = symbol_short!("count");

    #[contract]
    struct Victim;

    #[contractimpl]
    impl Victim {
        /// Guarded: effect first, then interaction (CEI), wrapped in the guard.
        pub fn guarded(env: Env, callee: Address) {
            enter(&env);
            let n: u32 = env.storage().instance().get(&COUNT).unwrap_or(0);
            env.storage().instance().set(&COUNT, &(n + 1));
            let args: SVec<Val> = (env.current_contract_address(),).into_val(&env);
            env.invoke_contract::<()>(&callee, &symbol_short!("attack"), args);
            exit(&env);
        }

        /// Unguarded twin, used to show which layer blocks re-entry.
        pub fn unguarded(env: Env, callee: Address) {
            let n: u32 = env.storage().instance().get(&COUNT).unwrap_or(0);
            env.storage().instance().set(&COUNT, &(n + 1));
            let args: SVec<Val> = (env.current_contract_address(),).into_val(&env);
            env.invoke_contract::<()>(&callee, &symbol_short!("attack"), args);
        }

        pub fn count(env: Env) -> u32 {
            env.storage().instance().get(&COUNT).unwrap_or(0)
        }
    }

    const REENTERED: Symbol = symbol_short!("reentered");
    const TARGET: Symbol = symbol_short!("target");

    /// Re-enters the victim endpoint named by `TARGET`, recording success.
    #[contract]
    struct Attacker;

    #[contractimpl]
    impl Attacker {
        pub fn setup(env: Env, target: Symbol, next: Option<Address>) {
            env.storage().instance().set(&TARGET, &target);
            if let Some(n) = next {
                env.storage().instance().set(&symbol_short!("next"), &n);
            }
        }

        pub fn attack(env: Env, victim: Address) {
            let target: Symbol = env.storage().instance().get(&TARGET).unwrap();
            // Nested mode: forward through a second hostile contract first.
            let next: Option<Address> = env.storage().instance().get(&symbol_short!("next"));
            let callee = next.unwrap_or(env.current_contract_address());
            let args: SVec<Val> = (callee,).into_val(&env);
            let ok = matches!(
                env.try_invoke_contract::<(), soroban_sdk::Error>(&victim, &target, args),
                Ok(Ok(()))
            );
            env.storage().instance().set(&REENTERED, &ok);
        }

        pub fn reentered(env: Env) -> bool {
            env.storage().instance().get(&REENTERED).unwrap_or(false)
        }
    }

    fn setup_attack(env: &Env, target: &str, nested: bool) -> (Address, Address) {
        let victim = env.register(Victim, ());
        let attacker = env.register(Attacker, ());
        let target = Symbol::new(env, target);
        let next = if nested {
            let hop = env.register(Attacker, ());
            AttackerClient::new(env, &hop).setup(&target, &None);
            Some(hop)
        } else {
            None
        };
        AttackerClient::new(env, &attacker).setup(&target, &next);
        (victim, attacker)
    }

    #[test]
    fn test_reentry_into_guarded_endpoint_has_no_duplicate_effect() {
        let env = Env::default();
        let (victim, attacker) = setup_attack(&env, "guarded", false);
        VictimClient::new(&env, &victim).guarded(&attacker);
        assert!(!AttackerClient::new(&env, &attacker).reentered());
        assert_eq!(VictimClient::new(&env, &victim).count(), 1);
    }

    #[test]
    fn test_nested_multi_hop_reentry_is_blocked() {
        let env = Env::default();
        let (victim, attacker) = setup_attack(&env, "guarded", true);
        VictimClient::new(&env, &victim).guarded(&attacker);
        assert!(!AttackerClient::new(&env, &attacker).reentered());
        assert_eq!(VictimClient::new(&env, &victim).count(), 1);
    }

    #[test]
    fn test_host_blocks_reentry_even_without_guard() {
        // Soroban forbids a contract frame from being re-entered, so the
        // storage guard is defence-in-depth for cross-contract paths; it is
        // load-bearing for same-frame re-entry (see test_guard_reentrant_panics).
        let env = Env::default();
        let (victim, attacker) = setup_attack(&env, "unguarded", false);
        VictimClient::new(&env, &victim).unguarded(&attacker);
        assert!(!AttackerClient::new(&env, &attacker).reentered());
        assert_eq!(VictimClient::new(&env, &victim).count(), 1);
    }
}
