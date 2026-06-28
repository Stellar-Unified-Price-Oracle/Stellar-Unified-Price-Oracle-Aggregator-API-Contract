# Error Code Registry

All contract errors are defined in `contracts/price-oracle/src/errors.rs` as the `ErrorCode` enum.
Each variant is a `u32` discriminant embedded in the Soroban host error returned to the caller.

## Quick Reference

| Code | Name | HTTP-like analogy |
|------|------|-------------------|
| 0 | `NotAuthorized` | 401 Unauthorized |
| 1 | `AlreadyInitialized` | 409 Conflict |
| 2 | `AssetNotRegistered` | 404 Not Found |
| 3 | `AssetAlreadyRegistered` | 409 Conflict |
| 4 | `SourceAlreadyExists` | 409 Conflict |
| 5 | `SourceNotFound` | 404 Not Found |
| 6 | `InsufficientSources` | 422 Unprocessable Entity |
| 7 | `InvalidPrice` | 400 Bad Request |
| 8 | `NoData` | 404 Not Found |
| 9 | `InvalidTimestamp` | 400 Bad Request |
| 10 | `InvalidConfiguration` | 400 Bad Request |
| 11 | `DescriptionTooLong` | 400 Bad Request |
| 12 | `ContractPaused` | 503 Service Unavailable |
| 13 | `TimelockNotReady` | 425 Too Early |
| 14 | `OperationNotFound` | 404 Not Found |
| 15 | `PriceBelowMinimum` | 400 Bad Request |

---

## Detailed Registry

### Code 0 — `NotAuthorized`

| Field | Value |
|-------|-------|
| **Code** | `0` |
| **Enum variant** | `ErrorCode::NotAuthorized` |

**Description**  
The caller is not authorized to perform the requested operation.

**Cause**  
- Calling an admin-only function from a non-admin address.
- Calling `submit_price` from an address that is not a registered oracle source.
- `require_auth()` check failed for the expected signer.

**Resolution**  
Ensure the transaction is signed by the contract admin (for admin functions) or a registered
oracle source (for price submissions). Use `get_admin_address()` to verify the current admin.

**Example**
```
Error(Contract, #0)
```

---

### Code 1 — `AlreadyInitialized`

| Field | Value |
|-------|-------|
| **Code** | `1` |
| **Enum variant** | `ErrorCode::AlreadyInitialized` |

**Description**  
`initialize` was called on a contract that has already been set up.

**Cause**  
`initialize()` may only be called once after deployment. A second call always panics with
this error regardless of the parameters supplied.

**Resolution**  
Do not call `initialize()` again. Use the appropriate setter functions (e.g. `set_admin`,
`set_decimals`) to change configuration after deployment.

**Example**
```
Error(Contract, #1)
```

---

### Code 2 — `AssetNotRegistered`

| Field | Value |
|-------|-------|
| **Code** | `2` |
| **Enum variant** | `ErrorCode::AssetNotRegistered` |

**Description**  
The specified asset has not been registered with `register_asset`.

**Cause**  
Any function that operates on an asset (e.g. `submit_price`, `get_price`,
`get_historical_price`) first verifies the asset is registered. If the address has never
been registered (or was later unregistered) this error is returned.

**Resolution**  
Call `register_asset(asset)` from the admin account before submitting or querying prices for
that asset. Use `is_asset_registered(asset)` to check registration status.

**Example**
```
Error(Contract, #2)
```

---

### Code 3 — `AssetAlreadyRegistered`

| Field | Value |
|-------|-------|
| **Code** | `3` |
| **Enum variant** | `ErrorCode::AssetAlreadyRegistered` |

**Description**  
`register_asset` was called for an asset that is already registered.

**Cause**  
Each asset address may only be registered once. Attempting a second `register_asset` call
for the same address triggers this error.

**Resolution**  
Use `is_asset_registered(asset)` to check before calling `register_asset`. To re-register
after removal, call `unregister_asset` first.

**Example**
```
Error(Contract, #3)
```

---

### Code 4 — `SourceAlreadyExists`

| Field | Value |
|-------|-------|
| **Code** | `4` |
| **Enum variant** | `ErrorCode::SourceAlreadyExists` |

**Description**  
`add_source` was called for an address that is already a registered oracle source.

**Cause**  
Each source address may only be added once. Attempting a second `add_source` call for the
same address triggers this error.

**Resolution**  
Use `is_source(address)` to check before calling `add_source`. To update a source's name,
remove and re-add it.

**Example**
```
Error(Contract, #4)
```

---

### Code 5 — `SourceNotFound`

| Field | Value |
|-------|-------|
| **Code** | `5` |
| **Enum variant** | `ErrorCode::SourceNotFound` |

**Description**  
The referenced oracle source address is not registered.

**Cause**  
- Calling `remove_source` for an address that was never added.
- Calling `submit_heartbeat` for an unregistered source.
- Any internal check that expects the source to be present.

**Resolution**  
Use `is_source(address)` to verify the source exists. Register it first with `add_source`.

**Example**
```
Error(Contract, #5)
```

---

### Code 6 — `InsufficientSources`

| Field | Value |
|-------|-------|
| **Code** | `6` |
| **Enum variant** | `ErrorCode::InsufficientSources` |

**Description**  
Fewer sources have submitted prices than the configured `min_sources_required`.

**Cause**  
`get_price` (and SEP-40 query methods) aggregate prices from all contributing sources. If the
number of sources with a valid, recent submission is below the minimum threshold, aggregation
is blocked.

**Resolution**  
- Ensure enough oracle sources have submitted prices for the asset.
- Temporarily lower `min_sources_required` if sources are unavailable.
- Check source activity with `get_oracle_sources()`.

**Example**
```
Error(Contract, #6)
```

---

### Code 7 — `InvalidPrice`

| Field | Value |
|-------|-------|
| **Code** | `7` |
| **Enum variant** | `ErrorCode::InvalidPrice` |

**Description**  
A submitted price value is zero or negative.

**Cause**  
`submit_price` rejects any price `<= 0`. All prices must be positive integers scaled by
`10^decimals`.

**Resolution**  
Submit a strictly positive price. Remember that prices are integers — e.g. for 18 decimals,
`1.0` is represented as `1_000_000_000_000_000_000`.

**Example**
```
Error(Contract, #7)
```

---

### Code 8 — `NoData`

| Field | Value |
|-------|-------|
| **Code** | `8` |
| **Enum variant** | `ErrorCode::NoData` |

**Description**  
No aggregate price data exists for the requested asset, or no history entry exists at the
requested ledger (when interpolation is disabled).

**Cause**  
- `get_price` called before any source has submitted a price for the asset.
- `get_historical_price` called for a ledger with no snapshot and interpolation disabled.
- `get_historical_prices` range exceeds `max_history_length`.

**Resolution**  
- Submit at least `min_sources_required` prices for the asset.
- Enable interpolation (`set_interpolation_enabled(true)`) to fill gaps in history.
- Narrow the ledger range when using `get_historical_prices`.

**Example**
```
Error(Contract, #8)
```

---

### Code 9 — `InvalidTimestamp`

| Field | Value |
|-------|-------|
| **Code** | `9` |
| **Enum variant** | `ErrorCode::InvalidTimestamp` |

**Description**  
The submitted timestamp lies too far in the future relative to the current ledger time.

**Cause**  
`submit_price` checks `timestamp <= ledger_time + timestamp_threshold`. If the supplied
timestamp exceeds this bound the submission is rejected. Default threshold is 300 s (5 min).

**Resolution**  
- Use the current Unix timestamp (seconds) when submitting prices.
- If your clock is ahead of Stellar's ledger clock, adjust accordingly.
- Admins can widen the window via `set_timestamp_threshold`.

**Example**
```
Error(Contract, #9)
```

---

### Code 10 — `InvalidConfiguration`

| Field | Value |
|-------|-------|
| **Code** | `10` |
| **Enum variant** | `ErrorCode::InvalidConfiguration` |

**Description**  
A configuration parameter is out of its valid range.

**Cause**  
- `set_min_sources_required(0)` — minimum must be ≥ 1.
- `set_min_sources_required(n)` where `n` exceeds the current number of registered sources.
- `set_max_price_deviation` with a value > 100 000 basis points.
- `set_heartbeat_interval(0)` — interval must be > 0.

**Resolution**  
Pass a value within the documented valid range for the configuration function. Consult the
contract interface documentation for per-field constraints.

**Example**
```
Error(Contract, #10)
```

---

### Code 11 — `DescriptionTooLong`

| Field | Value |
|-------|-------|
| **Code** | `11` |
| **Enum variant** | `ErrorCode::DescriptionTooLong` |

**Description**  
The description string exceeds the maximum allowed length of 256 characters.

**Cause**  
Both `initialize` and `set_description` enforce a hard limit of 256 characters. Strings
longer than this are rejected.

**Resolution**  
Shorten the description to 256 characters or fewer.

**Example**
```
Error(Contract, #11)
```

---

### Code 12 — `ContractPaused`

| Field | Value |
|-------|-------|
| **Code** | `12` |
| **Enum variant** | `ErrorCode::ContractPaused` |

**Description**  
The contract is currently paused; no price submissions or reads are allowed.

**Cause**  
An admin called `pause_contract()`. All state-changing operations and price queries are
blocked until the contract is unpaused.

**Resolution**  
An admin must call `unpause_contract()` to resume normal operations.

**Example**
```
Error(Contract, #12)
```

---

### Code 13 — `TimelockNotReady`

| Field | Value |
|-------|-------|
| **Code** | `13` |
| **Enum variant** | `ErrorCode::TimelockNotReady` |

**Description**  
A timelock operation cannot yet be executed because its delay period has not elapsed.

**Cause**  
Sensitive admin operations (e.g. `upgrade`, `set_admin`) require a timelock. The operation
must be *proposed* first and then *executed* after `timelock_duration` ledgers have passed.

**Resolution**  
Wait for the required number of ledgers to pass after proposing the operation, then call the
execute endpoint again.

**Example**
```
Error(Contract, #13)
```

---

### Code 14 — `OperationNotFound`

| Field | Value |
|-------|-------|
| **Code** | `14` |
| **Enum variant** | `ErrorCode::OperationNotFound` |

**Description**  
No pending timelock operation exists with the given ID.

**Cause**  
- The operation ID was never created.
- The operation was already executed or cancelled.
- An incorrect ID was supplied.

**Resolution**  
List pending operations to find the correct ID, or propose a new operation.

**Example**
```
Error(Contract, #14)
```

---

### Code 15 — `PriceBelowMinimum`

| Field | Value |
|-------|-------|
| **Code** | `15` |
| **Enum variant** | `ErrorCode::PriceBelowMinimum` |

**Description**  
The submitted price is below the asset's configured minimum price floor.

**Cause**  
An admin set a minimum price for an asset via `set_asset_min_price`. Any `submit_price`
call with a price below this floor is rejected.

**Resolution**  
- Submit a price at or above the asset's minimum price floor.
- An admin can lower or remove the floor via `set_asset_min_price`.

**Example**
```
Error(Contract, #15)
```

---

## Helper: Reading Error Codes

In Soroban test environments the error surfaces as `Error(Contract, #N)` where `N` is the
`u32` discriminant. In production, the host returns a `ScError` of kind `Contract` with
the matching value. Match on the `ErrorCode` enum directly in Rust consumers:

```rust
match result {
    Err(e) if e == ErrorCode::NotAuthorized as u32 => { /* handle */ }
    Err(e) if e == ErrorCode::NoData as u32 => { /* handle */ }
    _ => {}
}
```

For off-chain consumers (TypeScript, Python, etc.) compare the error code integer against
the table above to present a human-readable message.
