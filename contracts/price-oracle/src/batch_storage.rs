//! # Gas-Efficient Batch Storage Read (#295)
//!
//! Provides `get_storage_batch` — a single contract call that reads multiple
//! storage keys at once, avoiding the per-call overhead of individual reads.
//!
//! ## Design
//!
//! Each [`StorageBatchRequest`] specifies:
//! - which storage tier to query (`persistent`, `temporary`, or `instance`)
//! - a [`DataKey`] variant to look up
//!
//! The call returns one [`StorageBatchResult`] per request.  Missing entries
//! produce `value_json = None` and `exists = false`; present entries are
//! serialised to a human-readable string representation.
//!
//! ## Gas Savings
//!
//! Reading N keys individually costs O(N) cross-contract call overheads.
//! `get_storage_batch` performs all reads inside one invocation, amortising
//! that overhead across the whole batch.

use soroban_sdk::{Env, String, Vec};

use crate::types::{DataKey, StorageBatchRequest, StorageBatchResult, StorageTier};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Reads multiple storage keys in a single call.
///
/// For each entry in `requests` the function:
/// 1. Selects the storage tier specified by [`StorageBatchRequest::tier`].
/// 2. Checks whether the key exists.
/// 3. Returns a [`StorageBatchResult`] with the existence flag and — where the
///    value is a type with a simple string representation — a serialised value.
///
/// Keys that do not exist return `exists = false` and `value_json = None`.
///
/// # Gas note
///
/// All reads are performed in a single transaction, making this significantly
/// more gas-efficient than N individual `get_*` calls when N > 1.
///
/// # Panics
///
/// Does not panic; absent keys are surfaced as `exists = false`.
pub fn get_storage_batch(env: &Env, requests: Vec<StorageBatchRequest>) -> Vec<StorageBatchResult> {
    let mut results: Vec<StorageBatchResult> = Vec::new(env);

    let len = requests.len();
    for i in 0..len {
        let req = requests.get_unchecked(i);
        let result = read_one(env, &req);
        results.push_back(result);
    }

    results
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Read a single storage entry and return its result.
fn read_one(env: &Env, req: &StorageBatchRequest) -> StorageBatchResult {
    match req.tier {
        StorageTier::Persistent => read_persistent(env, req),
        StorageTier::Temporary => read_temporary(env, req),
        StorageTier::Instance => read_instance(env, req),
    }
}

fn read_persistent(env: &Env, req: &StorageBatchRequest) -> StorageBatchResult {
    let key = &req.key;
    let exists = env.storage().persistent().has(key);

    let value_json: Option<String> = if exists {
        Some(serialise_key_value_persistent(env, key))
    } else {
        None
    };

    StorageBatchResult {
        key: key.clone(),
        tier: req.tier.clone(),
        exists,
        value_json,
    }
}

fn read_temporary(env: &Env, req: &StorageBatchRequest) -> StorageBatchResult {
    let key = &req.key;
    let exists = env.storage().temporary().has(key);

    let value_json: Option<String> = if exists {
        Some(serialise_key_value_temporary(env, key))
    } else {
        None
    };

    StorageBatchResult {
        key: key.clone(),
        tier: req.tier.clone(),
        exists,
        value_json,
    }
}

fn read_instance(env: &Env, req: &StorageBatchRequest) -> StorageBatchResult {
    let key = &req.key;
    let exists = env.storage().instance().has(key);

    let value_json: Option<String> = if exists {
        Some(serialise_key_value_instance(env, key))
    } else {
        None
    };

    StorageBatchResult {
        key: key.clone(),
        tier: req.tier.clone(),
        exists,
        value_json,
    }
}

/// Attempt to read a persistent value and return a short description string.
/// We use `u32` as a generic probe; unknown / composite types fall back to a
/// generic "present" marker so the caller at least knows the key exists.
fn serialise_key_value_persistent(env: &Env, key: &DataKey) -> String {
    // Try reading as bool first (covers flags like PauseFlag, SrcActive, etc.)
    if let Some(v) = env.storage().persistent().get::<DataKey, bool>(key) {
        return if v {
            String::from_str(env, "true")
        } else {
            String::from_str(env, "false")
        };
    }
    // Try reading as u32 (covers counters and config values)
    if let Some(v) = env.storage().persistent().get::<DataKey, u32>(key) {
        return format_u32(env, v);
    }
    // Try reading as i128 (covers prices, balances)
    if let Some(v) = env.storage().persistent().get::<DataKey, i128>(key) {
        return format_i128(env, v);
    }
    // Fallback: key exists but value type is complex
    String::from_str(env, "<present>")
}

fn serialise_key_value_temporary(env: &Env, key: &DataKey) -> String {
    if let Some(v) = env.storage().temporary().get::<DataKey, bool>(key) {
        return if v {
            String::from_str(env, "true")
        } else {
            String::from_str(env, "false")
        };
    }
    if let Some(v) = env.storage().temporary().get::<DataKey, u32>(key) {
        return format_u32(env, v);
    }
    if let Some(v) = env.storage().temporary().get::<DataKey, i128>(key) {
        return format_i128(env, v);
    }
    String::from_str(env, "<present>")
}

fn serialise_key_value_instance(env: &Env, key: &DataKey) -> String {
    if let Some(v) = env.storage().instance().get::<DataKey, bool>(key) {
        return if v {
            String::from_str(env, "true")
        } else {
            String::from_str(env, "false")
        };
    }
    if let Some(v) = env.storage().instance().get::<DataKey, u32>(key) {
        return format_u32(env, v);
    }
    if let Some(v) = env.storage().instance().get::<DataKey, i128>(key) {
        return format_i128(env, v);
    }
    String::from_str(env, "<present>")
}

// ---------------------------------------------------------------------------
// Minimal no_std integer formatters
// ---------------------------------------------------------------------------

/// Format a `u32` as a decimal string.
fn format_u32(env: &Env, mut v: u32) -> String {
    if v == 0 {
        return String::from_str(env, "0");
    }
    let mut buf = [0u8; 10]; // max 10 digits for u32
    let mut pos = 10usize;
    while v > 0 {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
    }
    // buf[pos..10] holds the ASCII digits
    let slice = &buf[pos..];
    // Build a soroban String from the byte slice via from_str with a fixed-size str.
    // We iterate the digits and build the string character by character.
    build_string_from_bytes(env, slice)
}

/// Format an `i128` as a decimal string (handles negatives).
fn format_i128(env: &Env, v: i128) -> String {
    if v == 0 {
        return String::from_str(env, "0");
    }
    let negative = v < 0;
    let mut abs: u128 = if negative {
        // i128::MIN cannot be negated directly; handle by wrapping
        (v as i128).unsigned_abs()
    } else {
        v as u128
    };

    let mut buf = [0u8; 40]; // max 39 digits for u128 + sign
    let mut pos = 40usize;
    while abs > 0 {
        pos -= 1;
        buf[pos] = b'0' + (abs % 10) as u8;
        abs /= 10;
    }
    if negative {
        pos -= 1;
        buf[pos] = b'-';
    }
    build_string_from_bytes(env, &buf[pos..])
}

/// Build a Soroban `String` from a byte slice of ASCII characters.
/// This uses `String::from_str` with a known-static mapping via short fixed strings.
/// Because `soroban_sdk::String::from_str` requires a `&str` literal, we copy
/// into a fixed-size array and convert.
fn build_string_from_bytes(env: &Env, bytes: &[u8]) -> String {
    // Maximum expected length is 40 (i128 in decimal).
    // We use a stack array and convert the slice to &str.
    let mut arr = [0u8; 41];
    let len = bytes.len().min(40);
    arr[..len].copy_from_slice(&bytes[..len]);
    // Safety: all bytes are ASCII digits or '-'.
    let s = core::str::from_utf8(&arr[..len]).unwrap_or("?");
    String::from_str(env, s)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, LedgerInfo},
        Address, Env,
    };

    use crate::test_helpers::setup_contract;
    use crate::types::{StorageBatchRequest, StorageTier};

    fn ledger_at(e: &Env, seq: u32, ts: u64) {
        e.ledger().set(LedgerInfo {
            timestamp: ts,
            protocol_version: 26,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 10,
            min_persistent_entry_ttl: 10,
            max_entry_ttl: 4096,
        });
    }

    fn make_request(e: &Env, key: DataKey, tier: StorageTier) -> StorageBatchRequest {
        StorageBatchRequest { key, tier }
    }

    // ── #295 Test 1: empty batch returns empty results ────────────────────────
    #[test]
    fn test_batch_empty() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let requests: Vec<StorageBatchRequest> = Vec::new(&e);
        let results = client.get_storage_batch(&requests);
        assert_eq!(results.len(), 0);
    }

    // ── #295 Test 2: existing persistent key returns exists=true ────────────
    #[test]
    fn test_batch_existing_persistent_key() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);

        // CfgMinSources is always written during initialize
        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::CfgMinSources,
            StorageTier::Persistent,
        ));

        let results = client.get_storage_batch(&requests);
        assert_eq!(results.len(), 1);
        let r = results.get_unchecked(0);
        assert!(r.exists);
        assert!(r.value_json.is_some());
    }

    // ── #295 Test 3: missing key returns exists=false ────────────────────────
    #[test]
    fn test_batch_missing_key() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);

        // Use an address-keyed DataKey for a non-existent address
        let dummy = Address::generate(&e);
        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::SrcActive(dummy),
            StorageTier::Persistent,
        ));

        let results = client.get_storage_batch(&requests);
        assert_eq!(results.len(), 1);
        let r = results.get_unchecked(0);
        assert!(!r.exists);
        assert!(r.value_json.is_none());
    }

    // ── #295 Test 4: mixed existing + missing batch ───────────────────────────
    #[test]
    fn test_batch_mixed() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let dummy = Address::generate(&e);

        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::CfgDecimals,
            StorageTier::Persistent,
        ));
        requests.push_back(make_request(
            &e,
            DataKey::SrcActive(dummy.clone()),
            StorageTier::Persistent,
        ));
        requests.push_back(make_request(
            &e,
            DataKey::CfgMaxHistory,
            StorageTier::Persistent,
        ));

        let results = client.get_storage_batch(&requests);
        assert_eq!(results.len(), 3);
        assert!(results.get_unchecked(0).exists);
        assert!(!results.get_unchecked(1).exists);
        assert!(results.get_unchecked(2).exists);
    }

    // ── #295 Test 5: multiple config keys all present ────────────────────────
    #[test]
    fn test_batch_multiple_config_keys() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);

        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::CfgMinSources,
            StorageTier::Persistent,
        ));
        requests.push_back(make_request(
            &e,
            DataKey::CfgMaxHistory,
            StorageTier::Persistent,
        ));
        requests.push_back(make_request(
            &e,
            DataKey::CfgDecimals,
            StorageTier::Persistent,
        ));

        let results = client.get_storage_batch(&requests);
        assert_eq!(results.len(), 3);
        for i in 0..3u32 {
            assert!(results.get_unchecked(i).exists);
        }
    }

    // ── #295 Test 6: tier field is echoed back correctly ─────────────────────
    #[test]
    fn test_batch_tier_echoed() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);

        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::CfgMinSources,
            StorageTier::Persistent,
        ));

        let results = client.get_storage_batch(&requests);
        let r = results.get_unchecked(0);
        assert_eq!(r.tier, StorageTier::Persistent);
    }

    // ── #295 Test 7: key field is echoed back correctly ──────────────────────
    #[test]
    fn test_batch_key_echoed() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);

        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::CfgDecimals,
            StorageTier::Persistent,
        ));

        let results = client.get_storage_batch(&requests);
        let r = results.get_unchecked(0);
        assert_eq!(r.key, DataKey::CfgDecimals);
    }

    // ── #295 Test 8: registered source exists in persistent storage ──────────
    #[test]
    fn test_batch_registered_source_exists() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);
        let source = crate::test_helpers::register_test_source(&e, &client, "S1");

        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::SrcActive(source.clone()),
            StorageTier::Persistent,
        ));

        let results = client.get_storage_batch(&requests);
        assert!(results.get_unchecked(0).exists);
    }

    // ── #295 Test 9: batch reads are consistent with individual reads ─────────
    #[test]
    fn test_batch_consistent_with_individual() {
        let e = Env::default();
        ledger_at(&e, 100, 1_000_000);
        let (client, _admin) = setup_contract(&e);

        // Individual read
        let individual = client.get_min_sources_required();

        // Batch read of the same key
        let mut requests: Vec<StorageBatchRequest> = Vec::new(&e);
        requests.push_back(make_request(
            &e,
            DataKey::CfgMinSources,
            StorageTier::Persistent,
        ));
        let results = client.get_storage_batch(&requests);
        let r = results.get_unchecked(0);

        // Both agree the key exists and has a value
        assert!(r.exists);
        // The batch value should be the decimal string of the min sources
        let expected = crate::batch_storage::format_u32(&e, individual);
        assert_eq!(r.value_json, Some(expected));
    }
}
