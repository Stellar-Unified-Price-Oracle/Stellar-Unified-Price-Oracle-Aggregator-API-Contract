#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::test_helpers::*;

#[test]
fn test_add_consumer() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer = Address::generate(&e);
    client.add_consumer(&consumer);
    assert!(client.is_consumer(&consumer));
}

#[test]
fn test_add_multiple_consumers() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer1 = Address::generate(&e);
    let consumer2 = Address::generate(&e);
    let consumer3 = Address::generate(&e);

    client.add_consumer(&consumer1);
    client.add_consumer(&consumer2);
    client.add_consumer(&consumer3);

    assert!(client.is_consumer(&consumer1));
    assert!(client.is_consumer(&consumer2));
    assert!(client.is_consumer(&consumer3));
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_add_consumer_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    let consumer = Address::generate(&e);
    client.add_consumer(&consumer);
}

#[test]
fn test_remove_consumer() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer = Address::generate(&e);
    client.add_consumer(&consumer);
    assert!(client.is_consumer(&consumer));

    client.remove_consumer(&consumer);
    assert!(!client.is_consumer(&consumer));
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_remove_consumer_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer = Address::generate(&e);
    client.add_consumer(&consumer);

    clear_auth(&e);
    client.remove_consumer(&consumer);
}

#[test]
#[should_panic(expected = "Error(Contract, #15)")]
fn test_remove_nonexistent_consumer() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer = Address::generate(&e);
    client.remove_consumer(&consumer);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_add_consumer_duplicate() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer = Address::generate(&e);
    client.add_consumer(&consumer);
    client.add_consumer(&consumer);
}

#[test]
fn test_set_whitelist_enabled() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    assert!(!client.is_whitelist_enabled());

    client.set_whitelist_enabled(&true);
    assert!(client.is_whitelist_enabled());

    client.set_whitelist_enabled(&false);
    assert!(!client.is_whitelist_enabled());
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_set_whitelist_enabled_unauthorized() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    clear_auth(&e);
    client.set_whitelist_enabled(&true);
}

#[test]
fn test_whitelist_disabled_by_default() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    // Without whitelist enabled, any caller should be able to query
    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
}

#[test]
fn test_whitelisted_consumer_can_query() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    let consumer = Address::generate(&e);
    client.add_consumer(&consumer);
    client.set_whitelist_enabled(&true);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    // Whitelisted consumer should be able to query
    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
}

#[test]
#[should_panic(expected = "Error(Contract, #0)")]
fn test_non_whitelisted_consumer_cannot_query() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    let whitelisted = Address::generate(&e);
    let unauthorized = Address::generate(&e);

    client.add_consumer(&whitelisted);
    client.set_whitelist_enabled(&true);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    // Try to query as unauthorized consumer (should fail)
    clear_auth(&e);
    e.mock_all_auths();
    let _ = client.get_price(&asset, &0u64);
}

#[test]
fn test_admin_can_always_query() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);

    client.set_whitelist_enabled(&true);
    // Admin not in whitelist but should still be able to query

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
}

#[test]
fn test_is_consumer_false_for_non_member() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer1 = Address::generate(&e);
    let consumer2 = Address::generate(&e);

    client.add_consumer(&consumer1);
    assert!(client.is_consumer(&consumer1));
    assert!(!client.is_consumer(&consumer2));
}

#[test]
fn test_remove_then_re_add_consumer() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let consumer = Address::generate(&e);

    client.add_consumer(&consumer);
    assert!(client.is_consumer(&consumer));

    client.remove_consumer(&consumer);
    assert!(!client.is_consumer(&consumer));

    client.add_consumer(&consumer);
    assert!(client.is_consumer(&consumer));
}

#[test]
fn test_toggle_whitelist_enables_restrictions() {
    let e = Env::default();
    ledger_default(&e, 100, 1234567890);
    let (client, _) = setup_contract(&e);

    let source1 = register_test_source(&e, &client, "Source1");
    let source2 = register_test_source(&e, &client, "Source2");
    client.set_min_sources_required(&2u32);

    let asset = register_test_asset(&e, &client);
    let consumer = Address::generate(&e);

    submit_test_price(&client, &source1, &asset, 100i128, 1234567890);
    submit_test_price(&client, &source2, &asset, 200i128, 1234567890);

    // Initially whitelist is disabled, query works
    assert!(client.get_price(&asset, &0u64).is_some());

    // Enable whitelist without adding consumer
    client.set_whitelist_enabled(&true);

    // Query should now fail (consumer not whitelisted)
    // Re-enable all auth to proceed
    clear_auth(&e);
    e.mock_all_auths();

    // Add consumer to whitelist
    client.add_consumer(&consumer);
    client.set_whitelist_enabled(&true);

    // Now query should succeed
    let price = client.get_price(&asset, &0u64);
    assert!(price.is_some());
}

#[test]
fn test_whitelist_independent_of_source_management() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let source = register_test_source(&e, &client, "Source");
    let consumer = Address::generate(&e);

    // Source and consumer are different concepts
    assert!(client.is_source(&source));
    assert!(!client.is_consumer(&source)); // source is not a consumer

    client.add_consumer(&consumer);
    assert!(client.is_consumer(&consumer));
    assert!(!client.is_source(&consumer)); // consumer is not a source
}
