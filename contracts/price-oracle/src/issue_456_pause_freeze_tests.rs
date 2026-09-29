//! Issue #456 — Emergency pause and freeze bypass, overlap, and griefing.
//!
//! # Mechanisms
//!
//! * `pause` / `unpause` — global `CfgPauseFlag`.
//! * `emergency_pause` / `extend_emergency_pause` / `cancel_emergency_pause` —
//!   sets `CfgPauseFlag` plus `EmergencyPauseActive`/`Entry`/`Reason`.
//! * `freeze_price` / `unfreeze_price` — per-asset `FrozenPrice(asset)`.
//!
//! # Path × state matrix (enforced below)
//!
//! | Path | paused | frozen (asset) | paused + frozen |
//! |---|---|---|---|
//! | `submit_price` | `ContractPaused` (#12) | `PriceFrozen` (#116) | `ContractPaused` (#12) |
//! | `submit_prices` | `ContractPaused` (#12) | `PriceFrozen` (#116) | `ContractPaused` (#12) |
//! | `zk_submit_price` | **accepted (gap)** | **accepted (gap)** | **accepted (gap)** |
//! | `get_price` | live aggregate | frozen snapshot | frozen snapshot |
//! | admin config (`freeze_price`, `set_query_rate_limit`) | **accepted (gap)** | accepted | accepted |
//!
//! Precedence: the pause check runs before the freeze check, so a paused
//! contract always reports `ContractPaused`; a freeze outlives an unpause.
//!
//! # Findings
//!
//! * **Pause/freeze bypass.** `zk_submit_price` calls `submit_price_internal`,
//!   which checks neither the pause flag nor the freeze. A registered source can
//!   mutate the aggregate while the contract is paused or the asset frozen.
//! * **Governance while paused.** Admin configuration is not gated by pause.
//! * **Contradictory state.** A plain `unpause` during an emergency pause clears
//!   `CfgPauseFlag` but leaves `EmergencyPauseActive = true`.
//! * **Unbounded pause.** `auto_unpause_if_due` is never called, so the
//!   `auto_unpause_ledger` deadline is not enforced, and
//!   `extend_emergency_pause` accepts any extension. There is no escalation path.
//! * **Targeted source pausing.** There is no per-source pause; the only way to
//!   exclude a source is admin suspension, which is outside this mechanism.
//! * **Events.** Emergency pause events carry actor and reason; plain
//!   `pause`/`unpause` and `unfreeze_price` events carry no reason.

use soroban_sdk::{testutils::Ledger, vec, Address, Bytes, BytesN, Env, String, Vec};

use crate::test_helpers::{ledger_default, setup_basic};
use crate::types::{AggregatePrice, DataKey, Groth16Proof, Groth16VerifyingKey};
use crate::PriceOracleContractClient;

const TS: u64 = 1_000_000;

fn setup(e: &Env) -> (PriceOracleContractClient<'_>, Address, Address) {
    e.mock_all_auths();
    ledger_default(e, 100, TS);
    let (client, _admin, source, asset) = setup_basic(e);
    client.submit_price(&source, &asset, &1_000i128, &TS);
    (client, source, asset)
}

fn aggregate(e: &Env, client: &PriceOracleContractClient<'_>, asset: &Address) -> i128 {
    e.as_contract(&client.address, || {
        e.storage()
            .persistent()
            .get::<_, AggregatePrice>(&DataKey::Aggregate(asset.clone()))
            .unwrap()
            .price
    })
}

fn u64_signal(e: &Env, v: u64) -> BytesN<32> {
    let mut b = [0u8; 32];
    b[24..].copy_from_slice(&v.to_be_bytes());
    BytesN::from_array(e, &b)
}

/// Submits a ZK attestation whose tag is computed from public data only
/// (see issue #452 for why this is accepted).
fn zk_submit(
    e: &Env,
    client: &PriceOracleContractClient<'_>,
    source: &Address,
    asset: &Address,
    price: u64,
) {
    let precomp = Bytes::from_slice(e, &[7u8; 32]);
    client.zk_set_verification_key(&Groth16VerifyingKey {
        ic_len: 4,
        ic_bytes: Bytes::new(e),
        pairing_precomp: precomp.clone(),
    });
    let signals: Vec<BytesN<32>> =
        vec![e, u64_signal(e, 1), u64_signal(e, price), u64_signal(e, TS)];
    let (a, b, c) = (
        Bytes::from_slice(e, &[1u8; 64]),
        Bytes::from_slice(e, &[2u8; 128]),
        Bytes::from_slice(e, &[3u8; 64]),
    );
    let mut t = Bytes::new(e);
    t.append(&a);
    t.append(&b);
    t.append(&c);
    for s in signals.iter() {
        t.append(&Bytes::from_slice(e, &s.to_array()));
    }
    let mut tag = Bytes::from_slice(e, &e.crypto().sha256(&t).to_array());
    tag.append(&precomp);
    let fs_check = BytesN::from_array(e, &e.crypto().sha256(&tag).to_array());
    client.zk_submit_price(source, asset, &Groth16Proof { a, b, c, fs_check }, &signals);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn paused_rejects_submit_price() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    client.pause();
    client.submit_price(&source, &asset, &2_000i128, &TS);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn paused_rejects_submit_prices() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    client.pause();
    client.submit_prices(&source, &vec![&e, (asset, 2_000i128, TS)]);
}

#[test]
#[should_panic(expected = "Error(Contract, #116)")]
fn frozen_rejects_submit_price() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    client.freeze_price(&asset, &String::from_str(&e, "flash crash"));
    client.submit_price(&source, &asset, &2_000i128, &TS);
}

#[test]
#[should_panic(expected = "Error(Contract, #12)")]
fn pause_takes_precedence_over_freeze() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    client.freeze_price(&asset, &String::from_str(&e, "x"));
    client.pause();
    client.submit_price(&source, &asset, &2_000i128, &TS);
}

#[test]
#[should_panic(expected = "Error(Contract, #116)")]
fn freeze_outlives_unpause() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    client.freeze_price(&asset, &String::from_str(&e, "x"));
    client.pause();
    client.unpause();
    client.submit_price(&source, &asset, &2_000i128, &TS);
}

#[test]
fn queries_served_while_paused_and_frozen_snapshot_wins() {
    let e = Env::default();
    let (client, _source, asset) = setup(&e);
    client.pause();
    assert_eq!(client.get_price(&asset, &u64::MAX).unwrap().price, 1_000);
    client.freeze_price(&asset, &String::from_str(&e, "x"));
    assert_eq!(client.get_price(&asset, &u64::MAX).unwrap().price, 1_000);
}

#[test]
fn gap_zk_submission_bypasses_pause_and_freeze() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    client.freeze_price(&asset, &String::from_str(&e, "x"));
    client.pause();

    zk_submit(&e, &client, &source, &asset, 5_000);

    // The live aggregate changed while paused and frozen; get_price still
    // serves the frozen snapshot until unfreeze exposes the new value.
    assert_eq!(aggregate(&e, &client, &asset), 5_000);
    assert_eq!(client.get_price(&asset, &u64::MAX).unwrap().price, 1_000);
}

#[test]
fn gap_admin_config_mutable_while_paused() {
    let e = Env::default();
    let (client, _source, _asset) = setup(&e);
    client.pause();
    client.set_query_rate_limit(&50u32);
    assert_eq!(client.get_query_rate_limit(), 50);
    assert!(client.is_paused());
}

#[test]
fn gap_plain_unpause_leaves_emergency_flag_set() {
    let e = Env::default();
    let (client, _source, _asset) = setup(&e);
    client.emergency_pause(&String::from_str(&e, "incident"), &100u32);
    client.unpause();
    assert!(!client.is_paused());
    assert!(client.is_emergency_pause_active());
}

#[test]
fn cancel_emergency_pause_clears_all_state() {
    let e = Env::default();
    let (client, _source, _asset) = setup(&e);
    client.emergency_pause(&String::from_str(&e, "incident"), &100u32);
    client.cancel_emergency_pause();
    assert!(!client.is_paused());
    assert!(!client.is_emergency_pause_active());
    assert!(client.get_emergency_pause().is_none());
}

#[test]
fn gap_auto_unpause_deadline_not_enforced() {
    let e = Env::default();
    let (client, source, asset) = setup(&e);
    client.emergency_pause(&String::from_str(&e, "incident"), &10u32);
    e.ledger().with_mut(|l| l.sequence_number += 1_000);

    assert!(client.is_paused());
    assert!(client
        .try_submit_price(&source, &asset, &2_000i128, &TS)
        .is_err());
}

#[test]
fn gap_emergency_pause_extension_unbounded() {
    let e = Env::default();
    let (client, _source, _asset) = setup(&e);
    client.emergency_pause(&String::from_str(&e, "incident"), &10u32);
    client.extend_emergency_pause(&1_000_000_000u32);
    assert_eq!(
        client.get_emergency_pause().unwrap().auto_unpause_ledger,
        100 + 10 + 1_000_000_000
    );
}

#[test]
fn emergency_pause_records_actor_and_reason() {
    let e = Env::default();
    e.mock_all_auths();
    let (client, admin, _source, _asset) = setup_basic(&e);
    let reason = String::from_str(&e, "oracle compromise");
    client.emergency_pause(&reason, &10u32);
    let p = client.get_emergency_pause().unwrap();
    assert_eq!(p.reason, reason);
    assert_eq!(p.initiated_by, admin);
}
