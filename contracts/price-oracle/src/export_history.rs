//! # Exportable History Snapshot
//!
//! Paginated export of on-chain price history for off-chain archiving and
//! regulatory compliance purposes.
//!
//! ## Design
//!
//! * [`export_history`] reads from the existing temporary-storage history index
//!   (`DataKey::PriceHistoryLedgers`) and emits an [`ExportedHistorySnapshot`]
//!   containing up to `limit` entries starting at `from_ledger`.
//! * The snapshot carries a lightweight `data_hash` — an XOR-fold over all
//!   entry prices — that off-chain archivers can recompute locally to verify
//!   the payload hasn't been tampered with.
//! * [`verify_export`] re-runs the same hash computation over whatever entries
//!   are still in storage and returns `true` when the provided `merkle_root`
//!   matches, giving on-chain proof that a previously-exported snapshot is
//!   consistent with the current state.
//!
//! ## Limits
//!
//! The maximum `limit` per call is [`MAX_EXPORT_LIMIT`] (200). Callers that
//! need more entries should use cursor-based pagination via `next_cursor`.

use soroban_sdk::{panic_with_error, Address, Env, String, Vec};

use crate::storage::{check_registered_asset, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{DataKey, ErrorCode, ExportedEntry, ExportedHistorySnapshot, PriceHistoryEntry};

/// Hard cap on how many entries a single `export_history` call may return.
pub const MAX_EXPORT_LIMIT: u32 = 200;

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// XOR-fold `hash` with a u64 derived from `price` for the data integrity token.
///
/// We use a simple XOR strategy because Soroban does not provide SHA-256 inside
/// `#[no_std]` without an external crate.  The hash is **not** collision-resistant;
/// it is a lightweight integrity token, not a cryptographic commitment.
#[inline]
fn mix_hash(acc: u64, price: i128) -> u64 {
    // Split the i128 into two u64 halves and XOR-fold both in.
    let lo = price as u64;
    let hi = (price >> 64) as u64;
    acc ^ lo ^ hi.wrapping_mul(0x9e3779b97f4a7c15)
}

// ---------------------------------------------------------------------------
// Public functions
// ---------------------------------------------------------------------------

/// Export up to `limit` history entries for `asset`, starting from `from_ledger`.
///
/// # Arguments
///
/// * `asset`       — Registered asset address.
/// * `from_ledger` — Inclusive start ledger (pass `0` to start from the beginning).
/// * `limit`       — Maximum entries to return (1..=[`MAX_EXPORT_LIMIT`]).
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`]  — `asset` not registered.
/// * [`ErrorCode::ExportLimitExceeded`] — `limit` is `0` or greater than
///   [`MAX_EXPORT_LIMIT`].
pub fn export_history(
    env: &Env,
    asset: Address,
    from_ledger: u32,
    limit: u32,
) -> ExportedHistorySnapshot {
    check_registered_asset(env, &asset);

    if limit == 0 || limit > MAX_EXPORT_LIMIT {
        panic_with_error!(env, ErrorCode::ExportLimitExceeded);
    }

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    let total_available = ledger_list.len();
    let mut entries: Vec<ExportedEntry> = Vec::new(env);
    let mut data_hash: u64 = 0u64;
    let mut first_ledger: u32 = 0;
    let mut last_ledger: u32 = 0;
    let mut next_cursor: u32 = 0;

    for i in 0..total_available {
        let l = ledger_list.get_unchecked(i);
        if l < from_ledger {
            continue;
        }
        if entries.len() >= limit {
            next_cursor = l;
            break;
        }

        let key = DataKey::PriceHistory(asset.clone(), l);
        if env.storage().temporary().has(&key) {
            env.storage()
                .temporary()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            let entry: PriceHistoryEntry = env.storage().temporary().get(&key).unwrap();

            if entries.is_empty() {
                first_ledger = l;
            }
            last_ledger = l;

            data_hash = mix_hash(data_hash, entry.price);

            entries.push_back(ExportedEntry {
                asset: asset.clone(),
                price: entry.price,
                timestamp: entry.timestamp,
                ledger: entry.ledger,
                num_sources: entry.num_sources,
                is_interpolated: entry.is_interpolated,
            });
        }
    }

    ExportedHistorySnapshot {
        entries,
        data_hash,
        from_ledger: first_ledger,
        to_ledger: last_ledger,
        total_available,
        next_cursor,
    }
}

/// Verify that the provided `data_hash` matches the XOR-fold hash of all currently
/// stored history entries for `asset` in the ledger range `[from_ledger, to_ledger]`.
///
/// Returns `true` when the hash matches, `false` otherwise.
///
/// # Errors
///
/// * [`ErrorCode::AssetNotRegistered`] — `asset` not registered.
/// * [`ErrorCode::ExportNotFound`]     — no history entries exist in the given range.
pub fn verify_export(
    env: &Env,
    asset: Address,
    from_ledger: u32,
    to_ledger: u32,
    expected_data_hash: u64,
) -> bool {
    check_registered_asset(env, &asset);

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledger_list: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    let mut computed_hash: u64 = 0u64;
    let mut found_any = false;

    for i in 0..ledger_list.len() {
        let l = ledger_list.get_unchecked(i);
        if l < from_ledger || l > to_ledger {
            continue;
        }
        let key = DataKey::PriceHistory(asset.clone(), l);
        if env.storage().temporary().has(&key) {
            let entry: PriceHistoryEntry = env.storage().temporary().get(&key).unwrap();
            computed_hash = mix_hash(computed_hash, entry.price);
            found_any = true;
        }
    }

    if !found_any {
        panic_with_error!(env, ErrorCode::ExportNotFound);
    }

    computed_hash == expected_data_hash
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, LedgerInfo},
        Env,
    };

    use crate::test_helpers::{register_test_asset, register_test_source, setup_contract};

    fn set_ledger(e: &Env, seq: u32, ts: u64) {
        e.ledger().set(LedgerInfo {
            timestamp: ts,
            protocol_version: 26,
            sequence_number: seq,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 16,
            min_persistent_entry_ttl: 16,
            max_entry_ttl: 4096,
        });
    }

    /// Helper: submit prices from two sources to build history entries.
    fn build_history(
        e: &Env,
        client: &crate::PriceOracleContractClient,
        source1: &Address,
        source2: &Address,
        asset: &Address,
        prices: &[(u32, u64, i128)], // (ledger, timestamp, price)
    ) {
        for (seq, ts, price) in prices {
            set_ledger(e, *seq, *ts);
            client.submit_price(source1, asset, price, ts);
            client.submit_price(source2, asset, price, ts);
        }
    }

    #[test]
    fn test_export_empty_asset_returns_empty_snapshot() {
        let e = Env::default();
        set_ledger(&e, 100, 1_000_000);
        let (client, _) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);

        let snapshot = client.export_history(&asset, &0u32, &10u32);
        assert_eq!(snapshot.entries.len(), 0);
        assert_eq!(snapshot.total_available, 0);
        assert_eq!(snapshot.next_cursor, 0);
    }

    #[test]
    fn test_export_returns_entries_in_range() {
        let e = Env::default();
        set_ledger(&e, 100, 1_000_000);
        let (client, _) = setup_contract(&e);
        client.set_min_sources_required(&2u32);
        let s1 = register_test_source(&e, &client, "S1");
        let s2 = register_test_source(&e, &client, "S2");
        let asset = register_test_asset(&e, &client);

        build_history(
            &e,
            &client,
            &s1,
            &s2,
            &asset,
            &[
                (101, 1_000_001, 1_000),
                (102, 1_000_002, 2_000),
                (103, 1_000_003, 3_000),
            ],
        );

        let snapshot = client.export_history(&asset, &0u32, &10u32);
        assert_eq!(snapshot.entries.len(), 3);
        assert_eq!(snapshot.from_ledger, 101);
        assert_eq!(snapshot.to_ledger, 103);
        assert_eq!(snapshot.next_cursor, 0);
    }

    #[test]
    fn test_export_pagination_with_limit() {
        let e = Env::default();
        set_ledger(&e, 100, 1_000_000);
        let (client, _) = setup_contract(&e);
        client.set_min_sources_required(&2u32);
        let s1 = register_test_source(&e, &client, "S1");
        let s2 = register_test_source(&e, &client, "S2");
        let asset = register_test_asset(&e, &client);

        build_history(
            &e,
            &client,
            &s1,
            &s2,
            &asset,
            &[
                (101, 1_000_001, 1_000),
                (102, 1_000_002, 2_000),
                (103, 1_000_003, 3_000),
                (104, 1_000_004, 4_000),
                (105, 1_000_005, 5_000),
            ],
        );

        // Page 1: limit = 2
        let page1 = client.export_history(&asset, &0u32, &2u32);
        assert_eq!(page1.entries.len(), 2);
        let next = page1.next_cursor;
        assert!(next > 0);

        // Page 2: start from next_cursor, limit = 2
        let page2 = client.export_history(&asset, &next, &2u32);
        assert_eq!(page2.entries.len(), 2);

        // Page 3: remaining 1 entry
        let page3 = client.export_history(&asset, &page2.next_cursor, &2u32);
        assert_eq!(page3.entries.len(), 1);
        assert_eq!(page3.next_cursor, 0);
    }

    #[test]
    fn test_export_data_hash_consistent() {
        let e = Env::default();
        set_ledger(&e, 100, 1_000_000);
        let (client, _) = setup_contract(&e);
        client.set_min_sources_required(&2u32);
        let s1 = register_test_source(&e, &client, "S1");
        let s2 = register_test_source(&e, &client, "S2");
        let asset = register_test_asset(&e, &client);

        build_history(
            &e,
            &client,
            &s1,
            &s2,
            &asset,
            &[(101, 1_000_001, 5_000), (102, 1_000_002, 6_000)],
        );

        let snapshot = client.export_history(&asset, &0u32, &10u32);
        let hash = snapshot.data_hash;

        // verify_export should confirm the hash matches
        let valid = client.verify_export(&asset, &101u32, &102u32, &hash);
        assert!(valid);
    }

    #[test]
    fn test_verify_export_wrong_hash_returns_false() {
        let e = Env::default();
        set_ledger(&e, 100, 1_000_000);
        let (client, _) = setup_contract(&e);
        client.set_min_sources_required(&2u32);
        let s1 = register_test_source(&e, &client, "S1");
        let s2 = register_test_source(&e, &client, "S2");
        let asset = register_test_asset(&e, &client);

        build_history(&e, &client, &s1, &s2, &asset, &[(101, 1_000_001, 5_000)]);

        let wrong_hash: u64 = 0xDEADBEEF;
        let valid = client.verify_export(&asset, &101u32, &101u32, &wrong_hash);
        assert!(!valid);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #103)")]
    fn test_export_limit_zero_panics() {
        let e = Env::default();
        set_ledger(&e, 100, 1_000_000);
        let (client, _) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);
        client.export_history(&asset, &0u32, &0u32);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #103)")]
    fn test_export_limit_too_large_panics() {
        let e = Env::default();
        set_ledger(&e, 100, 1_000_000);
        let (client, _) = setup_contract(&e);
        let asset = register_test_asset(&e, &client);
        client.export_history(&asset, &0u32, &201u32);
    }
}
