//! # Ring-Buffer History Storage (#877)
//!
//! Evaluates and prototypes fixed-size circular history storage keyed by
//! `(asset, slot % capacity)` as an alternative to growing vectors.
//!
//! ## Design
//! - Storage key: `DataKey::PriceHistorySlot(asset, slot)`
//! - Meta key: `DataKey::RingBufferMeta(asset)`
//! - Writes: O(1) fixed write footprint (1 entry write + 1 bounded meta update).
//! - Reads: Direct O(1) lookup when ledger is within retained window `[head - len, head)`.

use soroban_sdk::{contracttype, Address, Env};
use crate::types::{DataKey, PriceHistoryEntry};

/// Metadata tracking the head and retained count of the circular buffer.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[contracttype]
pub struct RingBufferMeta {
    /// Next write slot sequence number.
    pub head: u32,
    /// Number of active items currently stored (bounded by `capacity`).
    pub count: u32,
    /// Fixed ring buffer capacity.
    pub capacity: u32,
}

/// Ring-buffer history storage manager.
pub struct RingBufferHistory;

impl RingBufferHistory {
    /// Default ring buffer capacity (e.g. 256 entries).
    pub const DEFAULT_CAPACITY: u32 = 256;

    /// Write a new history entry into the ring buffer slot.
    pub fn push(env: &Env, asset: &Address, entry: PriceHistoryEntry, capacity: u32) {
        let meta_key = DataKey::RingBufferMeta(asset.clone());
        let mut meta = env
            .storage()
            .temporary()
            .get::<_, RingBufferMeta>(&meta_key)
            .unwrap_or(RingBufferMeta {
                head: 0,
                count: 0,
                capacity: if capacity > 0 { capacity } else { Self::DEFAULT_CAPACITY },
            });

        let slot = meta.head % meta.capacity;
        let slot_key = DataKey::PriceHistorySlot(asset.clone(), slot);

        env.storage().temporary().set(&slot_key, &entry);

        meta.head = meta.head.wrapping_add(1);
        if meta.count < meta.capacity {
            meta.count += 1;
        }

        env.storage().temporary().set(&meta_key, &meta);
    }

    /// Direct O(1) lookup for a price history entry at `ledger`.
    pub fn get_by_ledger(env: &Env, asset: &Address, ledger: u32) -> Option<PriceHistoryEntry> {
        let meta_key = DataKey::RingBufferMeta(asset.clone());
        let meta = env.storage().temporary().get::<_, RingBufferMeta>(&meta_key)?;

        if meta.count == 0 {
            return None;
        }

        let earliest_ledger = meta.head.saturating_sub(meta.count);
        if ledger < earliest_ledger || ledger >= meta.head {
            return None;
        }

        let slot = ledger % meta.capacity;
        let slot_key = DataKey::PriceHistorySlot(asset.clone(), slot);
        let entry: PriceHistoryEntry = env.storage().temporary().get(&slot_key)?;

        if entry.ledger == ledger {
            Some(entry)
        } else {
            None
        }
    }

    /// Migrate legacy vector-based history into ring buffer slots.
    pub fn migrate_from_legacy(env: &Env, asset: &Address, capacity: u32) -> u32 {
        let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
        let ledgers = env
            .storage()
            .persistent()
            .get::<_, soroban_sdk::Vec<u32>>(&ledgers_key)
            .unwrap_or_else(|| soroban_sdk::Vec::new(env));

        let mut migrated = 0;
        for i in 0..ledgers.len() {
            let l = ledgers.get_unchecked(i);
            if let Some(entry) = crate::history::read_history_entry(env, asset, l) {
                Self::push(env, asset, entry, capacity);
                migrated += 1;
            }
        }
        migrated
    }
}
