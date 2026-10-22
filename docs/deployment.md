# Deployment Record — Stellar Unified Price Oracle

This document records the mainnet and testnet deployment configuration, contract addresses, initialization parameters, and admin details for all environments. Update this file after every deployment or configuration change.

---

## Contract Addresses

| Environment | Contract ID | Network Passphrase | Deployed At (Ledger) | Notes |
|---|---|---|---|---|
| **Mainnet** | `TBD` | `Public Global Stellar Network ; September 2015` | — | Not yet deployed |
| **Testnet** | `TBD` | `Test SDF Network ; September 2015` | — | Not yet deployed |
| **Futurenet** | `TBD` | `Test SDF Future Network ; October 2022` | — | Not yet deployed |

> **Instructions:** Replace each `TBD` with the contract ID returned by `soroban contract deploy` after deployment. Record the ledger sequence number at the time of deployment for auditability.

---

## Admin Addresses

| Environment | Admin Address | Type | Notes |
|---|---|---|---|
| **Mainnet** | `TBD` | Multi-sig recommended | Must be a hardware wallet or multi-sig for production |
| **Testnet** | `TBD` | Single key (test identity) | Test only — do not use a production key |

> **Security note:** The admin address has full control over source/asset management, contract upgrades, and configuration changes. For mainnet, the admin MUST be a multi-sig account or a governed smart contract. A single private key is not acceptable for production.

---

## Initialization Parameters

These are the parameters that must be passed to `initialize()` at deployment time. Record actual values used for each environment.

### Mainnet

| Parameter | Value Used | Notes |
|---|---|---|
| `admin` | `TBD` | Multi-sig address |
| `min_sources_required` | `TBD` | Recommended: ≥ 3 for production |
| `max_history_length` | `TBD` | Recommended: 100–500 |
| `decimals` | `TBD` | Typically `7` (Stellar native) or `18` |
| `description` | `TBD` | e.g. `"Stellar Unified Price Oracle — Mainnet"` |

### Testnet

| Parameter | Value Used | Notes |
|---|---|---|
| `admin` | `TBD` | Test identity |
| `min_sources_required` | `TBD` | Can be `1` for testing |
| `max_history_length` | `TBD` | |
| `decimals` | `TBD` | |
| `description` | `TBD` | e.g. `"Stellar Unified Price Oracle — Testnet"` |

---

## Post-Initialization Configuration

These settings are applied via admin calls after `initialize()`. Record the values used in each environment.

### Mainnet

| Setting | Function | Value Used | Notes |
|---|---|---|---|
| Timestamp threshold | `set_timestamp_threshold` | `TBD` | Default: `300` (5 min). Consider tightening for production. |
| Max price deviation | `set_max_price_deviation` | `TBD` | Default: `500` bp (5%). Tune per asset volatility. |
| Heartbeat interval | `set_heartbeat_interval` | `TBD` | Default: `3600` (1 hr). |
| Resolution window | `set_resolution` | `TBD` | Default: `0` (disabled). Set a staleness window for production. |
| Timelock duration | `set_timelock_duration` | `TBD` | Default: `10` ledgers. Increase for production governance safety. |
| Aggregation method | Set at init | `0` (Median) | Default. Change only with strong justification. |

### Testnet

| Setting | Function | Value Used | Notes |
|---|---|---|---|
| Timestamp threshold | `set_timestamp_threshold` | `TBD` | |
| Max price deviation | `set_max_price_deviation` | `TBD` | |
| Heartbeat interval | `set_heartbeat_interval` | `TBD` | |
| Resolution window | `set_resolution` | `TBD` | |
| Timelock duration | `set_timelock_duration` | `TBD` | |

---

## Registered Oracle Sources

Record all sources added via `add_source(address, name)` after deployment.

### Mainnet

| Source Address | Display Name | Added At (Ledger) | Status |
|---|---|---|---|
| `TBD` | `TBD` | — | — |

### Testnet

| Source Address | Display Name | Added At (Ledger) | Status |
|---|---|---|---|
| `TBD` | `TBD` | — | — |

> **Note:** Each source must be a permissioned address controlled by a trusted data provider. Sources must call `submit_heartbeat()` periodically within the configured `heartbeat_interval` to remain active.

---

## Registered Assets

Record all assets added via `register_asset(address)` after deployment.

### Mainnet

| Asset Address | Symbol | Registered At (Ledger) | Min Price Floor | Notes |
|---|---|---|---|---|
| `TBD` | `TBD` | — | `TBD` | |

### Testnet

| Asset Address | Symbol | Registered At (Ledger) | Min Price Floor | Notes |
|---|---|---|---|---|
| `TBD` | `TBD` | — | `TBD` | |

---

## WASM Build Information

Record the WASM hash used for each deployment. This hash is required to verify upgrades and to reproduce the deployment.

| Environment | WASM Hash | SDK Version | Rust Version | Built At |
|---|---|---|---|---|
| **Mainnet** | `TBD` | soroban-sdk 26 | See `rust-toolchain.toml` | — |
| **Testnet** | `TBD` | soroban-sdk 26 | See `rust-toolchain.toml` | — |

To compute the WASM hash after building:

```bash
# Build release WASM
cargo build -p price-oracle --target wasm32v1-none --release

# Get the hash (SHA-256 of the WASM binary)
sha256sum target/wasm32v1-none/release/price_oracle.wasm
```

---

## Deployment Checklist

Complete every item before deploying to each environment.

### Pre-Deployment

- [ ] All tests pass: `cargo test -p price-oracle --lib`
- [ ] Zero clippy warnings: `cargo clippy -p price-oracle -- -D warnings`
- [ ] Zero format issues: `cargo fmt --manifest-path contracts/price-oracle/Cargo.toml -- --check`
- [ ] Release WASM built: `cargo build -p price-oracle --target wasm32v1-none --release`
- [ ] WASM hash recorded in this document
- [ ] Admin address confirmed (multi-sig for mainnet)
- [ ] Initialization parameters reviewed and agreed upon
- [ ] Soroban CLI installed and identity configured for the target network
- [ ] Sufficient XLM in deployer account to cover deployment fees

### Deployment

- [ ] Deploy contract and record returned contract ID in this document
- [ ] Record ledger sequence number at deployment time
- [ ] Call `initialize()` with agreed parameters
- [ ] Verify `get_admin_address()` returns the expected admin
- [ ] Verify `get_min_sources_required()` returns the expected value
- [ ] Verify `get_decimals()` returns the expected value

### Post-Deployment Configuration

- [ ] Apply post-init configuration (timestamp threshold, deviation, heartbeat interval, resolution, timelock duration)
- [ ] Register all oracle sources via `add_source()`
- [ ] Register all tracked assets via `register_asset()`
- [ ] Verify each source with `is_source(address)` → `true`
- [ ] Verify each asset with `is_asset_registered(address)` → `true`
- [ ] Request each source to submit an initial price to confirm end-to-end flow
- [ ] Verify `get_price(asset, 0)` returns a valid `AggregatePrice`
- [ ] Verify events are visible on a Stellar explorer or indexer

### Mainnet-Specific Additional Steps

- [ ] Timelock duration set to a production-appropriate value (recommended: ≥ 100 ledgers)
- [ ] Resolution window set to a non-zero staleness threshold appropriate for the assets
- [ ] Monitoring dashboard configured (see `docs/monitoring/README.md`)
- [ ] Alerting rules configured for `InsufficientSources` and stale price conditions
- [ ] Incident response procedure documented and communicated to the team
- [ ] Security audit completed and findings resolved (see `docs/security-audit-checklist.md`)

---

## Upgrade History

Record every contract upgrade (WASM hash change) here.

| Environment | Date | Old WASM Hash | New WASM Hash | Proposed Ledger | Executed Ledger | Executed By | Notes |
|---|---|---|---|---|---|---|---|
| — | — | — | — | — | — | — | No upgrades yet |

---

## Deployment Commands Reference

```bash
# Build the contract
cargo build -p price-oracle --target wasm32v1-none --release

# Deploy to testnet
soroban contract deploy \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --source <identity> \
  --network testnet

# Deploy to mainnet
soroban contract deploy \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --source <identity> \
  --network mainnet

# Initialize the contract (replace placeholders)
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <admin-identity> \
  --network <network> \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --min_sources_required <N> \
  --max_history_length <N> \
  --decimals <N> \
  --description "<description>"

# Add an oracle source
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <admin-identity> \
  --network <network> \
  -- add_source \
  --source <SOURCE_ADDRESS> \
  --name "<Source Name>"

# Register an asset
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source <admin-identity> \
  --network <network> \
  -- register_asset \
  --asset <ASSET_CONTRACT_ADDRESS>
```
