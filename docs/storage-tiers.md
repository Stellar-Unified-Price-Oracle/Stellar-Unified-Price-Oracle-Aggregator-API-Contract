# Configurable History Storage Tier (#246)

An asset's price history is written to **exactly one** storage tier. The tier is
chosen by the admin, is discoverable on-chain, and is part of the asset's
*availability guarantee* — not a cost knob.

- Query: `get_asset_storage_tier(asset) -> u32` (`0` = temporary, `1` = persistent).
- Query: `get_storage_tier_info(asset) -> StorageTierInfo`.
- Read: `get_tiered_historical_price(asset, ledger) -> Option<PriceHistoryEntry>`.
- Read: `has_tiered_historical_price(asset, ledger) -> bool`.

```text
StorageTierInfo {
  tier,               // 0 = temporary, 1 = persistent
  retention_ledgers,  // guarantee in ledgers for that tier
  durable,            // false for temporary
  entry_count,        // entries currently retained (length of the ledger index)
  changed_at_ledger,  // last tier change or migration
}
```

The default for every asset — new and pre-existing — is `Temporary`, which is
byte-for-byte the pre-#246 storage behaviour.

## Retention guarantees

| Tier | `durable` | Storage | Retention guarantee | Cost profile |
|---|---|---|---|---|
| `Temporary` (0, default) | `false` | temporary, one entry per `(asset, ledger)` | **≥ 10 000 ledgers** (`TEMPORARY_RETENTION_LEDGERS`). Both writes *and* reads re-bump the entry's TTL to this window, so a consumer that polls at least once inside the window always sees the stored value. | cheap, entry may lapse |
| `Persistent` (1) | `true` | persistent week shard, `HistoryBucket(asset, ledger / 120 960)` | retained until the entry is pruned/archived — no protocol-level bound. Reported as `PERSISTENT_RETENTION_LEDGERS` (6 312 000) purely as a planning figure. | archival, more expensive |

The `durable` flag is the contract with the consumer: when it is `false`, an
absent historical read is **expected behaviour**, not a contract fault.

## TTL expiry yields an explicit absent result — never a fabricated value

This is the single most important consumer-facing rule.

- If a `Temporary` entry's TTL has lapsed, the host evicts it and
  `get_tiered_historical_price` returns `None`.
- `None` means **"there is no guaranteed price at this ledger"**. It is never a
  fabricated, interpolated, carried-forward, or otherwise synthesised number.
- `get_tiered_historical_price` contains no interpolation logic at all. A
  consumer that needs interpolation must opt into it explicitly through the
  legacy `get_historical_price` path and understand that it is a *different*
  guarantee.
- A `None` is never a lookup error to be retried elsewhere: there is nowhere
  else, by design (see the next section).

## No cross-tier fallback — the strict routing rule

Reads are routed by the asset's configured tier and consult **only that tier**.

- Asset on `Temporary` → read `PriceHistory(asset, ledger)`; a value sitting in
  the persistent shard for the same ledger is **invisible**.
- Asset on `Persistent` → search the week bucket; a value sitting in temporary
  storage for the same ledger is **invisible**.

There is deliberately no "try the other tier" fallback. A fallback would make
the tier meaningless as a guarantee: a consumer could believe it is reading a
durably-stored archival price and silently receive a value that was about to be
evicted, or vice-versa. Under this rule a missing price is always *visibly*
missing, so a consumer can never mistake a lapsed cheap entry for an archival
one — or the reverse.

The only place a cross-tier lookup happens is the explicit, admin-authorised
migration (`migrate_history_to_tier`), which is not a read path.

## Writes

Writes follow the same routing. `write_history_entry(asset, entry)` writes to
the asset's configured tier with that tier's TTL treatment:

- `Temporary` → one temporary entry at `PriceHistory(asset, ledger)`, TTL
  extended to the documented window. Exactly the pre-#246 behaviour.
- `Persistent` → appended (or replaced in place) inside the asset's week bucket,
  with the bucket's TTL extended.

`remove_history_entry(asset, ledger)` removes an entry from the configured tier
only, shrinking the week bucket and deleting the bucket key when it empties. It
is a silent no-op when the entry is absent, so it is safe to call
unconditionally from the aggregation/compaction path.

## Changing a tier

### Upgrade — `Temporary` → `Persistent`

Strengthens the guarantee, so it is immediate and single-admin:

```text
admin -> set_asset_storage_tier(asset, Persistent)
```

Emits `StorageTierChangedEvent { action: 0, old_tier: 0, new_tier: 1 }`.
Setting the tier an asset already has is a no-op that still emits
`action: 0`, so every call is observable.

### Downgrade — `Persistent` → `Temporary`

Weakens the guarantee, so it is **never a single-admin action**. It needs both
a distinct second party and a timelock:

1. `admin -> propose_storage_tier_downgrade(asset)` → returns the ledger by which
   all required approvals must be in (`current_ledger + 10`). Emits
   `StorageTierChangedEvent { action: 1 }`. A second proposal while one is
   pending is rejected, as is a proposal for an asset that is not currently
   persistent.
2. `approver -> approve_storage_tier_downgrade(asset, approver)` — the approver
   must authorise the call and **must not be the proposing admin**
   (`NotAuthorized`); the same party cannot approve twice
   (`InvalidConfiguration`). This is the coerced-admin defence: a compromised or
   extorted admin key still cannot downgrade on its own. Once
   `TIER_DOWNGRADE_REQUIRED_APPROVALS` (1) approvals are in, the timelock starts
   and `StorageTierChangedEvent { action: 2 }` is emitted.
3. After the timelock matures, `admin -> execute_storage_tier_downgrade(asset)`.
   Any earlier attempt fails with `StorageTierDowngradeNotReady` (#156) even
   though the request is fully approved. On success
   `StorageTierChangedEvent { action: 3 }` is emitted and the request is
   consumed (so it cannot be replayed).

`set_asset_storage_tier(asset, Temporary)` is also honoured *only* in the one
case where a matured, fully approved request already exists — it is a
convenience alias for step 3, never a bypass.

### A downgrade never erases history

Downgrading changes where **new** entries are written. Already-stored entries
are left exactly where they are and are allowed to expire naturally under their
own TTL. No downgrade path deletes a persistent entry in place, and none removes
the ledger index, so a record of what was once published is never rewritten
retroactively.

Operationally: after a downgrade the asset's older entries are still on disk in
the shard, but the *tier-routed reader* no longer surfaces them (strict routing,
above). If a consumer needs those entries served, run
`migrate_history_to_tier` — or better, do not downgrade.

## Migrating existing history

```text
admin -> migrate_history_to_tier(asset, to_tier) -> u32
```

Walks the ledgers in `PriceHistoryLedgers(asset)`, locates each entry with a
**cross-tier** lookup (temporary entry *or* persistent week-shard entry), and
copies it into `to_tier` with that tier's TTL treatment. Returns the number of
entries copied; emits `StorageTierMigratedEvent`. The source tier is left intact
— migration copies, it never moves. Migrating to the tier the asset already has
is a no-op returning `0`.

This is the supported path for pre-existing assets that need a durable
guarantee without waiting for new history to accumulate.

## Event schema

`StorageTierChangedEvent` (topics: `asset`; data: everything else)

```text
old_tier: u32, new_tier: u32, action: u32, actor: Address, ledger: u32
```

| `action` | Meaning |
|---|---|
| 0 | applied immediately (an upgrade, or a no-op `set`) |
| 1 | downgrade proposed |
| 2 | final approval recorded; timelock started |
| 3 | matured downgrade executed |

`StorageTierMigratedEvent` (topics: `asset`)

```text
from_tier: u32, to_tier: u32, entries_migrated: u32, actor: Address, ledger: u32
```

## Errors

| Code | Name | Raised when |
|---|---|---|
| 156 | `StorageTierDowngradeNotReady` | a downgrade is attempted with no request, an unapproved request, or a timelock that has not matured; also `propose` on a non-persistent asset or with a request already pending |
| 157 | `UnknownStorageTier` | reserved for unrecognised on-chain tier discriminants |
| 158 | `StorageTierMigrationFailed` | reserved for a migration that could not complete |
| 0 | `NotAuthorized` | caller is not the admin, or tried to approve their own downgrade proposal |
| 10 | `InvalidConfiguration` | the same party approved a downgrade twice |
| 2 | `AssetNotRegistered` | the asset is not registered |

## What the tests assert

`contracts/price-oracle/src/storage_tier_tests.rs`:

- the default tier is temporary and the guarantee is discoverable;
- an upgrade applies immediately and emits the documented event shape;
- a downgrade with no approval is rejected with `#156`, and the tier is
  unchanged;
- the full adversarial downgrade sequence (propose → admin self-approve refused
  → execute before maturity refused → distinct party approves → execute one
  ledger early refused → execute at maturity succeeds → replay refused), with
  the tier asserted persistent at every rejected step;
- no silent cross-tier fallback in either direction, plus a check that a missing
  ledger is never fabricated;
- TTL expiry produces an explicit `None` and `false`, while the same entry on a
  persistent asset survives far past the temporary window;
- migration returns the entry count, the entries become readable from the target
  tier, and `StorageTierMigratedEvent` is emitted; migrating to the current tier
  is a no-op; migration is non-destructive;
- an automated retention check that submits real prices and confirms, with no
  manual TTL bumping, that every in-window entry is still returned by the
  tier-routed reader — the same check for the persistent tier is run far past
  the temporary window.
