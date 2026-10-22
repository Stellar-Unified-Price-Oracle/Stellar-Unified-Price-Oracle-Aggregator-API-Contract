//! # #689 — Upgrade authorization and artifact mismatch
//!
//! The admin approves an upgrade by authorizing `upgrade(new_wasm_hash)` for the
//! hash of the approved release artifact. Soroban binds an authorization to the
//! exact call arguments, so that approval cannot be used to install any other
//! WASM. These tests prove it with a substituted artifact that is itself a valid,
//! uploaded contract: the only thing that stops it is the authorization.
//! The release procedure is documented in `docs/blue-green-upgrade.md`.

use soroban_sdk::{
    testutils::{Address as _, MockAuth, MockAuthInvoke},
    Address, Bytes, BytesN, Env, IntoVal,
};

use crate::test_helpers::*;
use crate::PriceOracleContractClient;

/// The approved release artifact, built by `make build`.
const RELEASE_WASM: &[u8] =
    include_bytes!("../../../target/wasm32v1-none/release/price_oracle.wasm");

/// Uploads the approved release artifact and returns its hash.
fn upload_release(e: &Env) -> BytesN<32> {
    e.deployer()
        .upload_contract_wasm(Bytes::from_slice(e, RELEASE_WASM))
}

/// Uploads a substituted artifact: the release WASM with an extra custom
/// section appended. It is still a valid contract, with a different hash.
fn upload_substitute(e: &Env) -> BytesN<32> {
    let mut wasm = Bytes::from_slice(e, RELEASE_WASM);
    // Custom section: id 0, size 5, name length 4, name "swap".
    wasm.extend_from_array(&[0x00, 0x05, 0x04, b's', b'w', b'a', b'p']);
    e.deployer().upload_contract_wasm(wasm)
}

/// The admin's approval, bound to exactly one WASM hash.
fn approve_upgrade_to(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    admin: &Address,
    hash: &BytesN<32>,
) {
    e.mock_auths(&[MockAuth {
        address: admin,
        invoke: &MockAuthInvoke {
            contract: &client.address,
            fn_name: "upgrade",
            args: (hash.clone(),).into_val(e),
            sub_invokes: &[],
        },
    }]);
}

/// An oracle serving one price, with the release artifact and a substitute uploaded.
/// Returns (client, admin, source, asset, approved hash, substitute hash).
fn setup(
    e: &Env,
) -> (
    PriceOracleContractClient<'_>,
    Address,
    Address,
    Address,
    BytesN<32>,
    BytesN<32>,
) {
    // Uploading and swapping the contract wasm is far more expensive than the
    // default test budget allows.
    e.cost_estimate().disable_resource_limits();
    e.cost_estimate().budget().reset_unlimited();
    let (client, admin, source, asset) = setup_basic(e);
    ledger_default(e, 1, 1000);
    submit_test_price_n(&client, &source, &asset, 55000, 1000, 1);

    let approved = upload_release(e);
    let substitute = upload_substitute(e);
    assert_ne!(
        approved, substitute,
        "the substitute must be a different artifact"
    );
    (client, admin, source, asset, approved, substitute)
}

/// Normal operation: the approval applies the approved artifact, and state survives.
#[test]
fn approved_artifact_upgrades_under_its_own_approval() {
    let e = Env::default();
    let (client, admin, _, asset, approved, _) = setup(&e);

    approve_upgrade_to(&e, &client, &admin, &approved);
    client.upgrade(&approved);

    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 55000);
}

/// A substituted hash fails before any mutation and leaves the prior
/// implementation serving reads and accepting submissions.
#[test]
fn substituted_artifact_is_refused_and_prior_implementation_stays_usable() {
    let e = Env::default();
    let (client, admin, source, asset, approved, substitute) = setup(&e);

    approve_upgrade_to(&e, &client, &admin, &approved);
    assert!(
        client.try_upgrade(&substitute).is_err(),
        "an approval for the release artifact must not install another hash"
    );

    // Nothing changed: same admin, same price, and the contract still works.
    assert_eq!(client.get_admin(), admin);
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 55000);
    e.mock_all_auths();
    ledger_default(&e, 2, 2000);
    submit_test_price_n(&client, &source, &asset, 56000, 2000, 2);
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 56000);
}

/// Recovery: after a refused substitution the approved release can still be applied.
#[test]
fn approved_artifact_still_applies_after_a_refused_substitution() {
    let e = Env::default();
    let (client, admin, _, asset, approved, substitute) = setup(&e);

    approve_upgrade_to(&e, &client, &admin, &approved);
    assert!(client.try_upgrade(&substitute).is_err());

    approve_upgrade_to(&e, &client, &admin, &approved);
    client.upgrade(&approved);
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 55000);
}

/// Boundary: a hash one byte away from the approved one is refused.
#[test]
fn hash_differing_in_one_byte_is_refused() {
    let e = Env::default();
    let (client, admin, _, asset, approved, _) = setup(&e);

    let mut near = approved.to_array();
    near[31] ^= 0x01;
    let near = BytesN::from_array(&e, &near);

    approve_upgrade_to(&e, &client, &admin, &approved);
    assert!(client.try_upgrade(&near).is_err());
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 55000);
}

/// The substitute is a valid upgrade target on its own, so the refusals above
/// come from the authorization binding and not from a broken artifact.
#[test]
fn substitute_is_a_valid_artifact_when_explicitly_approved() {
    let e = Env::default();
    let (client, admin, _, _, _, substitute) = setup(&e);

    approve_upgrade_to(&e, &client, &admin, &substitute);
    client.upgrade(&substitute);
}

/// An approval by anyone other than the admin does not authorize the release artifact.
#[test]
fn approval_from_a_non_admin_is_refused() {
    let e = Env::default();
    let (client, _, _, asset, approved, _) = setup(&e);

    let outsider = Address::generate(&e);
    approve_upgrade_to(&e, &client, &outsider, &approved);
    assert!(client.try_upgrade(&approved).is_err());
    assert_eq!(client.get_price(&asset, &0u64).unwrap().price, 55000);
}
