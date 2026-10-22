# Asset Lifecycle: Disabled, Delisted, and Replaced Assets

## Overview

This document specifies consumer behavior and history retention when an asset is disabled, delisted, or replaced. It makes the data transparency and quality decision explicit, reviewable, and verifiable.

## Terminology

- **Active**: An asset is in normal operation. New prices can be submitted and aggregated. Consumers can query current and historical prices.

- **Disabled**: An asset has been temporarily taken offline by the admin. New price submissions and aggregations are blocked. Consumers **cannot** query current aggregated prices (returns error). Historical prices **remain discoverable** via `get_historical_price()` and paginated queries for audit and reconciliation.

- **Delisted**: An asset has been permanently removed from the oracle registry (via `unregister_asset()`). New submissions and aggregations are blocked. Consumers **cannot** query current prices. Historical prices **remain in storage** indefinitely unless pruned by admin policy.

- **Replaced**: An asset is being migrated to a new asset address (e.g., token upgrade, bridge consolidation). The old asset is disabled, and a new asset is registered. Consumers must migrate to the new asset. Historical prices on the old asset remain queryable.

## Asset Status States

Assets transition through these states:

```
Active → Disabled → (Delisted OR Re-enabled)
Active → Deleted (via unregister_asset)
```

New states stored per asset:

```rust
pub enum AssetStatus {
    Active,       // Can submit, aggregate, query
    Disabled,     // Cannot submit/aggregate, can query history only
}
```

## Storage Model

### New Storage Keys

```rust
// Disable/enable state for an asset
AssetStatus(Address) -> AssetStatus

// Timestamp when an asset was disabled (audit trail)
AssetDisabledAt(Address) -> u64 (ledger sequence)

// Reason code for disabling (optional, for audit)
AssetDisableReason(Address) -> u32
```

### Historical Records Retained

- **Active → Disabled**: All `PriceHistory(asset, ledger)` entries remain in storage. The asset is marked as disabled in its status.
- **Active → Delisted** (via `unregister_asset`): Asset is removed from the registry, but all `PriceHistory` entries remain. Queries fail with `AssetNotRegistered` for current prices, but history queries still work.

## Consumer Behavior

### Current Price Queries (`get_price`, `get_source_price`, `lastprice`)

| State | Behavior |
|-------|----------|
| Active | Returns current aggregated price |
| Disabled | Returns `ErrorCode::AssetDisabled` |
| Delisted | Returns `ErrorCode::AssetNotRegistered` |

### Historical Price Queries (`get_historical_price`, `get_historical_prices_paginated`)

| State | Behavior |
|-------|----------|
| Active | Returns historical price at ledger |
| Disabled | Returns historical price at ledger (history preserved) |
| Delisted | Returns historical price at ledger (history preserved) |

**Rationale**: Historical queries remain available even for disabled/delisted assets to support:
- Audit trails and compliance
- Consumer contract reconciliation
- Price derivation and backtesting
- Incident investigation

### Asset Registry Queries (`assets`, `assets_paginated`)

| State | Behavior |
|-------|----------|
| Active | Included in registry listing |
| Disabled | Still included in registry (status marked as disabled) |
| Delisted | Removed from registry |

**Rationale**: Disabled assets should remain discoverable so consumers can identify why a query failed. Delisted assets are completely removed.

### Price Submission (`submit_price`)

| State | Behavior |
|-------|----------|
| Active | Accepted and aggregated |
| Disabled | Returns `ErrorCode::AssetDisabled` |
| Delisted | Returns `ErrorCode::AssetNotRegistered` |

### Admin Operations

| Operation | Behavior |
|-----------|----------|
| `disable_asset(asset)` | Marks asset as disabled (preserves history, allows re-enable) |
| `enable_asset(asset)` | Re-enables a disabled asset to Active state |
| `unregister_asset(asset)` | Removes asset from registry (history still queryable) |

## Failure and Recovery Paths

### Temporary Outage (Disable → Enable)

1. Admin calls `disable_asset(asset)` when a data quality issue is detected.
2. All submissions blocked; consumers get `AssetDisabled` error.
3. Consumers can still query historical prices to reconcile positions.
4. Admin investigates and fixes the issue.
5. Admin calls `enable_asset(asset)` to resume normal operation.
6. New submissions are accepted; aggregation resumes.

### Permanent Delisting (Unregister)

1. Admin calls `unregister_asset(asset)` when an asset is permanently removed.
2. Asset removed from the registry; `get_price()` returns `AssetNotRegistered`.
3. Historical prices remain queryable via `get_historical_price()` for audit.
4. Consumers migrate to a replacement asset (if available).

### Asset Migration (Old → New)

1. Admin registers a new asset address.
2. Admin disables the old asset with disable reason "Asset replaced by NEW_ADDRESS".
3. Sources start submitting prices to the new asset.
4. Consumers read the disable reason and migrate their queries.
5. Historical prices on the old asset remain queryable.
6. After a transition period, admin may unregister the old asset.

## Error Codes

| Code | Name | Trigger | Resolution |
|------|------|---------|-----------|
| 28 | `AssetDisabled` | Submit price or query aggregated price for disabled asset | Re-enable the asset or switch to enabled asset |
| 2 | `AssetNotRegistered` | Query or submit for unregistered asset | Register the asset or use an active asset |

## Query Response Format (SEP-40 Extension)

The `assets()` and `assets_paginated()` endpoints continue to return `Asset::Stellar(Address)` to maintain SEP-40 compliance. Status information can be queried separately via `get_asset_status(asset) -> (AssetStatus, Option<DisableReason>)`.

**Rationale**: SEP-40 does not define a status field in the asset list response. Consumers that need status must call a separate query.

## Test Coverage

### Normal Operation
- [x] Submit price to active asset
- [x] Query current price for active asset
- [x] Query historical price for active asset
- [x] List active assets in registry

### Disable/Enable Flow
- [x] Disable an active asset
- [x] Cannot submit price to disabled asset (returns `AssetDisabled`)
- [x] Cannot query aggregated price for disabled asset (returns `AssetDisabled`)
- [x] **Can** query historical prices for disabled asset
- [x] Re-enable a disabled asset
- [x] Resume submitting and querying after re-enable

### Delisting Flow
- [x] Unregister an asset
- [x] Cannot submit price to unregistered asset (returns `AssetNotRegistered`)
- [x] Cannot query aggregated price for unregistered asset (returns `AssetNotRegistered`)
- [x] **Can** query historical prices for unregistered asset
- [x] Asset removed from registry listing

### Boundary Conditions
- [x] Multiple assets with different states
- [x] Pagination cursor on mix of active/disabled/delisted assets
- [x] History queries at ledger boundaries
- [x] Concurrent enable/disable operations

## Documentation Updates

1. **README.md**: Add explanation of asset lifecycle and disabled/delisted states.
2. **Operator Runbook**: Add disable/enable/unregister procedures and troubleshooting.
3. **Error Registry**: Add `AssetDisabled` error code explanation and resolution.
4. **SEP-40 Conformance**: Clarify that disabled/delisted assets do not appear in `assets()` (active only).
5. **Consumer Integration Guide**: Explain handling of `AssetDisabled` and `AssetNotRegistered` errors.

## Backward Compatibility

- Existing contracts querying active assets are unaffected.
- New `AssetStatus` field is opt-in; consumers that don't check status get an error on disabled assets (similar to existing behavior).
- Historical queries remain unaffected.

## Implementation Checklist

- [x] Add `AssetStatus` enum and storage keys to `types.rs`
- [x] Implement `disable_asset()` and `enable_asset()` endpoints in `admin.rs`
- [x] Update `get_price()`, `get_source_price()`, `submit_price()` to check asset status
- [x] Update `assets_paginated()` to filter out disabled assets (or mark them)
- [x] Implement `get_asset_status()` query endpoint
- [x] Write comprehensive tests
- [x] Update documentation
- [x] Verify all builds and tests pass
