//! # Configurable history storage tier (#246)
//!
//! See `docs/storage-tiers.md`.
//!
//! An asset's price history is written to exactly one of two storage tiers,
//! selected by the admin and discoverable on-chain via
//! [`get_storage_tier_info`]:
//!
//! | Tier | Storage | Retention |
//! |---|---|---|
//! | [`HistoryStorageTier::Temporary`] | temporary, one entry per `(asset, ledger)` | [`TEMPORARY_RETENTION_LEDGERS`] ledgers, bumped on every read and write |
//! | [`HistoryStorageTier::Persistent`] | persistent week shard [`DataKey::HistoryBucket`] | until explicitly pruned/archived |
//!
//! The default is [`HistoryStorageTier::Temporary`], which preserves the exact
//! pre-#246 storage behaviour for every existing asset.
//!
//! ## The three security properties this module enforces
//!
//! 1. **Strict tier routing, never a cross-tier fallback.**
//!    [`read_history_entry`] resolves the asset's tier and then reads *only*
//!    that tier. It never consults the other tier. If a `Temporary` entry has
//!    expired, the read returns an explicit absent (`None`) — it never returns a
//!    value that happens to be sitting in the other tier, and it never fabricates
//!    or interpolates one. A missing price is therefore always visibly missing,
//!    never silently substituted. Consumers must treat `None` as "no
//!    guaranteed price at this ledger", not as a lookup failure to retry
//!    elsewhere.
//!
//! 2. **A downgrade is never a single-admin action.** Going `Persistent ->
//!    Temporary` weakens the asset's retention guarantee, so it requires a
//!    multi-party + timelock path ([`propose_storage_tier_downgrade`] ->
//!    [`approve_storage_tier_downgrade`] by a *distinct* party ->
//!    [`execute_storage_tier_downgrade`] after the delay). The proposing admin
//!    cannot approve their own request, and [`set_asset_storage_tier`] refuses
//!    a downgrade outright unless a matured request already exists.
//!
//! 3. **A downgrade never erases history.** Downgrading changes where *new*
//!    entries are written; already-stored entries are left untouched and
//!    allowed to expire naturally under their own TTL. No downgrade path
//!    deletes a persistent entry in place, and none removes the ledger index, so
//!    an audit of what was once published is never rewritten retroactively. Use
//!    [`migrate_history_to_tier`] to *copy* entries forward deliberately.

use soroban_sdk::{panic_with_error, Address, Env, Vec};

use crate::events::{StorageTierChangedEvent, StorageTierMigratedEvent};
use crate::history::{write_history_shard, LEDGERS_PER_WEEK};
use crate::storage::{check_registered_asset, get_admin, LEDGER_BUMP, LEDGER_THRESHOLD};
use crate::types::{
    DataKey, ErrorCode, HistoryStorageTier, PriceHistoryEntry, StorageTierDowngradeRequest,
    StorageTierInfo, TEMPORARY_RETENTION_LEDGERS, TIER_DOWNGRADE_DELAY_LEDGERS,
    TIER_DOWNGRADE_REQUIRED_APPROVALS,
};

/// `action` value emitted when a tier change is applied immediately (an
/// upgrade), and also for a `set` call that changes nothing.
pub const ACTION_APPLIED: u32 = 0;
/// `action` value emitted when a downgrade is proposed.
pub const ACTION_PROPOSED: u32 = 1;
/// `action` value emitted when the last required approval lands and the timelock
/// starts.
pub const ACTION_APPROVED: u32 = 2;
/// `action` value emitted when a matured downgrade is executed.
pub const ACTION_EXECUTED: u32 = 3;

/// TTL window applied to a `Temporary`-tier history entry on write and on read.
///
/// The `min()` keeps us within the per-transaction extend ceiling used
/// everywhere else in this contract.
#[inline]
fn temporary_ttl() -> u32 {
    TEMPORARY_RETENTION_LEDGERS.min(LEDGER_THRESHOLD)
}

/// Week-bucket index a ledger falls into.
#[inline]
fn ledger_to_bucket(ledger: u32) -> u32 {
    ledger / LEDGERS_PER_WEEK
}

// ─────────────────────────────────────────────────────────────────────────────
// Tier resolution
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the tier an asset's price history is written to. Default: temporary.
pub fn get_asset_storage_tier(env: &Env, asset: &Address) -> HistoryStorageTier {
    let key = DataKey::AssetStorageTier(asset.clone());
    match env
        .storage()
        .persistent()
        .get::<_, HistoryStorageTier>(&key)
    {
        Some(tier) => {
            env.storage()
                .persistent()
                .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
            tier
        }
        None => HistoryStorageTier::Temporary,
    }
}

/// Returns the discoverable retention guarantee for an asset's history tier.
///
/// A consumer reads this *before* trusting a historical price and knows exactly
/// what availability is promised: `durable == false` means entries may
/// legitimately vanish with their TTL, so an absent read is expected rather than
/// an error.
pub fn get_storage_tier_info(env: &Env, asset: &Address) -> StorageTierInfo {
    let tier = get_asset_storage_tier(env, asset);

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let entry_count: u32 = env
        .storage()
        .persistent()
        .get::<_, Vec<u32>>(&ledgers_key)
        .map(|v| v.len())
        .unwrap_or(0);

    StorageTierInfo {
        tier,
        retention_ledgers: tier.retention_ledgers(),
        durable: tier == HistoryStorageTier::Persistent,
        entry_count,
        changed_at_ledger: read_last_change_ledger(env, asset),
    }
}

/// Ledger of the last completed tier change *or* history migration.
///
/// Stored in [`DataKey::AssetStorageTierMigrated`]: that slot is documented as
/// the "last-change" ledger and is the single source of truth for both a plain
/// tier switch and a migration.
fn read_last_change_ledger(env: &Env, asset: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::AssetStorageTierMigrated(asset.clone()))
        .unwrap_or(0)
}

fn write_tier(env: &Env, asset: &Address, tier: HistoryStorageTier) {
    let key = DataKey::AssetStorageTier(asset.clone());
    env.storage().persistent().set(&key, &tier);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn write_last_change_ledger(env: &Env, asset: &Address, ledger: u32) {
    let key = DataKey::AssetStorageTierMigrated(asset.clone());
    env.storage().persistent().set(&key, &ledger);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn emit_changed(
    env: &Env,
    asset: &Address,
    old: HistoryStorageTier,
    new: HistoryStorageTier,
    action: u32,
    actor: &Address,
) {
    StorageTierChangedEvent {
        asset: asset.clone(),
        old_tier: old.as_u32(),
        new_tier: new.as_u32(),
        action,
        actor: actor.clone(),
        ledger: env.ledger().sequence(),
    }
    .publish(env);
}

// ─────────────────────────────────────────────────────────────────────────────
// Tier change: upgrade is immediate, downgrade is multi-party + timelocked
// ─────────────────────────────────────────────────────────────────────────────

/// Admin endpoint that changes an asset's history storage tier.
///
/// An *upgrade* (`Temporary -> Persistent`, i.e. `current < tier`) strengthens
/// the retention guarantee and applies immediately.
///
/// A *downgrade* (`current > tier`) weakens it and is therefore **never** a
/// single-admin action: it is applied here only if a downgrade request already
/// exists that is fully approved and whose `ready_at_ledger` has matured. Use
/// the explicit propose/approve/execute flow instead.
///
/// Calling this with the asset's current tier is a no-op that still emits
/// [`StorageTierChangedEvent`] with `action = 0`.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::StorageTierDowngradeNotReady`] — a downgrade was requested
///   without a matured, fully approved downgrade request.
pub fn set_asset_storage_tier(env: &Env, asset: Address, tier: HistoryStorageTier) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let current = get_asset_storage_tier(env, &asset);

    // No-op: nothing changes, but the call is still observable.
    if current == tier {
        emit_changed(env, &asset, current, tier, ACTION_APPLIED, &admin);
        return;
    }

    if tier < current {
        // Downgrade. Only honour it if a fully approved, matured request exists.
        let req = read_request(env, &asset)
            .unwrap_or_else(|| panic_with_error!(env, ErrorCode::StorageTierDowngradeNotReady));
        if req.ready_at_ledger == 0 || req.ready_at_ledger > env.ledger().sequence() {
            panic_with_error!(env, ErrorCode::StorageTierDowngradeNotReady);
        }
        apply_tier_change(env, &asset, tier);
        remove_request(env, &asset);
        emit_changed(env, &asset, current, tier, ACTION_EXECUTED, &admin);
        return;
    }

    // Upgrade: applies immediately.
    apply_tier_change(env, &asset, tier);
    emit_changed(env, &asset, current, tier, ACTION_APPLIED, &admin);
}

/// Applies a tier change and stamps the last-change ledger.
///
/// Deliberately touches **no** history entries: switching tiers changes where
/// *new* entries are written and lets already-stored entries expire naturally.
/// Nothing is ever deleted in place.
fn apply_tier_change(env: &Env, asset: &Address, new: HistoryStorageTier) {
    write_tier(env, asset, new);
    write_last_change_ledger(env, asset, env.ledger().sequence());
}

fn request_key(asset: &Address) -> DataKey {
    DataKey::AssetStorageTierDowngrade(asset.clone())
}

fn read_request(env: &Env, asset: &Address) -> Option<StorageTierDowngradeRequest> {
    let key = request_key(asset);
    let req: Option<StorageTierDowngradeRequest> = env.storage().persistent().get(&key);
    if req.is_some() {
        env.storage()
            .persistent()
            .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
    req
}

fn write_request(env: &Env, asset: &Address, req: &StorageTierDowngradeRequest) {
    let key = request_key(asset);
    env.storage().persistent().set(&key, req);
    env.storage()
        .persistent()
        .extend_ttl(&key, LEDGER_THRESHOLD, LEDGER_BUMP);
}

fn remove_request(env: &Env, asset: &Address) {
    env.storage().persistent().remove(&request_key(asset));
}

/// Proposes a persistent -> temporary downgrade for an asset's history tier.
///
/// Admin only. The asset must be registered and currently `Persistent`, and no
/// other request may be pending. Returns the ledger by which all required
/// approvals must be in for the timelock to start.
///
/// The proposing admin is recorded as `proposed_by` and is **barred** from
/// approving: a single admin can never downgrade on their own, so a coerced or
/// compromised admin key cannot silently weaken an asset's retention guarantee.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::StorageTierDowngradeNotReady`] — the asset is not currently
///   `Persistent`, or a request is already pending.
pub fn propose_storage_tier_downgrade(env: &Env, asset: Address) -> u32 {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let current = get_asset_storage_tier(env, &asset);
    if current != HistoryStorageTier::Persistent || read_request(env, &asset).is_some() {
        panic_with_error!(env, ErrorCode::StorageTierDowngradeNotReady);
    }

    write_request(
        env,
        &asset,
        &StorageTierDowngradeRequest {
            proposed_by: admin.clone(),
            approvals: Vec::new(env),
            ready_at_ledger: 0,
            from_tier: current,
        },
    );

    emit_changed(
        env,
        &asset,
        current,
        HistoryStorageTier::Temporary,
        ACTION_PROPOSED,
        &admin,
    );

    env.ledger()
        .sequence()
        .saturating_add(TIER_DOWNGRADE_DELAY_LEDGERS)
}

/// Records a second party's approval of a pending downgrade request.
///
/// `approver` must authorize the call and must be a **distinct** party from the
/// proposing admin. Once [`TIER_DOWNGRADE_REQUIRED_APPROVALS`] approvals have
/// been collected the timelock starts: `ready_at_ledger` is set to
/// `current_ledger + TIER_DOWNGRADE_DELAY_LEDGERS` and
/// [`StorageTierChangedEvent`] is emitted with `action = 2`.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — no pending request, or `approver` is the
///   proposing admin.
/// * [`ErrorCode::InvalidConfiguration`] — `approver` already approved.
pub fn approve_storage_tier_downgrade(env: &Env, asset: Address, approver: Address) {
    approver.require_auth();
    check_registered_asset(env, &asset);

    let mut req = read_request(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::StorageTierDowngradeNotReady));

    // Coerced-admin defence: the proposer can never count as the second party.
    if approver == req.proposed_by {
        panic_with_error!(env, ErrorCode::NotAuthorized);
    }

    for i in 0..req.approvals.len() {
        if req.approvals.get_unchecked(i) == approver {
            panic_with_error!(env, ErrorCode::InvalidConfiguration);
        }
    }

    req.approvals.push_back(approver.clone());
    if req.approvals.len() >= TIER_DOWNGRADE_REQUIRED_APPROVALS {
        req.ready_at_ledger = env
            .ledger()
            .sequence()
            .saturating_add(TIER_DOWNGRADE_DELAY_LEDGERS);
    }
    let ready_at = req.ready_at_ledger;
    write_request(env, &asset, &req);

    if ready_at > 0 {
        emit_changed(
            env,
            &asset,
            HistoryStorageTier::Persistent,
            HistoryStorageTier::Temporary,
            ACTION_APPROVED,
            &approver,
        );
    }
}

/// Executes a fully approved, matured downgrade request.
///
/// Admin only, and only once the timelock has run out. Applies the tier change
/// and removes the request. **No history entry is deleted**: already-stored
/// persistent entries survive and simply stop receiving new writes, while new
/// entries are written to the cheaper temporary tier. Consumers that need the
/// existing entries preserved durably should call [`migrate_history_to_tier`]
/// first.
///
/// # Errors
/// * [`ErrorCode::NotAuthorized`] — caller is not the admin.
/// * [`ErrorCode::AssetNotRegistered`] — `asset` is not registered.
/// * [`ErrorCode::StorageTierDowngradeNotReady`] — no request, not fully
///   approved, or the timelock has not matured.
pub fn execute_storage_tier_downgrade(env: &Env, asset: Address) {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let req = read_request(env, &asset)
        .unwrap_or_else(|| panic_with_error!(env, ErrorCode::StorageTierDowngradeNotReady));
    if req.ready_at_ledger == 0 || req.ready_at_ledger > env.ledger().sequence() {
        panic_with_error!(env, ErrorCode::StorageTierDowngradeNotReady);
    }

    apply_tier_change(env, &asset, HistoryStorageTier::Temporary);
    remove_request(env, &asset);
    emit_changed(
        env,
        &asset,
        req.from_tier,
        HistoryStorageTier::Temporary,
        ACTION_EXECUTED,
        &admin,
    );
}

/// Copies an asset's retained history entries into `to_tier`, returning the
/// number of entries copied.
///
/// Admin only. Entries are located with a *cross-tier* lookup (temporary entry
/// **or** persistent week-shard entry) so the migration works from whichever
/// tier the data currently lives in, then written into `to_tier`. Nothing is
/// removed from the source tier.
///
/// Returns `0` and does nothing when `to_tier` is already the asset's tier.
pub fn migrate_history_to_tier(env: &Env, asset: Address, to_tier: HistoryStorageTier) -> u32 {
    let admin = get_admin(env);
    admin.require_auth();
    check_registered_asset(env, &asset);

    let from_tier = get_asset_storage_tier(env, &asset);
    if from_tier == to_tier {
        return 0;
    }

    let ledgers_key = DataKey::PriceHistoryLedgers(asset.clone());
    let ledgers: Vec<u32> = env
        .storage()
        .persistent()
        .get(&ledgers_key)
        .unwrap_or(Vec::new(env));

    let mut migrated: u32 = 0;
    for i in 0..ledgers.len() {
        let ledger = ledgers.get_unchecked(i);
        if let Some(entry) = read_entry_cross_tier(env, &asset, ledger) {
            write_entry_to_tier(env, &asset, &entry, to_tier);
            migrated = migrated.saturating_add(1);
        }
    }

    write_last_change_ledger(env, &asset, env.ledger().sequence());

    StorageTierMigratedEvent {
        asset: asset.clone(),
        from_tier: from_tier.as_u32(),
        to_tier: to_tier.as_u32(),
        entries_migrated: migrated,
        actor: admin,
        ledger: env.ledger().sequence(),
    }
    .publish(env);

    migrated
}

// ─────────────────────────────────────────────────────────────────────────────
// Strict tier-routed read / write / remove
// ─────────────────────────────────────────────────────────────────────────────

/// Strict tier-routed history read. Never falls back across tiers.
///
/// Resolves the asset's configured tier and reads **only** that tier:
///
/// * `Temporary` — `DataKey::PriceHistory(asset, ledger)`, TTL-bumped on a hit.
/// * `Persistent` — the asset's week bucket `DataKey::HistoryBucket`, TTL-bumped
///   on a hit.
///
/// Returns `None` when the entry is absent from the asset's tier. In particular
/// an expired `Temporary` entry yields `None`; the other tier is **not**
/// consulted, and no value is fabricated or interpolated.
pub fn read_history_entry(env: &Env, asset: &Address, ledger: u32) -> Option<PriceHistoryEntry> {
    match get_asset_storage_tier(env, asset) {
        HistoryStorageTier::Temporary => {
            let key = DataKey::PriceHistory(asset.clone(), ledger);
            let entry: Option<PriceHistoryEntry> = env.storage().temporary().get(&key);
            if entry.is_some() {
                env.storage()
                    .temporary()
                    .extend_ttl(&key, temporary_ttl(), LEDGER_BUMP);
            }
            entry
        }
        HistoryStorageTier::Persistent => {
            let bucket_key = DataKey::HistoryBucket(asset.clone(), ledger_to_bucket(ledger));
            let bucket: Option<Vec<PriceHistoryEntry>> =
                env.storage().persistent().get(&bucket_key);
            let bucket = bucket?;
            for i in 0..bucket.len() {
                let entry = bucket.get_unchecked(i);
                if entry.ledger == ledger {
                    env.storage().persistent().extend_ttl(
                        &bucket_key,
                        LEDGER_THRESHOLD,
                        LEDGER_BUMP,
                    );
                    return Some(entry);
                }
            }
            None
        }
    }
}

/// Tier-routed history write used by the aggregation path.
///
/// Writes the entry to the asset's configured tier with that tier's TTL
/// treatment. For the default `Temporary` tier this is exactly the pre-#246
/// behaviour: a single `DataKey::PriceHistory(asset, ledger)` entry in temporary
/// storage, TTL-extended to the documented retention window. Nothing else is
/// touched.
pub fn write_history_entry(env: &Env, asset: &Address, entry: &PriceHistoryEntry) {
    write_entry_to_tier(env, asset, entry, get_asset_storage_tier(env, asset));
}

/// Tier-routed removal of a single history entry.
///
/// Removes the entry from the asset's *configured* tier only: the temporary
/// `DataKey::PriceHistory(asset, ledger)` key, or the matching
/// `PriceHistoryEntry` inside the week-bucket vector at
/// `DataKey::HistoryBucket(asset, ledger / LEDGERS_PER_WEEK)`. The vector is
/// shrunk and an emptied bucket key is removed rather than left behind.
///
/// Silently does nothing when the entry is absent — it never panics, so the
/// aggregation path's `should_skip_on_write` cleanup is safe to call
/// unconditionally.
pub fn remove_history_entry(env: &Env, asset: &Address, ledger: u32) {
    match get_asset_storage_tier(env, asset) {
        HistoryStorageTier::Temporary => {
            env.storage()
                .temporary()
                .remove(&DataKey::PriceHistory(asset.clone(), ledger));
        }
        HistoryStorageTier::Persistent => {
            remove_bucket_entry(env, asset, ledger);
        }
    }
}

fn write_entry_to_tier(
    env: &Env,
    asset: &Address,
    entry: &PriceHistoryEntry,
    tier: HistoryStorageTier,
) {
    match tier {
        HistoryStorageTier::Temporary => {
            let key = DataKey::PriceHistory(asset.clone(), entry.ledger);
            env.storage().temporary().set(&key, entry);
            env.storage()
                .temporary()
                .extend_ttl(&key, temporary_ttl(), LEDGER_BUMP);
        }
        HistoryStorageTier::Persistent => {
            // Appends (or replaces in place) the entry inside the asset's week
            // bucket and TTL-bumps the bucket key.
            write_history_shard(env, asset, entry);
        }
    }
}

/// Cross-tier lookup used only by [`migrate_history_to_tier`].
///
/// Checks the temporary entry first, then the persistent week shard. This is
/// deliberately *not* used by [`read_history_entry`], which must never cross
/// tiers on a consumer-facing read.
fn read_entry_cross_tier(env: &Env, asset: &Address, ledger: u32) -> Option<PriceHistoryEntry> {
    let key = DataKey::PriceHistory(asset.clone(), ledger);
    if let Some(entry) = env.storage().temporary().get::<_, PriceHistoryEntry>(&key) {
        return Some(entry);
    }

    let bucket_key = DataKey::HistoryBucket(asset.clone(), ledger_to_bucket(ledger));
    let bucket: Option<Vec<PriceHistoryEntry>> = env.storage().persistent().get(&bucket_key);
    let bucket = bucket?;
    for i in 0..bucket.len() {
        let entry = bucket.get_unchecked(i);
        if entry.ledger == ledger {
            return Some(entry);
        }
    }
    None
}

/// Drops `ledger`'s entry from its week bucket, removing the bucket key when the
/// bucket becomes empty.
fn remove_bucket_entry(env: &Env, asset: &Address, ledger: u32) {
    let bucket_key = DataKey::HistoryBucket(asset.clone(), ledger_to_bucket(ledger));
    let bucket: Vec<PriceHistoryEntry> = match env.storage().persistent().get(&bucket_key) {
        Some(b) => b,
        None => return,
    };

    let mut retained: Vec<PriceHistoryEntry> = Vec::new(env);
    for i in 0..bucket.len() {
        let entry = bucket.get_unchecked(i);
        if entry.ledger != ledger {
            retained.push_back(entry);
        }
    }

    if retained.len() == bucket.len() {
        // Nothing matched — leave the bucket untouched.
        return;
    }

    if retained.is_empty() {
        env.storage().persistent().remove(&bucket_key);
    } else {
        env.storage().persistent().set(&bucket_key, &retained);
        env.storage()
            .persistent()
            .extend_ttl(&bucket_key, LEDGER_THRESHOLD, LEDGER_BUMP);
    }
}
