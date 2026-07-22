# Security Audit Checklist — Stellar Unified Price Oracle

This document lists all items that must be reviewed or completed before a professional security audit of the price oracle contract. Each section maps to a specific audit domain and references the relevant source files.

---

## 1. Access Control Review

**Relevant files:** `src/admin.rs`, `src/sources.rs`, `src/assets.rs`, `src/prices.rs`, `src/pause.rs`, `src/timelock.rs`

### Admin Authentication
- [ ] All admin-only functions call `get_admin(env).require_auth()` before any state mutation
- [ ] `initialize` correctly gates re-initialization via `DataKey::Admin` existence check
- [ ] `set_admin` emits `AdminChangedEvent` and the change takes effect atomically
- [ ] `upgrade` is admin-only and emits `ContractUpgradedEvent` before applying the WASM hash
- [ ] `pause` / `unpause` are gated behind admin auth
- [ ] Timelock functions (`propose_operation`, `execute_operation`, `cancel_operation`, `set_timelock_duration`) all require admin auth
- [ ] Price override functions (`override_price`, `remove_price_override`) are admin-only

### Source Authentication
- [ ] `submit_price` calls `source.require_auth()` before any price or storage logic
- [ ] `submit_heartbeat` calls `source.require_auth()` before updating heartbeat state
- [ ] `check_source` in `storage.rs` panics with `ErrorCode::NotAuthorized` for non-registered sources
- [ ] Suspended sources are rejected in `submit_price` (`is_source_suspended` check)

### Role Separation
- [ ] No pathway exists for a source to call admin-only functions
- [ ] No pathway exists for an admin to submit prices without being a registered source
- [ ] Read functions (get_price, get_all_prices, etc.) are unrestricted — confirm this is intentional
- [ ] `check_not_paused` is invoked at the top of `submit_price` before auth checks

### Privilege Escalation
- [ ] There is no way to register a new admin except via `set_admin` with current admin auth
- [ ] There is no back-door initialization path after `DataKey::Admin` is set
- [ ] The timelock mechanism cannot be used to bypass admin auth (requires admin to propose AND execute)

---

## 2. Input Validation Audit

**Relevant files:** `src/prices.rs`, `src/admin.rs`, `src/assets.rs`, `src/sources.rs`, `src/types.rs`

### Price Submission
- [ ] `price <= 0` is rejected with `ErrorCode::InvalidPrice` before storage write
- [ ] `price < asset_min_price` is rejected with `ErrorCode::PriceBelowMinimum`
- [ ] `timestamp > ledger_time + threshold` is rejected with `ErrorCode::InvalidTimestamp` (default threshold: 300s)
- [ ] Source must be registered and not suspended before price is accepted
- [ ] Asset must be registered before price is accepted

### Configuration Parameters
- [ ] `min_sources_required = 0` is rejected with `ErrorCode::InvalidConfiguration`
- [ ] `set_min_sources_required` validates new value does not exceed current source count
- [ ] `deviation_basis_points > 100_000` is rejected with `ErrorCode::InvalidConfiguration`
- [ ] `heartbeat_interval = 0` is rejected with `ErrorCode::InvalidConfiguration`
- [ ] `description.len() > 256` is rejected with `ErrorCode::DescriptionTooLong`
- [ ] Price override `expiry_ledger <= current_ledger` is rejected with `ErrorCode::InvalidConfiguration`
- [ ] Price override `price <= 0` is rejected with `ErrorCode::InvalidPrice`

### Aggregation Logic
- [ ] Aggregation only proceeds when `contributing_sources >= min_sources_required`
- [ ] `valid_prices` is never empty before median/mean computation is called
- [ ] `compute_median` handles both even and odd source counts correctly
- [ ] Aggregation method discriminant falls back to median for unknown values
- [ ] `compute_mean` uses `saturating_add` to avoid i128 overflow

### Edge Cases
- [ ] `prices()` with `records = 0` returns `Some(empty Vec)`, not `None`
- [ ] `get_price` with `max_age = 0` disables age filter (resolution filter still applies)
- [ ] Historical price scan bounds (`start_ledger > end_ledger`) are handled
- [ ] `get_historical_prices` validates range does not exceed `max_history_length`

---

## 3. Arithmetic Safety Review

**Relevant files:** `src/storage.rs`, `src/prices.rs`, `src/admin.rs`

### Overflow / Underflow
- [ ] `compute_median` for even-count case uses `a + (b - a) / 2` to avoid intermediate overflow
- [ ] `compute_mean` uses `saturating_add` when summing i128 prices
- [ ] `compute_trimmed_mean` uses `saturating_mul` when computing trim count
- [ ] Heartbeat timeout check uses `saturating_add` for `hb_time + interval`
- [ ] `get_price_change` uses `saturating_sub` for price difference and multiplication

### Precision & Rounding
- [ ] Decimal precision is fixed at initialization and does not change retroactively for existing entries
- [ ] `set_decimals` documents that it does not rescale stored prices (see lib.rs docstring)
- [ ] Integer division truncation in median/mean computation is acceptable and documented
- [ ] Basis-point deviation calculations use integer arithmetic consistently

### Storage Arithmetic
- [ ] TTL values (`LEDGER_THRESHOLD = 1000`, `LEDGER_BUMP = 4000`) are sufficient for production use
- [ ] Ledger arithmetic in history scan (`current_ledger.saturating_sub(max_to_check)`) cannot underflow
- [ ] `PendingOpCount` increments (`op_count + 1`) cannot overflow `u32` under realistic usage

---

## 4. Storage Safety Review

**Relevant files:** `src/storage.rs`, `src/types.rs`, `src/prices.rs`, `src/history.rs`

### TTL Management
- [ ] All persistent storage reads that are load-bearing extend TTL on access
- [ ] `DataKey::Admin` TTL is extended in `get_admin_address`
- [ ] `DataKey::Aggregate` TTL is extended after price aggregation and on read
- [ ] `DataKey::Submission` TTL is extended when iterated during aggregation
- [ ] `DataKey::OracleSources` TTL is extended on read via `read_oracle_sources`
- [ ] Source and asset registration keys have TTL extended on positive lookup

### Data Consistency
- [ ] `unregister_asset` removes both `DataKey::AssetRegistered` and `DataKey::Aggregate` entries
- [ ] `remove_source` removes `DataKey::Source` and updates `OracleSources` atomically within one invocation
- [ ] `register_asset` checks for existing registration before writing to prevent duplicate state
- [ ] History ledger index (`DataKey::PriceHistoryLedgers`) stays consistent with actual history entries
- [ ] History pruning removes the corresponding `DataKey::PriceHistory` temporary entry and ledger index entry

### Temporary vs Persistent Storage
- [ ] Price history entries (`DataKey::PriceHistory`) are correctly stored in temporary storage
- [ ] History ledger index (`DataKey::PriceHistoryLedgers`) is correctly stored in persistent storage
- [ ] No critical state is stored in temporary storage that must survive ledger expiration

### Storage Key Uniqueness
- [ ] `DataKey` variants are unique and do not alias each other
- [ ] Address-parameterized keys (`Source(Address)`, `AssetRegistered(Address)`) cannot collide across types
- [ ] `(asset, source)` submission keys correctly namespace per asset-source pair

---

## 5. Upgrade Mechanism Review

**Relevant files:** `src/admin.rs`, `src/timelock.rs`, `src/lib.rs`

### Direct Upgrade Path
- [ ] `upgrade(new_wasm_hash)` requires current admin auth
- [ ] WASM update is applied via `env.deployer().update_current_contract_wasm()` after auth check
- [ ] `ContractUpgradedEvent` is emitted before the WASM is replaced (event integrity on upgrade)
- [ ] No post-upgrade migration logic exists — document that storage layout changes require manual migration

### Timelock-Protected Upgrade
- [ ] `OperationType::Upgrade` operations pass through the timelock queue
- [ ] Timelock duration is configurable by admin (`set_timelock_duration`)
- [ ] Default timelock duration is 10 ledgers — evaluate whether this is sufficient for production
- [ ] `execute_operation` validates `elapsed >= timelock_duration` before applying changes
- [ ] Cancelled operations are cleanly removed with no lingering state
- [ ] Operation IDs are monotonically incrementing and cannot be reused

### Upgrade Safety
- [ ] Admin role is preserved after an upgrade (stored in persistent storage, not WASM)
- [ ] All `DataKey` variants used in the new WASM must be backward-compatible with stored data
- [ ] Consider whether a two-step upgrade (propose → timelock → execute) should be enforced for all upgrades

---

## 6. Event Integrity Review

**Relevant files:** `src/events.rs`, `src/admin.rs`, `src/sources.rs`, `src/assets.rs`, `src/prices.rs`, `src/pause.rs`, `src/timelock.rs`

### Event Completeness
- [ ] Every state-changing function emits at least one event
- [ ] `initialize` emits via `emit_initialized` (manual publish due to SDK limitation)
- [ ] `set_admin` emits `AdminChangedEvent` with both old and new admin addresses
- [ ] `upgrade` emits `ContractUpgradedEvent` with the new WASM hash
- [ ] `add_source` / `remove_source` emit `SourceAddedEvent` / `SourceRemovedEvent`
- [ ] `register_asset` / `unregister_asset` emit `AssetRegisteredEvent` / `AssetUnregisteredEvent`
- [ ] `submit_price` always emits `PriceSubmittedEvent`, then either `PriceAggregatedEvent` or `SourcesInsufficientEvent`
- [ ] `pause` / `unpause` emit `ContractPausedEvent` / `ContractUnpausedEvent`
- [ ] All timelock operations emit corresponding `OperationProposed/Executed/CancelledEvent`
- [ ] Price override set/remove/expiry all emit events

### Event Accuracy
- [ ] `AdminChangedEvent` captures both `old_admin` and `new_admin` (prevents silent admin hijacking)
- [ ] `PriceAggregatedEvent` reports the correct `num_sources` at aggregation time
- [ ] `HistoryPrunedEvent` emits before the prune loop exits to avoid missed events
- [ ] `PriceOverrideExpiredEvent` is emitted when an expired override is encountered during `get_price`
- [ ] Manual events (`emit_initialized`, `emit_timestamp_threshold_changed`, `emit_max_price_deviation_changed`) use distinct symbol names (`init`, `tthr`, `devn`) to avoid topic collisions

### Event Indexability
- [ ] All events use `#[topic]` on address fields that indexers will filter by
- [ ] `PriceSubmittedEvent` topics include both `asset` and `source` for efficient filtering
- [ ] `AdminChangedEvent` topics include both `old_admin` and `new_admin`
- [ ] Events emitted in error paths (e.g. `PriceStaleEvent`, `SourcesInsufficientEvent`) are correctly attributed

---

## 7. Known Patterns and Mitigations

### Oracle Manipulation
- **Median aggregation** — the contract uses median by default, which tolerates up to `(n-1)/2` corrupted sources before the aggregate is affected. Verify `min_sources_required` is set high enough that a single compromised source cannot dominate.
- **Price deviation flag** — `MAX_PRICE_DEVIATION` (default 500 bp / 5%) flags outlier submissions via `PriceDeviationFlaggedEvent`. Verify this threshold is appropriate for the assets priced.
- **Timestamp manipulation** — submissions with timestamps more than `timestamp_threshold` (default 300s) in the future are rejected. Confirm this window is not too wide for fast-moving markets.
- [ ] Confirm median is used for all production-critical assets (not mean or trimmed mean which are less manipulation-resistant)

### Source Compromise
- **Heartbeat mechanism** — inactive sources (no heartbeat within `heartbeat_interval`) are excluded from aggregation. Verify interval is tuned for the deployment environment.
- **Source suspension** — `is_source_suspended` is currently a stub (always returns false). Evaluate whether automated suspension based on invalid submission count (`MAX_INVALID_SUBMISSIONS = 5`) should be implemented before audit.
- **`record_invalid_submission`** — currently a no-op stub. The counter exists in config but is not enforced. Document this gap explicitly.
- [ ] Determine whether manual source removal is sufficient or if automated suspension is required

### Reentrancy
- Soroban's execution model is single-threaded and does not support cross-contract callbacks within a single invocation in the same manner as EVM. However:
- [ ] Confirm no cross-contract calls are made within `submit_price` or any state-mutating function
- [ ] Confirm `env.deployer().update_current_contract_wasm()` in `upgrade` cannot trigger reentrancy

### Stale Price Risk
- [ ] `resolution` is set to `0` by default — prices never expire unless explicitly configured. Set a sensible resolution window before mainnet deployment.
- [ ] `max_age` parameter in `get_price` is caller-controlled — document that consumers must pass a reasonable value.
- [ ] Price override expiry is ledger-based, not time-based — document the ledger/time conversion assumption.

### Denial of Service
- [ ] `get_inactive_sources` iterates all sources — confirm source count is bounded for gas safety
- [ ] `get_all_prices` iterates all sources — confirm source count is bounded
- [ ] `get_historical_prices` range is bounded by `max_history_length`
- [ ] `prices()` scan is bounded by `records * 10` with a hard cap of 10,000 ledgers

### Single-Admin Risk
- The contract uses a single admin address with no multi-sig enforcement at the contract level. Mitigations:
- [ ] Admin address should be a multi-sig wallet or governed contract on mainnet
- [ ] Timelock duration should be increased from the default 10 ledgers for production governance operations
- [ ] Admin key rotation procedure should be documented and tested before mainnet

### No-Std / WASM Safety
- [ ] All arithmetic uses `i128` (no external numeric libraries) — confirm no hidden panics on edge values
- [ ] `panic_with_error!` macro is used consistently instead of bare `panic!` or `unwrap` in production paths
- [ ] `unwrap()` calls in `get_admin` and `get_source_price` — document that these assume invariants established at initialization

---

## Pre-Audit Completion Checklist

Before scheduling the audit engagement, confirm:

- [ ] All items above have been reviewed and findings documented
- [ ] Source suspension / invalid submission enforcement gap is resolved or explicitly accepted
- [ ] `resolution` default of `0` is changed or a deployment guide documents required configuration
- [ ] Timelock duration default (10 ledgers) has been evaluated and adjusted for mainnet
- [ ] Admin address for mainnet is a multi-sig; documented in `docs/deployment.md`
- [ ] All 76+ tests pass with zero warnings (`cargo test -p price-oracle --lib`)
- [ ] Clippy passes with zero warnings (`cargo clippy -p price-oracle -- -D warnings`)
- [ ] Contract WASM is built in release mode and hash is recorded
- [ ] Audit scope is defined (contract source only, or including indexer/monitoring infrastructure)
- [ ] Known limitations and intentional design decisions are documented for the auditors
