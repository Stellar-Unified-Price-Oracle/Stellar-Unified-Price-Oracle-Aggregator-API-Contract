# Workshop 03 — Oracle Governance

> **Incident classes covered:** admin path exposure · privilege escalation · upgrade without review  
> **Contract revision:** pin to `main` at the time of delivery.

---

## Learning Objectives

1. Understand which functions are admin-only and what guards them.
2. Safely transfer admin rights using `set_admin()`.
3. Understand the upgrade path (`upgrade()`) and why it must be gated.
4. Identify governance attacks and the controls that prevent them.

---

## Setup

```bash
cargo test -p price-oracle --lib workshop_03
```

---

## Background: Admin-Gated Operations

The following operations require admin auth (`admin.require_auth()`):

| Function | Risk if unrestricted |
|---|---|
| `set_admin()` | Transfers full control to any address |
| `upgrade()` | Replaces contract WASM — total compromise |
| `add_source()` / `remove_source()` | Controls which feeds contribute to median |
| `register_asset()` / `unregister_asset()` | Adds or removes trackable assets |
| `set_min_sources_required()` | Lowering to 1 enables single-source manipulation |
| `set_interpolation_enabled()` | Enables price gaps to be bridged by interpolation |

**All admin functions in this contract call `admin.require_auth()`.**  
The labs demonstrate what happens if a consumer or integration assumes this is not enforced.

---

## Lab A — Admin Path Exposure (Unauthorized Upgrade Attempt)

**Real-world incident class:** A contract upgrade replaces legitimate logic with attacker-controlled
WASM. If the upgrade path is not properly auth-guarded, any caller can replace the contract.

### Vulnerable Pattern (in a hypothetical unguarded contract)

```rust
// VULNERABLE (hypothetical): upgrade without auth check
pub fn upgrade_unguarded(env: Env, new_wasm_hash: BytesN<32>) {
    // Missing: admin.require_auth()
    env.deployer().update_current_contract_wasm(new_wasm_hash);
}
```

### Scripted Attack

```rust
#[test]
fn lab_a_unauthorized_upgrade_attempt() {
    // Against the REAL contract: non-admin tries to call upgrade()
    // Must receive NotAuthorized error
    let env = Env::default();
    // ... initialize oracle with admin A ...
    // ... attacker B tries to call upgrade() ...
    let result = oracle_client.try_upgrade(&attacker_wasm_hash);
    assert!(result.is_err(), "exploit verified: upgrade must require admin auth");
    // Confirm the error is NotAuthorized (error code 0)
}
```

### Fixed Version (the real contract)

```rust
// CORRECT (actual contract implementation in lib.rs):
pub fn upgrade(env: Env, new_wasm_hash: BytesN<32>) {
    let admin = get_admin(&env);
    admin.require_auth(); // non-admin callers are rejected here
    env.deployer().update_current_contract_wasm(new_wasm_hash);
    emit_contract_upgraded(&env, new_wasm_hash);
}

#[test]
fn lab_a_upgrade_requires_admin() {
    // Only the admin can upgrade; any other address gets NotAuthorized
    // ... set up oracle with admin A and attacker B ...
    let result = oracle_client.try_upgrade(&attacker_wasm_hash);
    assert!(result.is_err(), "fix verified: non-admin upgrade rejected");

    // Admin can upgrade
    env.mock_all_auths();
    let result = oracle_client.try_upgrade(&legitimate_wasm_hash);
    assert!(result.is_ok(), "fix verified: admin upgrade accepted");
}
```

---

## Lab B — Privilege Escalation via `set_admin()`

**Real-world incident class:** An attacker who gains temporary access (e.g. compromised key) calls
`set_admin()` to permanently transfer admin rights before the compromise is detected.

### Scripted Attack Demonstration

```rust
#[test]
fn lab_b_admin_transfer_attack() {
    // Demonstrate that set_admin() requires current admin auth
    // Attacker (non-admin) tries to transfer admin rights to themselves
    let result = oracle_client.try_set_admin(&attacker_address);
    // Must fail with NotAuthorized
    assert!(result.is_err(), "non-admin cannot transfer admin rights");

    // Correct flow: current admin transfers to a new trusted address
    env.mock_all_auths();
    oracle_client.set_admin(&new_admin_address);
    let current_admin = oracle_client.get_admin_address();
    assert_eq!(current_admin, new_admin_address, "admin transfer succeeded correctly");
}
```

### Governance Best Practice

- Admin keys should be held in a multisig or hardware wallet.
- Admin transfers should go through a timelock (see `TimelockNotReady` error — error code 13).
- Any admin transfer should be announced on-chain via the `AdminChangedEvent`.

---

## Lab C — Source Manipulation via `set_min_sources_required()`

**Real-world incident class:** Admin (or attacker with admin key) lowers `min_sources` to 1,
enabling single-source manipulation even on an otherwise well-configured oracle.

```rust
#[test]
fn lab_c_min_sources_manipulation() {
    // Start with min_sources = 3 (safe configuration)
    // Attacker (with admin key) sets min_sources = 1
    env.mock_all_auths();
    oracle_client.set_min_sources_required(&1u32);

    // Now a single source submission satisfies aggregation
    // Source submits manipulated price
    oracle_client.submit_price(&source, &asset, &manipulated_price, &timestamp);

    let price = oracle_client.get_price(&asset);
    assert_eq!(price.price, manipulated_price, "exploit: single source controls median");

    // Governance lesson: monitor min_sources changes; alert on reductions
}
```

---

## Summary

| Lab | Exploit | Fix / Control |
|---|---|---|
| A | Unguarded upgrade path allows contract replacement | `admin.require_auth()` on every admin function |
| B | Compromised admin key transfers rights permanently | Multisig admin, timelock, on-chain `AdminChangedEvent` monitoring |
| C | `min_sources` reduced to 1 enables single-source manipulation | Monitor `set_min_sources_required()` calls; alert on reduction |

---

## Governance Checklist

Before operating or integrating with this oracle in production:

- [ ] Admin key is in a multisig or hardware wallet.
- [ ] `min_sources` is ≥ 3 for any financial use case.
- [ ] An alert is configured on `AdminChangedEvent` and `ContractUpgradedEvent`.
- [ ] Upgrade proposals go through a documented review process before on-chain execution.
- [ ] Source additions/removals are announced and reviewed before they affect the live median.

---

## References

- [Threat Model](threat-model.md)
- [Workshop 01 — Consuming Prices](workshop-01-consume-prices.md)
- [Workshop 02 — Becoming a Source](workshop-02-become-a-source.md)
- [Security Audit Checklist](../security-audit-checklist.md)
- [Governance Proposal Template](../governance-proposal-template.md)
