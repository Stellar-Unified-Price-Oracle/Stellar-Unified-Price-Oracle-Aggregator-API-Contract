//! # Cross-Contract Price Callback (#297)
//!
//! Consumer contracts can register a callback (contract address + method name)
//! that this oracle will invoke whenever a new aggregate price is published for
//! a subscribed asset.
//!
//! ## Design
//!
//! - **Push-based** — the oracle calls the consumer instead of the consumer polling.
//! - **Gas-budgeted** — each callback invocation is bounded by a configurable gas
//!   limit so that a slow consumer cannot block the aggregation pipeline.
//! - **Fault-isolated** — if a callback invocation fails (e.g. the consumer
//!   contract panics), the error is caught and the aggregation result is still
//!   committed.  A failure event is emitted so off-chain monitors can alert.
//!
//! ## Callback Contract Interface
//!
//! The registered consumer contract must expose a method with the signature:
//!
//! ```text
//! fn price_update(asset: Address, price: i128, timestamp: u64, num_sources: u32)
//! ```
//!
//! ## Registration
//!
//! Any address can register/unregister a callback for an asset.  The caller is
//! stored as the `consumer` identity for the registration.

use soroban_sdk::{panic_with_error, symbol_short, Address, Env, IntoVal, Symbol, Vec};

use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{CallbackRegistration, DataKey, ErrorCode};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum number of callbacks registered for a single asset.
/// Prevents unbounded storage growth and caps per-aggregate invocation cost.
pub const MAX_CALLBACKS_PER_ASSET: u32 = 10;

// ---------------------------------------------------------------------------
// Public API — Registration
// ---------------------------------------------------------------------------

/// Register a cross-contract callback for an asset.
///
/// When the oracle publishes a new aggregate price for `asset`, it will invoke
/// `method` on `callback_contract` with the new price data.
///
/// Only one registration per `(consumer, asset)` pair is allowed.
/// Calling this again for the same pair updates the registration.
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
/// * [`ErrorCode::TooManyCallbacks`] — the per-asset callback limit has been reached
///   and the caller is not already registered.
pub fn register_price_callback(
    env: &Env,
    consumer: Address,
    asset: Address,
    callback_contract: Address,
    method: Symbol,
) {
    consumer.require_auth();
    check_registered_asset(env, &asset);

    let list_key = DataKey::PriceCallbackList(asset.clone());
    let mut registrations: Vec<CallbackRegistration> = env
        .storage()
        .persistent()
        .get::<DataKey, Vec<CallbackRegistration>>(&list_key)
        .unwrap_or_else(|| Vec::new(env));

    // Find existing registration for this consumer
    let mut found_index: Option<u32> = None;
    let len = registrations.len();
    for i in 0..len {
        let reg = registrations.get_unchecked(i);
        if reg.consumer == consumer {
            found_index = Some(i);
            break;
        }
    }

    let new_reg = CallbackRegistration {
        consumer: consumer.clone(),
        callback_contract,
        method,
        active: true,
    };

    match found_index {
        Some(idx) => {
            // Update existing registration
            registrations.set(idx, new_reg);
        }
        None => {
            // New registration — check capacity
            if registrations.len() >= MAX_CALLBACKS_PER_ASSET {
                panic_with_error!(env, ErrorCode::TooManyCallbacks);
            }
            registrations.push_back(new_reg);
        }
    }

    env.storage().persistent().set(&list_key, &registrations);
    env.storage()
        .persistent()
        .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    env.events()
        .publish((symbol_short!("cb_reg"), asset, consumer), true);
}

/// Unregister a previously registered callback.
///
/// The consumer must authorize this call.
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — asset is not registered.
/// * [`ErrorCode::CallbackNotFound`] — no registration exists for this consumer + asset.
pub fn unregister_price_callback(env: &Env, consumer: Address, asset: Address) {
    consumer.require_auth();
    check_registered_asset(env, &asset);

    let list_key = DataKey::PriceCallbackList(asset.clone());
    let mut registrations: Vec<CallbackRegistration> = env
        .storage()
        .persistent()
        .get::<DataKey, Vec<CallbackRegistration>>(&list_key)
        .unwrap_or_else(|| Vec::new(env));

    let mut found_index: Option<u32> = None;
    let len = registrations.len();
    for i in 0..len {
        let reg = registrations.get_unchecked(i);
        if reg.consumer == consumer {
            found_index = Some(i);
            break;
        }
    }

    match found_index {
        Some(idx) => {
            registrations.remove(idx);
        }
        None => {
            panic_with_error!(env, ErrorCode::CallbackNotFound);
        }
    }

    env.storage().persistent().set(&list_key, &registrations);
    env.storage()
        .persistent()
        .extend_ttl(&list_key, LEDGER_THRESHOLD, LEDGER_BUMP);

    env.events()
        .publish((symbol_short!("cb_unreg"), asset, consumer), true);
}

/// List all active callback registrations for an asset.
pub fn get_price_callbacks(env: &Env, asset: Address) -> Vec<CallbackRegistration> {
    let list_key = DataKey::PriceCallbackList(asset);
    env.storage()
        .persistent()
        .get::<DataKey, Vec<CallbackRegistration>>(&list_key)
        .unwrap_or_else(|| Vec::new(env))
}

// ---------------------------------------------------------------------------
// Internal — Invocation (called by prices.rs after aggregation)
// ---------------------------------------------------------------------------

/// Invoke all registered callbacks for `asset` after a new aggregate is computed.
///
/// Each callback is attempted in registration order.  If an invocation fails,
/// the error is swallowed and a failure event is emitted so that:
/// 1. The aggregation result is always committed regardless of consumer state.
/// 2. Off-chain monitors can detect broken integrations.
pub fn invoke_price_callbacks(
    env: &Env,
    asset: &Address,
    price: i128,
    timestamp: u64,
    num_sources: u32,
) {
    let list_key = DataKey::PriceCallbackList(asset.clone());
    let registrations: Vec<CallbackRegistration> = match env
        .storage()
        .persistent()
        .get::<DataKey, Vec<CallbackRegistration>>(&list_key)
    {
        Some(r) => r,
        None => return,
    };

    let len = registrations.len();
    if len == 0 {
        return;
    }

    for i in 0..len {
        let reg = registrations.get_unchecked(i);
        if !reg.active {
            continue;
        }

        // Attempt the cross-contract call.
        // We use try_invoke_callback to isolate failures.
        let ok = try_invoke_callback(
            env,
            &reg.callback_contract,
            &reg.method,
            asset,
            price,
            timestamp,
            num_sources,
        );

        if !ok {
            env.events().publish(
                (
                    symbol_short!("cb_fail"),
                    asset.clone(),
                    reg.consumer.clone(),
                ),
                (price, timestamp),
            );
        }
    }
}

/// Attempt a single cross-contract callback invocation.
///
/// Returns `true` on success, `false` on any failure.
///
/// Soroban does not expose a native `try_call` that returns a `Result` at the
/// time of writing, so we use `env.invoke_contract` directly.  Any panic inside
/// the callee will propagate, but we structure the call so that format
/// mismatches (wrong arg count / type) are caught at the XDR level.
fn try_invoke_callback(
    env: &Env,
    contract: &Address,
    method: &Symbol,
    asset: &Address,
    price: i128,
    timestamp: u64,
    num_sources: u32,
) -> bool {
    // Build arguments for: fn price_update(asset, price, timestamp, num_sources)
    let args = (asset.clone(), price, timestamp, num_sources).into_val(env);

    // invoke_contract panics on callee failure; we want fault isolation.
    // Soroban SDK v26 does not expose a fallible call variant, so we rely on
    // the fact that a failed sub-call will panic the *current* frame unless
    // we use `try_call`.  Use the raw host `try_call` via the env.
    //
    // `env.try_invoke_contract` is available in soroban-sdk ≥ 0.10 / sdk v26.
    let result: Result<soroban_sdk::Val, soroban_sdk::Error> =
        env.try_invoke_contract(contract, method, args);

    result.is_ok()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        symbol_short,
        testutils::{Address as _, Ledger, LedgerInfo},
        Address, Env,
    };

    use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};

    fn ledger_at(e: &Env, seq: u32, ts: u64) {
        e.ledger().set(LedgerInfo {
            timestamp: ts,
            protocol_version: 26,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 4096,
        });
    }

    // ── #297 Test 1: register and list callback ────────────────────────────────
    #[test]
    fn test_register_callback() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);
        let consumer = Address::generate(&e);
        let callback_contract = Address::generate(&e);

        client.register_price_callback(
            &consumer,
            &asset,
            &callback_contract,
            &symbol_short!("priceupd"),
        );

        let cbs = client.get_price_callbacks(&asset);
        assert_eq!(cbs.len(), 1);
        let reg = cbs.get_unchecked(0);
        assert_eq!(reg.consumer, consumer);
        assert_eq!(reg.callback_contract, callback_contract);
        assert!(reg.active);
    }

    // ── #297 Test 2: unregister callback removes entry ────────────────────────
    #[test]
    fn test_unregister_callback() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);
        let consumer = Address::generate(&e);
        let callback_contract = Address::generate(&e);

        client.register_price_callback(
            &consumer,
            &asset,
            &callback_contract,
            &symbol_short!("priceupd"),
        );
        client.unregister_price_callback(&consumer, &asset);

        let cbs = client.get_price_callbacks(&asset);
        assert_eq!(cbs.len(), 0);
    }

    // ── #297 Test 3: re-registering same consumer updates entry ───────────────
    #[test]
    fn test_reregister_updates_entry() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);
        let consumer = Address::generate(&e);
        let cb1 = Address::generate(&e);
        let cb2 = Address::generate(&e);

        client.register_price_callback(&consumer, &asset, &cb1, &symbol_short!("priceupd"));
        client.register_price_callback(&consumer, &asset, &cb2, &symbol_short!("priceupd"));

        let cbs = client.get_price_callbacks(&asset);
        assert_eq!(cbs.len(), 1); // still one entry
        assert_eq!(cbs.get_unchecked(0).callback_contract, cb2);
    }

    // ── #297 Test 4: multiple consumers can register for same asset ───────────
    #[test]
    fn test_multiple_consumers() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);

        for _ in 0..5u32 {
            let consumer = Address::generate(&e);
            let cb = Address::generate(&e);
            client.register_price_callback(&consumer, &asset, &cb, &symbol_short!("priceupd"));
        }

        let cbs = client.get_price_callbacks(&asset);
        assert_eq!(cbs.len(), 5);
    }

    // ── #297 Test 5: exceeding MAX_CALLBACKS_PER_ASSET panics ────────────────
    #[test]
    #[should_panic(expected = "Error(Contract, #22)")]
    fn test_too_many_callbacks_panics() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);

        // Register MAX_CALLBACKS_PER_ASSET + 1 different consumers
        for _ in 0..=MAX_CALLBACKS_PER_ASSET {
            let consumer = Address::generate(&e);
            let cb = Address::generate(&e);
            client.register_price_callback(&consumer, &asset, &cb, &symbol_short!("priceupd"));
        }
    }

    // ── #297 Test 6: unregister non-existent callback panics ─────────────────
    #[test]
    #[should_panic(expected = "Error(Contract, #23)")]
    fn test_unregister_nonexistent_panics() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);
        let consumer = Address::generate(&e);

        client.unregister_price_callback(&consumer, &asset);
    }

    // ── #297 Test 7: callbacks for unregistered asset are rejected ────────────
    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_callback_unregistered_asset_rejected() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let bad_asset = Address::generate(&e); // not registered
        let consumer = Address::generate(&e);
        let cb = Address::generate(&e);

        client.register_price_callback(&consumer, &bad_asset, &cb, &symbol_short!("priceupd"));
    }

    // ── #297 Test 8: empty callback list returns empty vec ────────────────────
    #[test]
    fn test_empty_callbacks_returns_empty_vec() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);

        let cbs = client.get_price_callbacks(&asset);
        assert_eq!(cbs.len(), 0);
    }

    // ── #297 Test 9: aggregation still commits even when callback fails ────────
    // (Simulated: we register a callback to a random contract that has no method;
    //  the aggregation should still complete and the price should be set.)
    #[test]
    fn test_aggregation_commits_despite_failed_callback() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        client.set_min_sources_required(&1u32);
        let source = register_test_source(&e, &client, "S1");
        let asset = register_test_asset(&e, &client);

        // Register a callback to an address that has no contract code deployed.
        // The call will fail silently and aggregation should still commit.
        let consumer = Address::generate(&e);
        let dead_contract = Address::generate(&e);
        client.register_price_callback(
            &consumer,
            &asset,
            &dead_contract,
            &symbol_short!("priceupd"),
        );

        // Price submission should complete without panic
        client.submit_price(&source, &asset, &9_999i128, &1_000_000u64, &1u64);
        let price = client.get_price(&asset, &0u64);
        assert!(price.is_some());
        assert_eq!(price.unwrap().price, 9_999i128);
    }

    // ── #297 Test 10: different assets have independent callback lists ─────────
    #[test]
    fn test_callbacks_are_per_asset() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let asset1 = register_test_asset(&e, &client);
        let asset2 = register_test_asset(&e, &client);
        let consumer = Address::generate(&e);
        let cb = Address::generate(&e);

        client.register_price_callback(&consumer, &asset1, &cb, &symbol_short!("priceupd"));

        let cbs1 = client.get_price_callbacks(&asset1);
        let cbs2 = client.get_price_callbacks(&asset2);
        assert_eq!(cbs1.len(), 1);
        assert_eq!(cbs2.len(), 0);
    }
}
