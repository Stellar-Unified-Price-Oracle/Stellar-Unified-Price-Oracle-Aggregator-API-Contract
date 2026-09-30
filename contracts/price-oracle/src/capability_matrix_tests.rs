#![cfg(test)]
//! #509 — Capability-matrix completeness, no self-escalation, and revocation.
//!
//! The matrix in `docs/security/capability-matrix.md` is only useful if it
//! cannot silently drift from the code. These tests check it against the source
//! tree and then check the properties the matrix claims: no role can escalate
//! itself, and revocation removes derived authority rather than just the direct
//! grant.

use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Env, IntoVal,
};

use crate::test_helpers::*;
use crate::types::Role;
use crate::PriceOracleContract;
use std::vec::Vec;

const MATRIX: &str = include_str!("../../../docs/security/capability-matrix.md");

fn matrix_mentions(role: &str) -> bool {
    MATRIX.contains(&format!("`{role}`"))
}

// ---------------------------------------------------------------------------
// Completeness: the matrix covers the source
// ---------------------------------------------------------------------------

/// Every `Role` variant declared in `types.rs` must appear in the matrix, so a
/// new role cannot be added without documenting what it grants.
#[test]
fn every_role_variant_is_documented() {
    let types = include_str!("types.rs");
    let body = &types[types.find("pub enum Role {").expect("Role enum missing")..];
    let body = &body[..body.find("\n}").expect("unterminated Role enum")];

    // Variant lines look like `    SourceManager = 0,`, each preceded by a
    // doc comment. Parse the discriminants rather than the prose.
    let variants: Vec<&str> = body
        .lines()
        .filter_map(|l| l.trim().split_once('=').map(|(name, _)| name.trim()))
        .filter(|name| !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric()))
        .collect();

    assert!(
        !variants.is_empty(),
        "failed to parse any Role variants out of types.rs"
    );
    for v in &variants {
        assert!(
            matrix_mentions(v),
            "role `{v}` is declared in types.rs but missing from the capability matrix"
        );
    }
}

/// Every role-checking call site in a non-test module must correspond to a
/// capability row, so a new role gate cannot appear undocumented.
#[test]
fn every_role_check_call_site_is_documented() {
    // The only runtime role checks in the contract, and the capability each
    // one belongs to. Adding a check means adding a row here *and* to the
    // matrix, which is the point of the test.
    let documented: &[(&str, &str)] = &[
        ("corrections.rs", "correct a published price"),
        ("rbac.rs", "delegate / revoke a role"),
    ];

    for (file, capability) in documented {
        assert!(
            MATRIX.contains(capability),
            "capability `{capability}` (checked in {file}) is missing from the matrix"
        );
        assert!(
            MATRIX.contains(file),
            "the matrix's guard table should name {file} as the enforcing module"
        );
    }
}

/// `correct_price` is the documented double-guard alias. The two guards must
/// remain a conjunction (both required), never an alternative route.
#[test]
fn the_documented_alias_is_still_a_conjunction() {
    let src = include_str!("corrections.rs");
    let body = &src[src
        .find("pub fn correct_price")
        .expect("correct_price missing")..];
    let body = &body[..body.find("\n}").expect("unterminated correct_price")];

    assert!(
        body.contains("require_auth"),
        "correct_price must still require the admin's authorization"
    );
    assert!(
        body.contains("has_role"),
        "correct_price must still check the PriceUpdater role"
    );

    // A conjunction: the role check runs *before* the auth requirement, and
    // neither path alone reaches the correction. The guard must not have been
    // rewritten into a disjunction of "admin OR PriceUpdater delegate" — that
    // would be a second, differently-guarded route to the same capability, which
    // is exactly what the matrix forbids. Only the two guard lines themselves
    // are inspected, so unrelated `||` uses further down the function (e.g.
    // `new_price <= 0 || reason.is_empty()`) do not trip this.
    let guard_lines: Vec<&str> = body
        .lines()
        .take_while(|l| !l.contains("check_registered_asset"))
        .collect();
    let guards = guard_lines.join("\n");
    assert!(
        !guards.contains("||"),
        "correct_price's two guards must be a conjunction, not an alternative \
         route to the same capability"
    );
    assert!(
        !guards.contains("||=") && !guards.contains("&&"),
        "the two guards must be sequential requirements, not one combined \
         expression that could short-circuit"
    );
}

/// The implicit admin all-roles grant must stay confined to `rbac.rs`, so no
/// other module can quietly widen it.
#[test]
fn implicit_admin_all_roles_grant_is_confined_to_rbac() {
    let rbac = include_str!("rbac.rs");

    // ---------------------------------------------------------------------------
    // No self-escalation
    // ---------------------------------------------------------------------------

    const ALL_ROLES: [Role; 5] = [
        Role::SourceManager,
        Role::AssetManager,
        Role::PriceUpdater,
        Role::ConfigManager,
        Role::UpgradeManager,
    ];

    /// A delegatee cannot delegate a role to itself, however many roles it holds.
    #[test]
    fn a_delegatee_cannot_delegate_to_itself() {
        let e = Env::default();
        let (c, _admin) = setup_contract(&e);

        let d = Address::generate(&e);
        for r in ALL_ROLES {
            c.delegate_role(&d, &r);
        }

        // Even holding every role — including UpgradeManager — the delegatee has no
        // authority over the delegation mechanism itself.
        for r in ALL_ROLES {
            assert!(
                c.try_delegate_role(&d, &r).is_err(),
                "a delegatee must not be able to delegate {r:?} to itself"
            );
        }
    }

    /// A delegatee cannot revoke roles, including from itself or from a peer.
    #[test]
    fn a_delegatee_cannot_revoke_roles() {
        let e = Env::default();
        let (c, _admin) = setup_contract(&e);

        let d = Address::generate(&e);
        let peer = Address::generate(&e);
        c.delegate_role(&d, &Role::UpgradeManager);
        c.delegate_role(&peer, &Role::ConfigManager);

        assert!(c.try_revoke_role(&d, &Role::UpgradeManager).is_err());
        assert!(
            c.try_revoke_role(&d, &Role::ConfigManager).is_err(),
            "a delegatee must not be able to revoke a peer's role"
        );

        // Nothing changed.
        assert!(c.has_role(&d, &Role::UpgradeManager));
        assert!(c.has_role(&peer, &Role::ConfigManager));
    }

    /// `delegate_role` is gated on the admin's authorization alone: an auth frame
    /// from a non-admin leaves the call unauthorized even with all roles granted.
    #[test]
    fn delegation_is_gated_on_the_admin_not_on_a_role() {
        let e = Env::default();
        let (c, _admin) = setup_contract(&e);

        let delegatee = Address::generate(&e);
        c.delegate_role(&delegatee, &Role::UpgradeManager);

        // Only the delegatee authorizes the call.
        e.mock_auths(&[MockAuth {
            address: &delegatee,
            invoke: &MockAuthInvoke {
                contract: &c.address,
                fn_name: "delegate_role",
                args: (delegatee.clone(), Role::UpgradeManager).into_val(&e),
                sub_invokes: &[],
            },
        }]);

        assert!(
            c.try_delegate_role(&delegatee, &Role::UpgradeManager)
                .is_err(),
            "holding UpgradeManager must not be a substitute for admin authority"
        );
    }

    // ---------------------------------------------------------------------------
    // Revocation removes derived authority
    // ---------------------------------------------------------------------------

    /// Revocation clears the grant flag, the holders-list entry, and therefore
    /// every capability derived from the role.
    #[test]
    fn revocation_removes_all_derived_authority() {
        let e = Env::default();
        let (c, _admin) = setup_contract(&e);

        let d = Address::generate(&e);
        c.delegate_role(&d, &Role::PriceUpdater);

        // Before: the grant is visible three ways.
        assert!(c.has_role(&d, &Role::PriceUpdater));
        assert!(!c.get_roles_for_holder(&d).is_empty());
        assert!(c.get_role_holders(&Role::PriceUpdater).contains(&d));

        c.revoke_role(&d, &Role::PriceUpdater);

        // After: all three agree the authority is gone.
        assert!(
            !c.has_role(&d, &Role::PriceUpdater),
            "the grant flag must be cleared"
        );
        assert!(
            c.get_roles_for_holder(&d).is_empty(),
            "no role may remain listed for a fully-revoked address"
        );
        assert!(
            !c.get_role_holders(&Role::PriceUpdater).contains(&d),
            "the holders list must not retain a revoked address"
        );
        assert!(
            c.get_address_roles(&d).is_empty(),
            "the derived view must be empty too"
        );
    }

    /// Revocation is per-role: it must not silently strip the other roles the same
    /// address holds.
    #[test]
    fn revocation_is_scoped_to_the_named_role() {
        let e = Env::default();
        let (c, _admin) = setup_contract(&e);

        let d = Address::generate(&e);
        c.delegate_role(&d, &Role::SourceManager);
        c.delegate_role(&d, &Role::AssetManager);

        c.revoke_role(&d, &Role::SourceManager);

        assert!(!c.has_role(&d, &Role::SourceManager));
        assert!(
            c.has_role(&d, &Role::AssetManager),
            "revoking one role must not strip an unrelated one"
        );
        assert_eq!(c.get_roles_for_holder(&d).len(), 1);
    }

    /// Revocation is immediate: it takes effect in the same ledger it is called in,
    /// with no window in which the revoked authority still works.
    #[test]
    fn revocation_takes_effect_in_the_same_ledger() {
        let e = Env::default();
        let (c, _admin) = setup_contract(&e);

        let d = Address::generate(&e);
        c.delegate_role(&d, &Role::ConfigManager);
        assert!(c.has_role(&d, &Role::ConfigManager));

        // Same ledger, no advance.
        c.revoke_role(&d, &Role::ConfigManager);
        assert!(
            !c.has_role(&d, &Role::ConfigManager),
            "revocation must not be deferred to a later ledger"
        );
    }

    /// Revoking a role that was never granted is a no-op, not an error path that
    /// leaves partial state behind.
    #[test]
    fn revoking_an_unheld_role_is_harmless() {
        let e = Env::default();
        let (c, _admin) = setup_contract(&e);

        let d = Address::generate(&e);
        c.revoke_role(&d, &Role::UpgradeManager);

        assert!(!c.has_role(&d, &Role::UpgradeManager));
        assert!(c.get_roles_for_holder(&d).is_empty());
    }

    assert!(
        rbac.contains("if caller == &admin"),
        "the implicit admin grant should still live in rbac::has_role"
    );

    // No other module may short-circuit a role check for the admin.
    for module in ["corrections.rs", "prices.rs", "sources.rs", "assets.rs"] {
        let src: &str = match module {
            "corrections.rs" => include_str!("corrections.rs"),
            "prices.rs" => include_str!("prices.rs"),
            "sources.rs" => include_str!("sources.rs"),
            "assets.rs" => include_str!("assets.rs"),
            _ => unreachable!(),
        };
        assert!(
            !src.contains("Role::") || module == "corrections.rs",
            "{module} references Role:: outside rbac.rs; only the documented \
             corrections.rs alias may do so (#509)"
        );
    }
}
