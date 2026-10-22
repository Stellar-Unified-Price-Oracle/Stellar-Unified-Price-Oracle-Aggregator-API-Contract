# Reentrancy Call-Graph Map (#446)

Harness: `contracts/price-oracle/src/reentrancy.rs` (`Victim` / `Attacker` contracts).

## Outbound call sites

| Site | Callee | Re-entry attempted | Status |
|---|---|---|---|
| `price_callback.rs` (subscriber notify) | consumer contract | submission / query endpoints | Blocked by host; `try_invoke_contract` isolates failures |
| `alerts.rs` (alert callback) | consumer contract | admin endpoints | Blocked by host |
| `cross_reference.rs` (reference read) | external oracle | submission endpoints | Blocked by host; read-only result |
| Token transfers (fees, bonds) | SAC token | any endpoint | Safe by construction (see below) |
| Deployer `update_current_contract_wasm` | host | n/a | Not a contract call |

## Guarantees and tests

- **Single-hop re-entry**: `test_reentry_into_guarded_endpoint_has_no_duplicate_effect`.
  The counter is incremented exactly once and the attacker's re-entry fails.
- **Nested / multi-hop** (A → attacker → hop → A):
  `test_nested_multi_hop_reentry_is_blocked`.
- **Guard load-bearing check**: `test_host_blocks_reentry_even_without_guard`
  shows that for cross-contract paths the Soroban host itself refuses to
  re-enter a contract already on the call stack. The storage guard
  (`enter`/`exit`) is therefore defence-in-depth there. It is load-bearing
  for same-frame re-entry, and `test_guard_reentrant_panics` fails if it is
  removed.
- **No duplicated effect**: every harness test asserts `count == 1`.

## Safe by construction

- **Soroban host re-entry prohibition.** A contract cannot be invoked while
  it is already on the call stack, so a callee cannot reach any oracle endpoint
  mid-operation.
- **SAC token transfers** do not call back into user contracts.
- **Checks-effects-interactions.** Outbound calls happen after state writes.
  If the whole invocation fails, the host rolls back all of its storage writes
  atomically, so no partial aggregate is ever observable.
