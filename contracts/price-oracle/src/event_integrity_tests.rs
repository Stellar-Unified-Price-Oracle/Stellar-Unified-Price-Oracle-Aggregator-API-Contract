//! Event and indexing integrity tests (#469).
//!
//! See `docs/security/event-census.md` for the event census and monitor guidance.

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    xdr::{ContractEvent, ContractEventBody, ScVal},
    Address, Env, Event, String, TryFromVal,
};

use crate::events::{
    AdminChangedEvent, AssetRegisteredEvent, SourceAddedEvent, SourceRemovedEvent,
};
use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};
use crate::{PriceOracleContract, PriceOracleContractClient};

fn topics(ev: &ContractEvent) -> std::vec::Vec<ScVal> {
    match &ev.body {
        ContractEventBody::V0(v0) => v0.topics.to_vec(),
    }
}

fn contract_events(e: &Env, c: &PriceOracleContractClient<'_>) -> std::vec::Vec<ContractEvent> {
    e.events()
        .all()
        .filter_by_contract(&c.address)
        .events()
        .to_vec()
}

#[test]
fn untrusted_contract_cannot_emit_privileged_looking_event_under_oracle_id() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, admin) = setup_contract(&e);
    let attacker = e.register(PriceOracleContract, ());
    let mallory = Address::generate(&e);

    // Attacker publishes a byte-identical AdminChanged payload from its own contract.
    e.as_contract(&attacker, || {
        AdminChangedEvent {
            old_admin: admin.clone(),
            new_admin: mallory.clone(),
        }
        .publish(&e)
    });
    let all = e.events().all();
    // The spoof is attributed to the attacker's contract id, never the oracle's.
    assert!(all.filter_by_contract(&c.address).events().is_empty());
    let forged = AdminChangedEvent {
        old_admin: admin.clone(),
        new_admin: mallory,
    };
    assert!(all
        .filter_by_contract(&attacker)
        .events()
        .contains(&forged.to_xdr(&e, &attacker)));
    assert_ne!(forged.to_xdr(&e, &attacker), forged.to_xdr(&e, &c.address));
    assert_eq!(c.get_admin(), admin);
}

#[test]
fn state_modifying_events_carry_actor() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, admin) = setup_contract(&e);

    let src = Address::generate(&e);
    let name = String::from_str(&e, "S1");
    c.add_source(&src, &name);
    let ev = SourceAddedEvent {
        source: src.clone(),
        admin: admin.clone(),
        name,
    };
    assert!(contract_events(&e, &c).contains(&ev.to_xdr(&e, &c.address)));

    let asset = Address::generate(&e);
    c.register_asset(&asset);
    let ev = AssetRegisteredEvent {
        asset,
        admin: admin.clone(),
    };
    assert!(contract_events(&e, &c).contains(&ev.to_xdr(&e, &c.address)));

    c.remove_source(&src);
    let ev = SourceRemovedEvent {
        source: src,
        admin: admin.clone(),
    };
    assert!(contract_events(&e, &c).contains(&ev.to_xdr(&e, &c.address)));

    let new_admin = Address::generate(&e);
    c.set_admin(&new_admin);
    let ev = AdminChangedEvent {
        old_admin: admin,
        new_admin,
    };
    assert!(contract_events(&e, &c).contains(&ev.to_xdr(&e, &c.address)));
}

#[test]
fn rejected_operations_emit_no_success_events() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, _) = setup_contract(&e);
    let src = register_test_source(&e, &c, "S1");
    let asset = register_test_asset(&e, &c);

    // Duplicate source, unknown source removal, unregistered submitter, bad price.
    assert!(c
        .try_add_source(&src, &String::from_str(&e, "dup"))
        .is_err());
    assert!(contract_events(&e, &c).is_empty());
    assert!(c.try_remove_source(&Address::generate(&e)).is_err());
    assert!(contract_events(&e, &c).is_empty());
    let ts = e.ledger().timestamp();
    assert!(c
        .try_submit_price(&Address::generate(&e), &asset, &100, &ts)
        .is_err());
    assert!(contract_events(&e, &c).is_empty());
    assert!(c.try_submit_price(&src, &asset, &-1, &ts).is_err());
    assert!(contract_events(&e, &c).is_empty());
}

#[test]
fn rejected_unauthorized_admin_call_emits_nothing() {
    let e = Env::default();
    let (c, _) = setup_contract(&e);
    e.set_auths(&[]);
    assert!(c
        .try_add_source(&Address::generate(&e), &String::from_str(&e, "x"))
        .is_err());
    assert!(contract_events(&e, &c).is_empty());
}

/// Rebuilds the source set from `SourceAdded` / `SourceRemoved` events alone and
/// checks it against on-chain state after every lifecycle step.
#[test]
fn event_log_reconstruction_matches_state() {
    let e = Env::default();
    e.mock_all_auths();
    let (c, admin) = setup_contract(&e);

    let probe = Address::generate(&e);
    let added_sym = topics(
        &SourceAddedEvent {
            source: probe.clone(),
            admin: admin.clone(),
            name: String::from_str(&e, ""),
        }
        .to_xdr(&e, &c.address),
    )[0]
    .clone();
    let removed_sym = topics(
        &SourceRemovedEvent {
            source: probe,
            admin,
        }
        .to_xdr(&e, &c.address),
    )[0]
    .clone();

    let mut model: std::vec::Vec<Address> = std::vec::Vec::new();
    let mut apply = |e: &Env, c: &PriceOracleContractClient<'_>| {
        for ev in contract_events(e, c) {
            let t = topics(&ev);
            let addr = || Address::try_from_val(e, &t[1]).unwrap();
            if t[0] == added_sym {
                model.push(addr());
            } else if t[0] == removed_sym {
                let a = addr();
                model.retain(|x| *x != a);
            }
        }
        let mut actual: std::vec::Vec<Address> = c.get_oracle_sources().sources.iter().collect();
        let mut expected = model.clone();
        actual.sort();
        expected.sort();
        assert_eq!(actual, expected);
    };

    let mut live = std::vec::Vec::new();
    for i in 0..8u32 {
        let s = Address::generate(&e);
        c.add_source(&s, &String::from_str(&e, "src"));
        apply(&e, &c);
        live.push(s);
        if i % 3 == 2 {
            let victim = live.remove(0);
            c.remove_source(&victim);
            apply(&e, &c);
        }
    }
    // Re-admission after removal is also reconstructed correctly.
    let back = live[0].clone();
    c.remove_source(&back);
    apply(&e, &c);
    c.add_source(&back, &String::from_str(&e, "back"));
    apply(&e, &c);
}
