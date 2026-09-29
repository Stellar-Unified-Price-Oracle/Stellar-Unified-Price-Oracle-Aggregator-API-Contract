# Capability Matrix — Role × Authority (#509)

Least privilege is only credible with a matrix that is **checked against the
source**, not maintained by hand alongside it. This document is that matrix, and
`contracts/price-oracle/src/capability_matrix_tests.rs` fails if it drifts from
the code.

Endpoint-level authority (which of `admin` / `caller:<arg>` / `public-read` / …
guards each entrypoint) is already tracked separately in
[`endpoint-authority-matrix.md`](endpoint-authority-matrix.md). This document
covers the question that one does not: **which role holds which capability, and
where the model has intentional exceptions.**

## Roles

The five delegated roles of `types::Role`:

| Role | Discriminant | Purpose |
|---|---|---|
| `SourceManager` | 0 | admit and remove oracle sources |
| `AssetManager` | 1 | register and unregister assets |
| `PriceUpdater` | 2 | submit and correct prices |
| `ConfigManager` | 3 | change global configuration |
| `UpgradeManager` | 4 | upgrade the contract and transfer admin |

## Role × capability matrix

`—` means the role confers nothing on that capability. Read this as: the only
way to exercise the right-hand column is through the guard named in it.

| Capability | `SourceManager` | `AssetManager` | `PriceUpdater` | `ConfigManager` | `UpgradeManager` | Guard |
|---|---|---|---|---|---|---|
| add / remove oracle source | ✅ | — | — | — | — | `admin.require_auth` + source registry |
| register / unregister asset | — | ✅ | — | — | — | `admin.require_auth` + asset registry |
| submit a price | — | — | ✅ | — | — | `source.require_auth` + source registry |
| correct a published price | — | — | ✅ | — | — | `admin.require_auth` **and** `rbac::has_role(PriceUpdater)` |
| change global config | — | — | — | ✅ | — | `admin.require_auth` + config bounds |
| upgrade / transfer admin | — | — | — | — | ✅ | `admin.require_auth` |
| delegate / revoke a role | — | — | — | — | — | `admin.require_auth` (admin only, no role path) |

**The matrix is deliberately sparse, and that is the finding.** Delegated roles
are a *delegation* mechanism layered on top of admin-gated endpoints, not a
replacement for them. Only one capability — price correction — actually consults
`rbac::has_role` at runtime; every other role is currently a bookkeeping record
that `delegate_role` writes and `has_role` reports but that no endpoint gate
consults. Adding a role check to an endpoint is therefore a **narrowing** of
that endpoint's reachable authority set, never a widening, which is what makes
this matrix safe to extend incrementally.

### Why `PriceUpdater` on `correct_price` is the one alias, and why it is safe

`correct_price` is the only capability with two guards. Both must pass:

1. `admin.require_auth` — the caller must be the current admin; and
2. `rbac::has_role(env, &admin, Role::PriceUpdater)` — the admin must hold the
   `PriceUpdater` role.

Because `has_role` returns `true` unconditionally for the admin, guard 2 is
currently *inert*: the admin holds every role by construction. It is kept because
it documents intent, and because it becomes load-bearing the moment the admin's
implicit all-roles rule is narrowed. The alias is therefore an **admin-gated
path that shadows a role capability**, recorded here as an intentional exception
rather than left to be discovered. Removing it would not change today's
behaviour; that is precisely why it is safe to leave in place while the rest of
the model is tightened.

## Guards: exactly one per capability
| Capability | Single guard | Enforced in |
|---|---|---|
| add/remove source | `get_admin` + `require_auth` | `sources.rs` |
| register/unregister asset | `get_admin` + `require_auth` | `assets.rs` |
| submit price | `source.require_auth` + `check_source` | `prices.rs` |
| correct price | `get_admin` + `require_auth` + `has_role(PriceUpdater)` | `corrections.rs` |
| config change | `get_admin` + `require_auth` | `admin.rs` |
| upgrade / set admin | `get_admin` + `require_auth` | `lib.rs` |
| delegate/revoke role | `get_admin` + `require_auth` | `rbac.rs` |
| multisig signer rotation | `get_admin` + `require_auth` | `multisig.rs` (#508) |
| dead-man arming | `get_admin` + `require_auth` | `dead_man.rs` (#510) |
| dead-man recovery | `guardian.require_auth` + membership in the recovery set **or** admin | `dead_man.rs` (#510) |
| guardian recovery of admin | `guardian.require_auth` + threshold of guardians | `recovery.rs` |

No capability in this table is reachable through two *differently guarded* paths
except `correct_price`, whose double guard is a conjunction (both required), not
an alternative route. That is the distinction the `capability_matrix_tests`
suite enforces.

## No self-escalation

The escalation path in a role system is: obtain one role, then use it to obtain
another. Here that path is closed structurally rather than by convention:

* `delegate_role` and `revoke_role` require the **admin's** authorization and do
  not consult any role. A delegatee holding `UpgradeManager` — the most powerful
  role — still cannot delegate, revoke, or promote itself, because no role
  appears in those guards.
* There is no "rename a role" operation, so a role cannot be turned into
  another.
* `has_role` grants the admin every role implicitly. This is the one implicit
  capability in the model. It is a **narrowing** risk rather than an escalation
  one (it only ever adds authority to the already-most-privileged address) and
  it is recorded as an intentional exception; the implicit grant is spelled out
  in exactly one place, `rbac::has_role`, which is where it would be changed if
  the admin's all-roles rule is ever removed.

## Revocation removes derived authority

`revoke_role` clears three things, and all three are load-bearing:

1. the `DelegatedRole` grant flag — so `has_role` reports `false`;
2. the address's entry in `RoleHolders` — so `get_role_holders` no longer lists
   it; and
3. consequently every derived capability, because `has_role` is the *only*
   runtime source of delegated authority.

There is no separate "derived" store to invalidate, precisely because
`has_role` reads the primary flag directly rather than a cached copy. The tests
assert all three effects, and assert that revoking one role leaves the others
intact.

## Auto-check

`capability_matrix_tests.rs` asserts, against the source tree:

* every `Role` variant in `types.rs` appears in the matrix above;
* every `check_role` / `has_role` call site in a non-test module corresponds to a
  capability row in this document (so a new role check cannot be added without
  documenting it);
* the `correct_price` double guard is still a conjunction, and no other
  capability has acquired a second guard;
* no self-escalation: a delegatee cannot delegate, revoke, or promote itself;
* revocation removes the grant flag, the holders-list entry, and every derived
  capability;
* the implicit admin all-roles grant is confined to `rbac.rs`, so no other module
  can quietly widen it.

## Intentional exceptions

| Exception | Why it exists | Why it is safe |
|---|---|---|
| Admin implicitly holds every role | Keeps the admin usable while roles are adopted incrementally | Only ever adds authority to the most-privileged address; spelled out in exactly one place, `rbac::has_role` |
| `correct_price` checks both admin and `PriceUpdater` | Documents that correction is a price-writer capability | Conjunction, not an alternative route; currently inert because the admin holds every role |
| Multisig signers execute without being admin | Governance is a separate authority from administration | Signer set is admin-settable and auditable via `ms_get_governors` and the #508 rotation event |
| Dead-man recovery accepts the admin *or* a guardian | Guardians exist precisely so recovery survives loss of the admin key | Neither authority can arm or trip the switch; only clear it |
| Guardian recovery threshold > 1 | A single compromised guardian must not be able to replace the admin | Enforced in `recovery.rs`; dead-man recovery deliberately requires only one guardian because clearing a degraded state is strictly less dangerous than replacing the admin |

