//! Feature flags that keep unused optional settings out of a transaction's
//! storage footprint.
//!
//! Soroban caps how many ledger entries one transaction may touch. Every
//! optional setting (outlier filter, triggers, blackout windows, ...) used to
//! be read on each price submission even when nobody had configured it, so a
//! plain submit came close to the cap on its own. Each setting now owns a bit
//! here: setters call [`mark`] when they write the setting, and hot-path
//! readers call [`on`] first and fall back to the default when the bit is
//! clear. The bitmask lives in instance storage, which every invocation loads
//! anyway, so checking it adds no footprint entries.
//!
//! Bits are only ever set, never cleared. A set bit with no stored value just
//! means a wasted read, never a skipped one.
//!
//! Contracts upgraded from a build without flags must call
//! `enable_all_feature_flags` once so that settings written before the upgrade
//! are still honoured.

use soroban_sdk::{symbol_short, Env, Symbol};

const KEY: Symbol = symbol_short!("FEATFLAG");

pub const INFLUENCE_CAP: u64 = 1 << 0;
pub const ANOMALY_RETENTION: u64 = 1 << 1;
pub const BFT: u64 = 1 << 2;
pub const COMPACTION_THRESHOLD: u64 = 1 << 3;
pub const PAUSE: u64 = 1 << 4;
pub const TIMESTAMP_THRESHOLD: u64 = 1 << 5;
pub const COMMIT_REVEAL: u64 = 1 << 6;
pub const CONTRIB_SCORING_WINDOW: u64 = 1 << 7;
pub const CORRELATION: u64 = 1 << 8;
pub const FRESHNESS_WINDOW: u64 = 1 << 9;
pub const MAX_AGGREGATION_SOURCES: u64 = 1 << 10;
pub const MAX_EVENTS_PER_CALL: u64 = 1 << 11;
pub const MAX_HISTORY_PER_ASSET: u64 = 1 << 12;
pub const MIN_SUBMISSION_INTERVAL: u64 = 1 << 13;
pub const PUBLICATION_GUARDS: u64 = 1 << 14;
pub const RISK_TIER: u64 = 1 << 15;
pub const SOURCE_BOND: u64 = 1 << 16;
pub const ASSET_POLICY: u64 = 1 << 17;
pub const ASSET_DEFERRAL: u64 = 1 << 18;
pub const ASSET_DISABLED: u64 = 1 << 19;
// 1 << 20 was ASSET_MIN_SUBMISSION_INTERVAL; reserved so no other flag reuses it.
pub const STORAGE_TIER: u64 = 1 << 21;
pub const SOURCE_LIFECYCLE: u64 = 1 << 22;
pub const DQ_PIPELINE: u64 = 1 << 23;
pub const SOURCE_COMPARISON: u64 = 1 << 24;
pub const FRESHNESS_CURVE: u64 = 1 << 25;
pub const FREEZE: u64 = 1 << 26;
pub const OUTLIER_FILTER: u64 = 1 << 27;
pub const PRICE_CALLBACK: u64 = 1 << 28;
pub const TRIGGERS: u64 = 1 << 29;
pub const BLACKOUT: u64 = 1 << 30;
pub const SOURCE_ASSETS: u64 = 1 << 31;
pub const SOURCE_DEMERITS: u64 = 1 << 32;
pub const SOURCE_INACTIVE: u64 = 1 << 33;

fn read(env: &Env) -> u64 {
    env.storage().instance().get(&KEY).unwrap_or(0)
}

/// True when the setting behind `bit` may have been written.
pub fn on(env: &Env, bit: u64) -> bool {
    read(env) & bit != 0
}

/// Records that the setting behind `bit` has been written.
pub fn mark(env: &Env, bit: u64) {
    let flags = read(env);
    if flags & bit != bit {
        env.storage().instance().set(&KEY, &(flags | bit));
    }
}

/// Sets every bit, restoring the read-everything behaviour. For upgrades.
pub fn mark_all(env: &Env) {
    env.storage().instance().set(&KEY, &u64::MAX);
}
