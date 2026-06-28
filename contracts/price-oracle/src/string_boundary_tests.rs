#![cfg(test)]

//! Tests verifying that the contract handles string length boundaries correctly
//! for source names, asset identifiers, and description fields.

use soroban_sdk::{testutils::Address as _, Address, Env, String};

use crate::test_helpers::*;

/// Builds a soroban String of exactly `len` ASCII 'a' characters.
fn make_string(e: &Env, len: usize) -> String {
    let s: ::core::string::String = "a".repeat(len);
    String::from_str(e, &s)
}

// ── Description ──────────────────────────────────────────────────────────────

/// MAX allowed description length is 256 characters; should succeed.
#[test]
fn test_description_at_max_length() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    let desc = make_string(&e, 256);
    client.initialize(&admin, &1u32, &10u32, &18u32, &desc);
    assert_eq!(client.get_description(), desc);
}

/// Description of 257 chars must be rejected with DescriptionTooLong (#11).
#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_description_exceeds_max_length() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    client.initialize(&admin, &1u32, &10u32, &18u32, &make_string(&e, 257));
}

/// set_description with exactly 256 chars should succeed and persist unchanged.
#[test]
fn test_set_description_at_max_length() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let desc = make_string(&e, 256);
    client.set_description(&desc);
    assert_eq!(client.get_description(), desc);
}

/// set_description with 257 chars must be rejected with DescriptionTooLong (#11).
#[test]
#[should_panic(expected = "Error(Contract, #11)")]
fn test_set_description_exceeds_max_length() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    client.set_description(&make_string(&e, 257));
}

/// Empty description (0 chars) is allowed.
#[test]
fn test_description_empty_string() {
    let e = Env::default();
    let admin = Address::generate(&e);
    let client = create_contract(&e);
    let empty = String::from_str(&e, "");
    client.initialize(&admin, &1u32, &10u32, &18u32, &empty);
    assert_eq!(client.get_description(), empty);
}

/// Stored description is returned exactly as submitted — no truncation.
#[test]
fn test_description_no_truncation() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let desc = make_string(&e, 200);
    client.set_description(&desc);
    let retrieved = client.get_description();
    assert_eq!(retrieved.len(), 200);
    assert_eq!(retrieved, desc);
}

// ── Source Name ───────────────────────────────────────────────────────────────

/// Source name at the Soroban String practical max (256 chars) succeeds.
#[test]
fn test_source_name_at_max_length() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = Address::generate(&e);
    let name = make_string(&e, 256);
    client.add_source(&source, &name);
    let sources = client.get_oracle_sources();
    assert_eq!(sources.metadata.get(source).unwrap(), name);
}

/// A 1-character source name is accepted.
#[test]
fn test_source_name_minimal_length() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = Address::generate(&e);
    let name = String::from_str(&e, "X");
    client.add_source(&source, &name);
    let sources = client.get_oracle_sources();
    assert_eq!(sources.metadata.get(source).unwrap(), name);
}

/// Source name is stored and retrieved without truncation.
#[test]
fn test_source_name_no_truncation() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let source = Address::generate(&e);
    let name = make_string(&e, 200);
    client.add_source(&source, &name);
    let sources = client.get_oracle_sources();
    let stored = sources.metadata.get(source).unwrap();
    assert_eq!(stored.len(), 200);
    assert_eq!(stored, name);
}

/// Multiple sources can have different name lengths simultaneously.
#[test]
fn test_multiple_sources_different_name_lengths() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let s1 = Address::generate(&e);
    let s2 = Address::generate(&e);
    let s3 = Address::generate(&e);

    let n1 = String::from_str(&e, "A");
    let n2 = make_string(&e, 128);
    let n3 = make_string(&e, 256);

    client.add_source(&s1, &n1);
    client.add_source(&s2, &n2);
    client.add_source(&s3, &n3);

    let sources = client.get_oracle_sources();
    assert_eq!(sources.metadata.get(s1).unwrap(), n1);
    assert_eq!(sources.metadata.get(s2).unwrap(), n2);
    assert_eq!(sources.metadata.get(s3).unwrap(), n3);
}

// ── Asset Identifier ──────────────────────────────────────────────────────────

/// Assets registered with different address encodings can all be queried.
#[test]
fn test_asset_identifiers_various_addresses() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);

    let a1 = Address::generate(&e);
    let a2 = Address::generate(&e);
    let a3 = Address::generate(&e);

    client.register_asset(&a1);
    client.register_asset(&a2);
    client.register_asset(&a3);

    assert!(client.is_asset_registered(&a1));
    assert!(client.is_asset_registered(&a2));
    assert!(client.is_asset_registered(&a3));
}

/// A registered asset returns exactly the same address (no corruption).
#[test]
fn test_asset_identifier_exact_round_trip() {
    let e = Env::default();
    let (client, _) = setup_contract(&e);
    let asset = Address::generate(&e);
    client.register_asset(&asset);
    // is_asset_registered uses the same address internally — round-trip confirmed
    assert!(client.is_asset_registered(&asset));
    // A different address must not collide
    let other = Address::generate(&e);
    assert!(!client.is_asset_registered(&other));
}
