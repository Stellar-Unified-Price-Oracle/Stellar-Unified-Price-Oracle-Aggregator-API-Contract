#![cfg(test)]

//! Tests for #246 — configurable history storage tier.
//!
//! The suite is written adversarially: every downgrade path is attacked with
//! the strongest thing available to an attacker (the admin key itself), and the
//! retention guarantees are checked against real stored data rather than
//! against constants.

use soroban_sdk::{
    testutils::{Address as _, Events as _},
    xdr::{ContractEvent, ToXdr},
    Address, Env, Event as _, Vec,
};

use crate::events::{StorageTierChangedEvent, StorageTierMigratedEvent};
use crate::history::LEDGERS_PER_WEEK;
use crate::test_helpers::*;
use crate::types::{
    DataKey, HistoryStorageTier, PriceHistoryEntry, StorageTierDowngradeRequest,
    TEMPORARY_RETENTION_LEDGERS,
};

/// Contract-emitted events, filtered to the oracle contract id.
fn oracle_events(
    e: &Env,
    client: &crate::PriceOracleContractClient<'_>,
) -> std::vec::Vec<ContractEvent> {
    e.events()
        .all()
        .filter_by_contract(&client.address)
        .events()
        .to_vec()
}

/// Asserts a `try_*` call failed with exactly `Error(Contract, #code)`.
///
/// The generated `try_*` clients return nested results whose `Debug` rendering
/// contains the contract error, so this matches on that rendering.
fn expect_contract_err<T: core::fmt::Debug, A: core::fmt::Debug, B: core::fmt::Debug>(
    res: Result<Result<T, A>, B>,
    code: u32,
) {
    let rendered = format!("{:?}", res);
    assert!(
        rendered.contains(&format!("#{}", code)),
        "expected Error(Contract, #{}), got {}",
        code,
        rendered
    );
}

/// Reads a pending downgrade request straight from contract storage.
fn read_request(
    e: &Env,
    client: &crate::PriceOracleContractClient<'_>,
    asset: &Address,
) -> Option<StorageTierDowngradeRequest> {
    let key = DataKey::AssetStorageTierDowngrade(asset.clone());
    e.as_contract(&client.address, || e.storage().persistent().get(&key))
}

/// Asserts that a `StorageTierChangedEvent` with the given shape was among the
/// snapshotted contract events.
///
/// `e.events().all()` only reflects the most recent contract invocation, so the
/// caller must snapshot immediately after the mutating call (via
/// [`oracle_events`]) and assert against that snapshot afterwards.
fn changed_event(
    snap: &[ContractEvent],
    e: &Env,
    client: &crate::PriceOracleContractClient<'_>,
    asset: &Address,
    old: HistoryStorageTier,
    new: HistoryStorageTier,
    action: u32,
    actor: &Address,
) -> bool {
    snap.contains(
        &StorageTierChangedEvent {
            asset: asset.clone(),
            old_tier: old.as_u32(),
            new_tier: new.as_u32(),
            action,
            actor: actor.clone(),
            ledger: e.ledger().sequence(),
        }
        .to_xdr(e, &client.address),
    )
}

/// Drives the full, legitimate downgrade: propose -> approve by a second party
/// -> wait out the timelock -> execute. Returns the second-party address.
fn perform_legit_downgrade(
    e: &Env,
    client: &crate::PriceOracleContractClient<'_>,
    asset: &Address,
) -> Address {
    client.propose_storage_tier_downgrade(asset);
    let second = Address::generate(e);
    client.approve_storage_tier_downgrade(asset, &second);
    // Mature the timelock.
    ledger_default(
        e,
        e.ledger().sequence() + 1_000,
        e.ledger().timestamp() + 5_000,
    );
    client.execute_storage_tier_downgrade(asset);
    second
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Default tier + discoverability
// ─────────────────────────────────────────────────────────────────────────────

/// A freshly registered asset is `Temporary` and the guarantee is discoverable.
#[test]
fn test_default_tier_is_temporary_and_discoverable() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Temporary
    );

    let info = client.get_storage_tier_info(&asset);
    assert_eq!(info.tier, HistoryStorageTier::Temporary);
    assert_eq!(info.retention_ledgers, TEMPORARY_RETENTION_LEDGERS);
    assert!(
        !info.durable,
        "temporary history is not durable by definition"
    );
    assert_eq!(info.entry_count, 0u32);
    assert_eq!(info.changed_at_ledger, 0u32);
}

/// After an upgrade the info reports the persistent guarantee.
#[test]
fn test_storage_tier_info_reports_persistent_after_upgrade() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 42, 4_000);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    let info = client.get_storage_tier_info(&asset);
    assert_eq!(info.tier, HistoryStorageTier::Persistent);
    assert!(info.durable);
    assert_eq!(info.changed_at_ledger, 42u32);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Upgrade applies immediately and is observable
// ─────────────────────────────────────────────────────────────────────────────

/// `Temporary -> Persistent` applies immediately and emits the event.
#[test]
fn test_upgrade_applies_immediately_and_emits_event() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 7, 700);

    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    let snap = oracle_events(&e, &client);

    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );
    assert!(changed_event(
        &snap,
        &e,
        &client,
        &asset,
        HistoryStorageTier::Temporary,
        HistoryStorageTier::Persistent,
        0,
        &admin
    ));
}

/// Setting the tier it already has is an observable no-op.
#[test]
fn test_set_same_tier_is_noop_but_emits() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Temporary);
    let snap = oracle_events(&e, &client);

    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Temporary
    );
    assert!(changed_event(
        &snap,
        &e,
        &client,
        &asset,
        HistoryStorageTier::Temporary,
        HistoryStorageTier::Temporary,
        0,
        &admin
    ));
}

/// Only the admin may change a tier.
#[test]
#[should_panic]
fn test_set_tier_unauthorized() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    clear_auth(&e);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. A downgrade without approval is rejected
// ─────────────────────────────────────────────────────────────────────────────

/// The admin alone cannot downgrade: `set_asset_storage_tier(.., Temporary)` is
/// refused with `StorageTierDowngradeNotReady` (#156).
#[test]
fn test_downgrade_without_approval_is_rejected() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    expect_contract_err(
        client.try_set_asset_storage_tier(&asset, &HistoryStorageTier::Temporary),
        156,
    );

    // Tier is untouched.
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );
}

/// A downgrade attempt with no authorization at all is refused by `require_auth`
/// before any tier logic runs, and the tier is untouched.
#[test]
fn test_set_tier_downgrade_without_auth_is_rejected() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    clear_auth(&e);
    assert!(client
        .try_set_asset_storage_tier(&asset, &HistoryStorageTier::Temporary)
        .is_err());

    e.mock_all_auths();
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. A downgrade cannot happen without multi-party + timelock authorization
// ─────────────────────────────────────────────────────────────────────────────

/// Full adversarial sequence: every shortcut is attempted and refused, and the
/// tier is asserted to still be `Persistent` at each rejected step.
#[test]
fn test_downgrade_requires_multiparty_and_timelock() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let second = Address::generate(&e);

    ledger_default(&e, 100, 1_000);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    // Step 1: direct downgrade attempt — refused.
    assert!(client
        .try_set_asset_storage_tier(&asset, &HistoryStorageTier::Temporary)
        .is_err());
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );

    // Step 2: execute with no request at all — refused.
    assert!(client.try_execute_storage_tier_downgrade(&asset).is_err());
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );

    // Step 3: propose (emits action = 1).
    let ready_by = client.propose_storage_tier_downgrade(&asset);
    assert_eq!(ready_by, 100u32 + 10u32);
    let proposed = oracle_events(&e, &client);
    assert!(changed_event(
        &proposed,
        &e,
        &client,
        &asset,
        HistoryStorageTier::Persistent,
        HistoryStorageTier::Temporary,
        1,
        &admin
    ));

    // Step 4: the proposing admin tries to self-approve — refused (#0), and the
    // request stays unapproved so the timelock has not even started.
    expect_contract_err(client.try_approve_storage_tier_downgrade(&asset, &admin), 0);
    let req = read_request(&e, &client, &asset).expect("request must still be pending");
    assert_eq!(req.proposed_by, admin);
    assert_eq!(req.approvals.len(), 0u32);
    assert_eq!(req.ready_at_ledger, 0u32);
    assert_eq!(req.from_tier, HistoryStorageTier::Persistent);

    // Step 5: execute before the timelock matures (nothing approved) — refused.
    assert!(client.try_execute_storage_tier_downgrade(&asset).is_err());
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );

    // Step 6: a distinct second party approves. Timelock starts, action = 2.
    client.approve_storage_tier_downgrade(&asset, &second);
    let approved = oracle_events(&e, &client);
    let req = read_request(&e, &client, &asset).expect("request must still be pending");
    assert_eq!(req.approvals.len(), 1u32);
    assert_eq!(req.approvals.get_unchecked(0), second);
    assert_eq!(req.ready_at_ledger, 100u32 + 10u32);
    assert!(changed_event(
        &approved,
        &e,
        &client,
        &asset,
        HistoryStorageTier::Persistent,
        HistoryStorageTier::Temporary,
        2,
        &second
    ));

    // Step 7: the same party cannot approve twice.
    expect_contract_err(
        client.try_approve_storage_tier_downgrade(&asset, &second),
        10,
    );

    // Step 8: execute one ledger before `ready_at_ledger` — refused, even though
    // the request is fully approved.
    ledger_default(&e, 109, 1_090);
    assert!(client.try_execute_storage_tier_downgrade(&asset).is_err());
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );

    // Step 9: at `ready_at_ledger` it succeeds and the tier flips.
    ledger_default(&e, 110, 1_100);
    client.execute_storage_tier_downgrade(&asset);
    let executed = oracle_events(&e, &client);
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Temporary
    );
    assert!(changed_event(
        &executed,
        &e,
        &client,
        &asset,
        HistoryStorageTier::Persistent,
        HistoryStorageTier::Temporary,
        3,
        &admin
    ));

    // The request is consumed, so it cannot be replayed.
    assert!(client.try_execute_storage_tier_downgrade(&asset).is_err());
}

/// An already-matured request also authorises the one-line
/// `set_asset_storage_tier` downgrade path — that is the *only* case in which
/// that call may lower a tier.
#[test]
fn test_set_tier_downgrade_allowed_only_after_matured_approval() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    let second = Address::generate(&e);

    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    client.propose_storage_tier_downgrade(&asset);
    client.approve_storage_tier_downgrade(&asset, &second);

    // Not matured yet.
    assert!(client
        .try_set_asset_storage_tier(&asset, &HistoryStorageTier::Temporary)
        .is_err());
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );

    ledger_default(&e, 100, 1_000);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Temporary);
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Temporary
    );
}

/// A downgrade never deletes history: the entry stays in the bucket and only
/// the destination for *new* entries changes.
#[test]
fn test_downgrade_does_not_erase_stored_history() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    ledger_default(&e, 100, 1_000);
    submit_test_price(&client, &source, &asset, 4_200_000i128, 1_000u64);
    assert!(client.get_tiered_historical_price(&asset, &100).is_some());

    perform_legit_downgrade(&e, &client, &asset);
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Temporary
    );

    // The bucket entry was NOT deleted — it is still physically there, and a
    // migration (an explicit, non-destructive copy) finds it again.
    let bucket_key = DataKey::HistoryBucket(asset.clone(), 100u32 / LEDGERS_PER_WEEK);
    let still_there: Option<Vec<PriceHistoryEntry>> = e.as_contract(&client.address, || {
        e.storage().persistent().get(&bucket_key)
    });
    assert!(still_there.is_some(), "downgrade must not delete entries");
    assert_eq!(still_there.unwrap().len(), 1u32);
}

/// `propose` is rejected for a non-persistent asset and when one is pending.
#[test]
fn test_propose_downgrade_guards() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);

    // Still Temporary -> nothing to downgrade.
    assert!(client.try_propose_storage_tier_downgrade(&asset).is_err());

    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    client.propose_storage_tier_downgrade(&asset);
    // A second proposal while one is pending is refused.
    assert!(client.try_propose_storage_tier_downgrade(&asset).is_err());
}

/// Only the admin may propose or execute; any party may approve (but not the
/// proposer).
#[test]
fn test_downgrade_roles_are_enforced() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    clear_auth(&e);
    assert!(client.try_propose_storage_tier_downgrade(&asset).is_err());

    e.mock_all_auths();
    client.propose_storage_tier_downgrade(&asset);

    clear_auth(&e);
    assert!(client
        .try_approve_storage_tier_downgrade(&asset, &Address::generate(&e))
        .is_err());

    // Fully approved + matured, but the admin must still authorize execution.
    e.mock_all_auths();
    client.approve_storage_tier_downgrade(&asset, &Address::generate(&e));
    ledger_default(&e, 1_000, 10_000);
    clear_auth(&e);
    assert!(client.try_execute_storage_tier_downgrade(&asset).is_err());
    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Persistent
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. No silent cross-tier fallback
// ─────────────────────────────────────────────────────────────────────────────

/// An entry that only exists in the *temporary* tier is invisible once the
/// asset is `Persistent`. The read returns an explicit `None` — it never
/// reaches across tiers for the value that is sitting right there.
#[test]
fn test_no_cross_tier_fallback_temporary_value_hidden_by_persistent_tier() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 100, 1_000);

    // Write an entry that exists ONLY in temporary storage (no shard write).
    let entry = PriceHistoryEntry {
        price: 7_777_777i128,
        timestamp: 1_000u64,
        ledger: 100u32,
        num_sources: 1u32,
        is_interpolated: false,
    };
    e.as_contract(&client.address, || {
        crate::storage_tier::write_history_entry(&e, &asset, &entry)
    });

    // Temporary tier sees it.
    assert_eq!(
        client
            .get_tiered_historical_price(&asset, &100)
            .map(|e| e.price),
        Some(7_777_777i128)
    );

    // Flip to Persistent: the same ledger is now an explicit absence, not the
    // temporary value.
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    assert!(client.get_tiered_historical_price(&asset, &100).is_none());
    assert!(!client.has_tiered_historical_price(&asset, &100));
}

/// The symmetric case: an entry that only exists in the *persistent* shard is
/// invisible once the asset is `Temporary`.
#[test]
fn test_no_cross_tier_fallback_persistent_value_hidden_by_temporary_tier() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 100, 1_000);

    // Move the asset to Persistent so the write below lands in the week bucket.
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    // Write an entry that exists ONLY in the persistent week bucket.
    let entry = PriceHistoryEntry {
        price: 8_888_888i128,
        timestamp: 1_000u64,
        ledger: 100u32,
        num_sources: 1u32,
        is_interpolated: false,
    };
    e.as_contract(&client.address, || {
        crate::storage_tier::write_history_entry(&e, &asset, &entry)
    });
    assert_eq!(
        client
            .get_tiered_historical_price(&asset, &100)
            .map(|x| x.price),
        Some(8_888_888i128)
    );

    // Downgrade (via the legitimate multi-party + timelock flow) and confirm
    // the shard value is not surfaced through the temporary tier.
    perform_legit_downgrade(&e, &client, &asset);

    assert_eq!(
        client.get_asset_storage_tier(&asset),
        HistoryStorageTier::Temporary
    );
    assert!(client.get_tiered_historical_price(&asset, &100).is_none());
    assert!(!client.has_tiered_historical_price(&asset, &100));
}

/// An empty ledger is simply absent — never interpolated by the tier-routed
/// reader (which has no interpolation logic at all).
#[test]
fn test_absent_ledger_is_never_fabricated() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 100, 1_000);
    submit_test_price(
        &client,
        &register_test_source(&e, &client, "S"),
        &asset,
        1i128,
        1_000u64,
    );

    client.set_min_sources_required(&1u32);
    for ledger in [50u32, 101u32, 250u32] {
        assert!(client
            .get_tiered_historical_price(&asset, &ledger)
            .is_none());
        assert!(!client.has_tiered_historical_price(&asset, &ledger));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. TTL expiry yields an explicit absent result
// ─────────────────────────────────────────────────────────────────────────────

/// Once the temporary entry's TTL lapses the host evicts it and the read yields
/// an explicit `None` — never a fabricated or interpolated price.
#[test]
fn test_temporary_ttl_expiry_yields_explicit_absent() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    ledger_default(&e, 100, 1_000);
    submit_test_price(&client, &source, &asset, 9_999_999i128, 1_000u64);

    let info = client.get_storage_tier_info(&asset);
    assert_eq!(info.tier, HistoryStorageTier::Temporary);
    assert!(!info.durable);
    assert_eq!(info.entry_count, 1u32);
    assert!(client.get_tiered_historical_price(&asset, &100).is_some());

    // Well past the documented retention window. Nothing reads the entry in
    // between, so only the host's own eviction can remove it.
    ledger_default(
        &e,
        100 + 200 * TEMPORARY_RETENTION_LEDGERS,
        1_000 + 100_000_000,
    );

    assert!(
        client.get_tiered_historical_price(&asset, &100).is_none(),
        "expired temporary entry must read as explicitly absent"
    );
    assert!(!client.has_tiered_historical_price(&asset, &100));
}

/// The same expiry on a `Persistent` asset does not lose the entry, because the
/// shard TTL is re-bumped far beyond the window.
#[test]
fn test_persistent_entry_survives_far_past_temporary_window() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    ledger_default(&e, 100, 1_000);
    submit_test_price(&client, &source, &asset, 5_555_555i128, 1_000u64);

    ledger_default(
        &e,
        100 + 200 * TEMPORARY_RETENTION_LEDGERS,
        1_000 + 100_000_000,
    );
    assert_eq!(
        client
            .get_tiered_historical_price(&asset, &100)
            .map(|e| e.price),
        Some(5_555_555i128)
    );
    assert!(client.get_storage_tier_info(&asset).durable);
}

// ─────────────────────────────────────────────────────────────────────────────
// 7. Migration path
// ─────────────────────────────────────────────────────────────────────────────

/// `migrate_history_to_tier` copies every indexed entry, the entries are then
/// readable from the target tier, and the migration event is emitted.
#[test]
fn test_migrate_history_to_persistent_tier() {
    let e = Env::default();
    let (client, admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    let mut expected: u32 = 0;
    for i in 1..=4u32 {
        ledger_default(&e, i * 10, i as u64 * 100);
        submit_test_price(
            &client,
            &source,
            &asset,
            1_000_000i128 * i as i128,
            i as u64 * 100,
        );
        expected += 1;
    }

    ledger_default(&e, 500, 50_000);
    let migrated = client.migrate_history_to_tier(&asset, &HistoryStorageTier::Persistent);
    assert_eq!(migrated, expected);
    let snap = oracle_events(&e, &client);

    // Entries are now readable from the persistent tier.
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    for i in 1..=4u32 {
        assert_eq!(
            client
                .get_tiered_historical_price(&asset, &(i * 10))
                .map(|e| e.price),
            Some(1_000_000i128 * i as i128)
        );
    }

    let ledger = 500u32;
    let found = snap.contains(
        &StorageTierMigratedEvent {
            asset: asset.clone(),
            from_tier: HistoryStorageTier::Temporary.as_u32(),
            to_tier: HistoryStorageTier::Persistent.as_u32(),
            entries_migrated: expected,
            actor: admin,
            ledger,
        }
        .to_xdr(&e, &client.address),
    );
    assert!(found, "StorageTierMigratedEvent must be emitted");

    // The migration ledger is discoverable via StorageTierInfo.
    assert_eq!(
        client.get_storage_tier_info(&asset).changed_at_ledger,
        500u32
    );
}

/// Migrating to the tier the asset is already on is a no-op returning `0`.
#[test]
fn test_migrate_to_same_tier_is_noop() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 10, 1_000);
    submit_test_price(&client, &source, &asset, 1_000i128, 1_000u64);

    assert_eq!(
        client.migrate_history_to_tier(&asset, &HistoryStorageTier::Temporary),
        0u32
    );
}

/// Migration is admin-only and leaves the source tier intact.
#[test]
fn test_migration_is_admin_only_and_non_destructive() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 10, 1_000);
    submit_test_price(&client, &source, &asset, 1_000i128, 1_000u64);

    clear_auth(&e);
    assert!(client
        .try_migrate_history_to_tier(&asset, &HistoryStorageTier::Persistent)
        .is_err());

    e.mock_all_auths();
    client.migrate_history_to_tier(&asset, &HistoryStorageTier::Persistent);
    // Source tier entry untouched.
    let key = DataKey::PriceHistory(asset.clone(), 10u32);
    let still: Option<PriceHistoryEntry> =
        e.as_contract(&client.address, || e.storage().temporary().get(&key));
    assert_eq!(still.map(|x| x.price), Some(1_000i128));
}

/// Migration back down to `Temporary` copies out of the shard without deleting
/// it.
#[test]
fn test_migrate_persistent_to_temporary() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    ledger_default(&e, 10, 1_000);
    submit_test_price(&client, &source, &asset, 4_000i128, 1_000u64);
    ledger_default(&e, 20, 2_000);
    submit_test_price(&client, &source, &asset, 5_000i128, 2_000u64);

    let migrated = client.migrate_history_to_tier(&asset, &HistoryStorageTier::Temporary);
    assert_eq!(migrated, 2u32);

    perform_legit_downgrade(&e, &client, &asset);
    assert_eq!(
        client
            .get_tiered_historical_price(&asset, &10)
            .map(|x| x.price),
        Some(4_000i128)
    );
    assert_eq!(
        client
            .get_tiered_historical_price(&asset, &20)
            .map(|x| x.price),
        Some(5_000i128)
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// 8. Retention guarantee verified against real data
// ─────────────────────────────────────────────────────────────────────────────

/// Automated check that the documented `Temporary` retention window is actually
/// satisfied by real submitted data: every ledger in the index is still returned
/// by the tier-routed reader, with no manual TTL bumping anywhere in the test.
#[test]
fn test_temporary_retention_guarantee_holds_for_real_data() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);

    // Spread submissions across ledgers, then move the ledger forward by most of
    // the retention window without touching storage directly.
    let mut written: std::vec::Vec<(u32, i128)> = std::vec::Vec::new();
    for i in 1..=6u32 {
        let seq = i * 100;
        ledger_default(&e, seq, seq as u64 * 10);
        let price = 1_000_000i128 + i as i128;
        submit_test_price(&client, &source, &asset, price, seq as u64 * 10);
        written.push((seq, price));
    }

    let info = client.get_storage_tier_info(&asset);
    assert_eq!(info.retention_ledgers, TEMPORARY_RETENTION_LEDGERS);
    assert!(!info.durable);
    assert_eq!(info.entry_count, written.len() as u32);

    // The host's own eviction is the only thing acting on the TTLs here.
    let last_seq = written[written.len() - 1].0;
    let target = last_seq + TEMPORARY_RETENTION_LEDGERS - 1_000;
    ledger_default(&e, target, target as u64 * 10);

    let mut checked = 0u32;
    for (seq, price) in &written {
        // Entries written early in the window may legitimately have lapsed
        // (temporary storage is not archival); entries inside the window must
        // still be there with their exact value.
        if *seq + TEMPORARY_RETENTION_LEDGERS > target {
            assert_eq!(
                client
                    .get_tiered_historical_price(&asset, seq)
                    .map(|x| x.price),
                Some(*price),
                "entry at ledger {} inside the retention window must be readable",
                seq
            );
            checked += 1;
        }
    }
    assert!(
        checked > 0,
        "the check must have verified at least one in-window entry"
    );
}

/// The guarantee is symmetric for `Persistent`: every indexed entry stays
/// readable indefinitely with no bumping.
#[test]
fn test_persistent_retention_guarantee_holds_for_real_data() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);

    let mut written: std::vec::Vec<(u32, i128)> = std::vec::Vec::new();
    for i in 1..=5u32 {
        let seq = i * 100;
        ledger_default(&e, seq, seq as u64 * 10);
        let price = 2_000_000i128 + i as i128;
        submit_test_price(&client, &source, &asset, price, seq as u64 * 10);
        written.push((seq, price));
    }

    // Advance far past the temporary window without reading in between.
    ledger_default(&e, 100 * TEMPORARY_RETENTION_LEDGERS, 100_000_000);

    let info = client.get_storage_tier_info(&asset);
    assert!(info.durable);
    assert_eq!(info.entry_count, written.len() as u32);
    for (seq, price) in &written {
        assert_eq!(
            client
                .get_tiered_historical_price(&asset, seq)
                .map(|x| x.price),
            Some(*price),
            "persistent entry at ledger {} must remain readable",
            seq
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tier-routed removal
// ─────────────────────────────────────────────────────────────────────────────

/// `remove_history_entry` clears the entry from the asset's configured tier and
/// is a silent no-op when absent.
#[test]
fn test_remove_history_entry_is_tier_routed_and_silent() {
    let e = Env::default();
    let (client, _admin) = setup_contract(&e);
    client.set_min_sources_required(&1u32);
    let source = register_test_source(&e, &client, "S");
    let asset = register_test_asset(&e, &client);
    ledger_default(&e, 10, 1_000);
    submit_test_price(&client, &source, &asset, 3_000i128, 1_000u64);
    assert!(client.get_tiered_historical_price(&asset, &10).is_some());

    // Removing an entry that was never written is a no-op, not a panic.
    e.as_contract(&client.address, || {
        crate::storage_tier::remove_history_entry(&e, &asset, 999_999)
    });
    assert!(client.get_tiered_historical_price(&asset, &10).is_some());

    // Real removal from the temporary tier.
    e.as_contract(&client.address, || {
        crate::storage_tier::remove_history_entry(&e, &asset, 10)
    });
    assert!(client.get_tiered_historical_price(&asset, &10).is_none());

    // Same for the persistent tier, shrinking the week bucket.
    client.set_asset_storage_tier(&asset, &HistoryStorageTier::Persistent);
    e.as_contract(&client.address, || {
        crate::storage_tier::write_history_entry(
            &e,
            &asset,
            &PriceHistoryEntry {
                price: 6_000i128,
                timestamp: 2_000u64,
                ledger: 20u32,
                num_sources: 1u32,
                is_interpolated: false,
            },
        )
    });
    assert!(client.get_tiered_historical_price(&asset, &20).is_some());
    e.as_contract(&client.address, || {
        crate::storage_tier::remove_history_entry(&e, &asset, 20)
    });
    assert!(client.get_tiered_historical_price(&asset, &20).is_none());

    let bucket_key = DataKey::HistoryBucket(asset.clone(), 0u32);
    let bucket: Option<Vec<PriceHistoryEntry>> = e.as_contract(&client.address, || {
        e.storage().persistent().get(&bucket_key)
    });
    let bucket = bucket.expect("the ledger-10 shard entry must still be there");
    assert_eq!(bucket.len(), 1u32, "only the removed entry may be dropped");
    assert_eq!(bucket.get_unchecked(0).ledger, 10u32);

    // Emptying the bucket entirely must remove the key, not leave a husk.
    e.as_contract(&client.address, || {
        crate::storage_tier::remove_history_entry(&e, &asset, 10)
    });
    let empty: Option<Vec<PriceHistoryEntry>> = e.as_contract(&client.address, || {
        e.storage().persistent().get(&bucket_key)
    });
    assert!(
        empty.is_none(),
        "an emptied bucket key must not be left behind"
    );
}
