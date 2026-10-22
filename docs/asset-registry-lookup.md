# Asset registry lookup: Vec scan → O(1) membership index

## Current behavior (backward compatible)
- `assets()` continues to return the ordered list from the legacy `DataKey::AssetRegistry` (a `Vec<Address>`).
- `is_asset_registered` / `check_registered_asset` now use a new persistent membership index:
  - `DataKey::AssetRegistryIndex(Address) -> bool`

## Implementation
- On new `register_asset`, we write:
  - `DataKey::AssetRegistered(asset) -> true` (legacy)
  - `DataKey::AssetRegistryIndex(asset) -> true` (new)
  - and we append to `DataKey::AssetRegistry` (for enumeration)

- On `unregister_asset`, we remove both flags and the aggregate price.

## Backward compatibility / lazy migration
Older deployed instances might only have the legacy `DataKey::AssetRegistered(asset)` flag and the `DataKey::AssetRegistry` vec.

When an asset is queried via the membership check:
- If `AssetRegistryIndex(asset)` is missing but `AssetRegistered(asset)` exists, the contract:
  - treats the asset as registered
  - lazily writes `AssetRegistryIndex(asset) -> true`

This ensures lookups become O(1) after the first query per asset without requiring an admin migration transaction.

## Complexity
- Legacy: membership check was effectively O(n) if it required scanning the vec (worst-case).
- New: membership check is O(1) persistent lookup.
- Enumeration remains O(n) because it must return a vec.

## Gas and resource tradeoff
- Extra storage writes per asset registration/unregistration:
  - one additional persistent entry (`AssetRegistryIndex(asset)`) and TTL extension.
- Extra memory in execution is minimal: it performs a single map-like lookup instead of a vec scan.

Net effect:
- **Lookup gas decreases for large registries (50+ assets and beyond).**
- **Registration/unregistration becomes slightly more expensive** due to the added index entry.

## Benchmarking
A local micro-benchmark is provided in:
- `contracts/price-oracle/src/asset_registry_gas_tests.rs`

It measures CPU instructions + memory bytes for `is_asset_registered` after registering 50 assets.

