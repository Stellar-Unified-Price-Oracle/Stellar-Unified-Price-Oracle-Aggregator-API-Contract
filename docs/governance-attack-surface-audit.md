# Cross-contract governance attack surface audit

Scope: `external_governance.rs` and `cross_contract_governance_tests.rs`
(issue #470). Tests: `issues_470_477_tests.rs`.

## Caused-action inventory

Every action the external governor can cause in this contract:

| Action | Mechanism | Authority needed |
|---|---|---|
| *(none)* | No endpoint reads `get_external_governor` or `is_governor_op_allowed` to authorize anything. | — |

The governor address and its allow-list are **advisory state**: they are
recorded and queryable, but no privileged oracle function accepts the governor
as an authorizer. Every mutating function in `external_governance.rs` requires
the **admin**. Consequently a captured governor contract cannot, by itself,
cause any state change in the oracle. If a future endpoint starts honoring the
allow-list it must be added to this table and covered by the tests below.

## Attack review

| # | Attack | Result | Test |
|---|---|---|---|
| 1 | Governor changes the parameters that govern governance (`set_external_governor`, `allow_governor_op`, `disallow_governor_op`, `clear_external_governor`, `reauthorize_governor`) | Not possible: all require admin auth; the governor is never the authorizer. | `governor_cannot_change_its_own_authority` |
| 2 | External contract upgrades to a hostile version and inherits trust | Grants are bound to an **authorization epoch**. Replacing/clearing the governor or calling `reauthorize_governor` (the admin's response to an external upgrade) starts a new epoch and invalidates every grant. The oracle cannot observe another contract's upgrade, so the admin must trigger this. | `governor_upgrade_requires_reauthorization`, `replacing_or_clearing_governor_drops_inherited_trust` |
| 3 | Callback re-entrancy through the governance boundary | The oracle makes no cross-contract calls into the governor, so there is no callback edge; the existing reentrancy guard covers the rest of the contract. Additionally the oracle's own address is rejected as governor (`InvalidConfiguration`) so it cannot delegate to itself. | `governor_cannot_be_the_oracle_itself` |
| 4 | Blocking by refusing to return / exhausting a shared budget | No call to the governor exists, so it cannot block or consume budget of unrelated oracle operations. | `unreachable_or_hostile_governor_does_not_block_oracle_operation` |
| 5 | Ambiguity over whose authority wins on an overlapping parameter | **Admin always wins.** The governor is subordinate: parameters are changed only under admin auth, and the admin can revoke the governor at any time. | `admin_outranks_governor_on_overlapping_parameters` |

## Residual trust and cost of compromise

* The admin remains the root of trust. Compromise of the admin compromises the
  oracle regardless of the governor.
* Compromise of the external governor **alone** costs nothing today (see the
  inventory). Once an endpoint honors the allow-list, compromise costs exactly
  the operations allow-listed in the current epoch — keep the list minimal.
* Detecting an upgrade of the external contract is an operational duty:
  the admin must call `reauthorize_governor` and re-allow the operations it
  still trusts. `get_governor_epoch` lets monitors confirm the reset.
* The security of the external governance contract's own implementation is out
  of scope.

The `multisig` module's "governors" (`ms_set_governors`) are a separate,
in-contract mechanism and are not affected by this audit.
