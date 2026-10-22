# Price Oracle

A Soroban-based price oracle for Stellar. This document covers the operator-facing
runbook, including the incident severity and decision authority matrix.

## Incident severity and decision authority matrix

Oracle impact levels map to response roles, pause authority, communications, and
escalation deadlines. Use the highest severity that applies.

| Severity | Oracle impact | Incident commander | Pause authority | Communications | Escalation deadline |
| --- | --- | --- | --- | --- | --- |
| SEV1 | Stale or manipulated prices served on-chain; funds at risk | On-call lead + protocol maintainer | On-call lead may pause immediately; maintainer confirms within 15 min | Public status page + consumer notice within 30 min | Acknowledge in 5 min; escalate to maintainers in 15 min |
| SEV2 | Degraded feeds, partial asset coverage, or delayed updates | On-call lead | On-call lead may pause affected assets | Consumer notice within 1 hour | Acknowledge in 15 min; escalate in 1 hour |
| SEV3 | Single asset anomaly with fallback intact | On-call engineer | On-call engineer may pause the single asset | Internal channel notice within 4 hours | Acknowledge in 1 hour; escalate in 4 hours |
| SEV4 | Cosmetic or non-impacting issue | On-call engineer | No pause authority required | Logged in incident tracker | Triage within 1 business day |

### Decision authority rules

- Only the roles listed for a severity may pause the oracle or an asset.
- Pausing above the listed authority requires incident-commander approval.
- Every pause must be recorded with severity, role, timestamp, and rationale.
- Resuming requires the same authority that paused, plus a post-incident review.

## Tabletop exercise

The severity matrix is verified by a repeatable tabletop check. For each severity,
walk the scenario and confirm:

1. The correct incident commander and pause authority are identified.
2. The communications requirement and channel are met within the stated window.
3. The escalation deadline is respected.

Run the tabletop before each release and after any incident. Record the outcome in
the incident tracker; a severity with no correct decision-maker or response path
fails the check and blocks release.

## Development

| Function | Description |
|----------|-------------|
| `submit_price(source, asset, price, timestamp)` | Submit a price (source only) |

### Price Queries

| Function | Description |
|----------|-------------|
| `get_price(asset) -> AggregatePrice` | Get latest aggregated price for an asset |
| `get_source_price(asset, source) -> PriceEntry` | Get latest price from a specific source |
| `get_all_prices(asset) -> Vec<PriceEntry>` | Get latest prices from all sources |
| `get_latest_ledger() -> u32` | Get the latest ledger with price data |

### Historical

| Function | Description |
|----------|-------------|
| `get_historical_price(asset, ledger) -> PriceHistoryEntry` | Get historical price at a specific ledger (interpolated if enabled) |
| `get_historical_prices(asset, start, end) -> Vec<PriceHistoryEntry>` | Get historical prices in an inclusive range of up to 50 ledger numbers |
| `get_historical_prices_paginated(asset, cursor, limit)` | Get up to 50 entries in ascending ledger order; pass `next_cursor` to continue |
| `has_historical_price(asset, ledger) -> bool` | Check if historical price exists |
| `set_interpolation_enabled(bool)` | Enable/disable linear interpolation for history gaps (admin) |
| `get_interpolation_enabled() -> bool` | Check if interpolation is enabled |

### Asset Lifecycle Management

When an asset has a data quality issue, is being replaced, or needs temporary maintenance, the admin can temporarily **disable** or permanently **delist** it:

| Function | Description |
|----------|-------------|
| `disable_asset(asset) -> void` | Temporarily disable an asset (admin only; blocks new submissions and aggregations) |
| `enable_asset(asset) -> void` | Re-enable a disabled asset (admin only; resumes normal operation) |
| `is_asset_disabled(asset) -> bool` | Check if an asset is currently disabled |
| `get_asset_disabled_at(asset) -> u32` | Get ledger sequence when an asset was disabled |
| `unregister_asset(asset) -> void` | Permanently remove an asset from the registry (admin only; price history preserved) |

**Disabled vs Delisted:**
- **Disabled**: Temporary offline state. New submissions blocked. Queries return `ErrorCode::AssetDisabled` (190). Historical prices remain queryable for audit. Can be re-enabled.
- **Delisted**: Permanent removal via `unregister_asset()`. Asset removed from registry. Queries return `ErrorCode::AssetNotRegistered`. Historical prices remain queryable indefinitely.

For full semantics, see [Asset Lifecycle Design](docs/ASSET_LIFECYCLE_DESIGN.md).

### SEP-40 Oracle Consumer Interface

| Function | Description |
|----------|-------------|
| `base() → Asset` | Returns the base asset (USD) |
| `assets() → Vec<Asset>` | Returns all registered assets as `Asset::Stellar` (maximum 100; use pagination for larger registries) |
| `assets_paginated(cursor, limit)` | Get up to 50 assets in registration order; pass `next_cursor` to continue |
| `decimals() → u32` | Returns price decimals |
| `resolution() → u32` | Returns staleness window in seconds (0 = no expiry) |
| `price(asset, timestamp) → Option<PriceData>` | Get price at or before a given timestamp |
| `prices(asset, records) → Option<Vec<PriceData>>` | Get up to 50 latest historical price records |
| `lastprice(asset) → Option<PriceData>` | Get latest aggregated price |

### Contract Events

| Event | Trigger | Topics | Data |
|-------|---------|--------|------|
| `ContractInitializedEvent` | `initialize()` | admin | min_sources, max_history, decimals, description |
| `SourceAddedEvent` | `add_source()` | source, admin | name |
| `SourceRemovedEvent` | `remove_source()` | source, admin | — |
| `AssetRegisteredEvent` | `register_asset()` | asset, admin | — |
| `AssetUnregisteredEvent` | `unregister_asset()` | asset, admin | — |
| `PriceSubmittedEvent` | `submit_price()` | asset, source | price, timestamp |
| `PriceUpdatedEvent` | aggregate price changes | asset | new_price, old_price, timestamp |
| `AdminChangedEvent` | `set_admin()` | old_admin, new_admin | — |
| `ContractUpgradedEvent` | `upgrade()` | new_wasm_hash | — |

## Contract Identity & Verification Registry

Canonical deployments are recorded in a machine-readable registry at
[`config/contract-registry.json`](config/contract-registry.json). Each entry
records the network, the canonical contract ID, the deployed WASM hash, and the
verification timestamp, so consumers can independently confirm that the on-chain
contract matches the recorded release.

### Registry format

```json
{
  "schema_version": 1,
  "entries": [
    {
      "network": "mainnet",
      "contract_id": "CAAAA...",
      "wasm_hash": "<64-char hex sha256 of the deployed WASM>",
      "verified_at": "2025-01-01T00:00:00Z"
    }
  ]
}
```

- `network` — `mainnet`, `testnet`, or `futurenet`
- `contract_id` — the canonical Stellar contract address (`C...`)
- `wasm_hash` — lowercase hex SHA-256 of the deployed WASM (64 hex chars)
- `verified_at` — RFC 3339 UTC timestamp of the last verification

### Verifying a deployment

Use the repeatable check to resolve a contract ID and compare its on-chain WASM
hash against the recorded release:

```bash
./scripts/verify-contract.sh --network mainnet --contract-id CAAAA...
```

The script exits non-zero and prints a clear error when the contract ID is not
in the registry, the recorded hash is malformed, or the on-chain hash does not
match the recorded release. Run it after every deploy or upgrade and in CI to
keep the registry honest.

## oracle-cli

`scripts/oracle-cli.sh` is a shell script plugin that wraps `stellar contract invoke` with
oracle-specific ergonomics.

### Prerequisites

- `stellar` CLI installed and configured
- `ORACLE_CONTRACT_ID` environment variable set to your deployed contract address

### Commands

| Command | Description |
|---------|-------------|
| `oracle-cli init` | Initialize a newly deployed contract |
| `oracle-cli submit-price` | Submit a price from a registered source |
| `oracle-cli get-price` | Get the latest aggregated price for an asset |
| `oracle-cli add-source` | Register a new oracle source (admin) |
| `oracle-cli register-asset` | Register a new asset (admin) |
| `oracle-cli health-check` | Display contract configuration and live status |

### Quick start

```bash
export ORACLE_CONTRACT_ID=CAAAA...
export ORACLE_NETWORK=testnet        # default: testnet

# Initialize
./scripts/oracle-cli.sh init \
  --admin GAAA... --admin-key my-admin \
  --description "My Oracle" --decimals 18

# Add a source
./scripts/oracle-cli.sh add-source \
  --address GBBB... --name "Chainlink" --admin-key my-admin

# Register an asset
./scripts/oracle-cli.sh register-asset \
  --asset GCCC... --admin-key my-admin

# Submit a price  (price = 50 000 × 10^18)
./scripts/oracle-cli.sh submit-price \
  --source GBBB... --asset GCCC... \
  --price 50000000000000000000 \
  --source-key my-source-identity

# Query latest price
./scripts/oracle-cli.sh get-price --asset GCCC...

# Health check
./scripts/oracle-cli.sh health-check
```

### Query response limits and cursors

Historical range queries include at most 50 ledger numbers, and the SEP-40
`prices` query returns at most 50 records. For longer history reads, call
`get_historical_prices_paginated(asset, cursor, limit)` with `cursor = 0` on
the first request and pass each returned `next_cursor` unchanged; it identifies
the first ledger to include on the next page. Results are in ascending ledger
order and gaps are skipped.

The full `assets()` and `list_assets()` responses are capped at 100 assets.
Use `assets_paginated(cursor, limit)` for larger registries: start with
`cursor = None`, then pass the last returned asset as the next cursor. This
preserves registration order without repeats. If that cursor asset has been
unregistered, the contract returns `InvalidPageCursor`; restart from the first
page to avoid silently omitting assets. Page limits for both paginated query
families are 1–50.

## Getting Started

### Prerequisites

- Rust (stable toolchain, see `rust-toolchain.toml`)
- Soroban CLI (optional, for deployment)

### Build

```bash
make build
```

### Test

```bash
make test
```

All **79 tests pass** with zero warnings.

### End-to-End Testnet Test

Deploys the contract to Stellar testnet and runs a full lifecycle test:

```bash
./scripts/e2e-testnet.sh
```

See [docs/e2e-testnet.md](docs/e2e-testnet.md) for prerequisites, configuration, and expected output.

Automated testnet lifecycle CI (with adversarial phases) runs via `scripts/testnet-lifecycle.sh`; see `.github/workflows/testnet-lifecycle.yml`.

Related reports: [gas cost dashboard](docs/gas-dashboard.md), [chaos invariants](docs/chaos-invariants.md), and mutation testing (`scripts/mutation-gate.sh`, config in `.cargo/mutants.toml`).

### Deploy

```bash
soroban contract deploy \
  --wasm target/wasm32v1-none/release/price_oracle.wasm \
  --source <identity> \
  --network testnet
```

After deploying, record the contract ID and WASM hash in
`config/contract-registry.json` and run `./scripts/verify-contract.sh` to confirm
the on-chain hash matches the recorded release.

## Project Structure

```
contracts/price-oracle/
├── Cargo.toml
├── .cargo/config.toml
└── src/
    ├── lib.rs       # Contract entrypoint and endpoint implementations
    ├── types.rs     # Data types, storage keys, error codes
    ├── storage.rs   # Storage helpers and median computation
    ├── events.rs    # Contract event definitions
    └── test.rs      # Test suite (65 tests)
```

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Contract | Rust, Soroban SDK v26 |
| Target | `wasm32v1-none` (WebAssembly) |
| Aggregation | On-chain median (Rust) |
| Testing | `#[cfg(test)]` with `soroban-sdk/testutils` |

## Error Codes

| Code | Name | Description |
|------|------|-------------|
| 0 | `NotAuthorized` | Caller is not the admin or required signer |
| 1 | `AlreadyInitialized` | Contract already initialized |
| 2 | `AssetNotRegistered` | Asset not found |
| 3 | `AssetAlreadyRegistered` | Asset already registered |
| 4 | `SourceAlreadyExists` | Source already registered |
| 5 | `SourceNotFound` | Source not found |
| 6 | `InsufficientSources` | Not enough sources for aggregation |
| 7 | `InvalidPrice` | Price is zero or negative |
| 8 | `NoData` | No price data available (or gap with interpolation disabled) |
| 9 | `InvalidTimestamp` | Submitted timestamp too far in the future |
| 10 | `InvalidConfiguration` | Configuration parameter out of valid range |
| 11 | `DescriptionTooLong` | Description exceeds 256 characters |
| 12 | `ContractPaused` | Contract is paused; operations blocked |
| 13 | `TimelockNotReady` | Timelock delay has not elapsed |
| 14 | `OperationNotFound` | No pending timelock operation with that ID |
| 15 | `PriceBelowMinimum` | Price is below the asset's configured minimum |

See [`docs/error-codes.md`](docs/error-codes.md) for the full registry with causes and resolutions.

## Documentation

| Document | Description |
|---|---|
| [Architecture](docs/ARCHITECTURE.md) | System design, data flow, module structure |
| [Hackathon Bounty Track](docs/hackathon-bounty-track.md) | Hackathon bounty track definition, curated project ideas, judging rubric, and reward tiers |
| [Deployment Record](docs/deployment.md) | Contract addresses, initialization parameters, admin addresses, deployment checklist |
| [Security Audit Checklist](docs/security-audit-checklist.md) | Pre-audit review items: access control, input validation, arithmetic safety, storage safety, upgrade mechanism, event integrity, known patterns |
| [Attack-Regression Corpus](docs/security/attack-regression-corpus.md) | Every historical adversarial scenario pinned as a test, by attack class, with a mutation check proving the corpus has teeth (#511) |
| [Formal Verification](docs/security/formal-verification.md) | Bounded proofs for the aggregation math, the stated bounded domain, and the proven-vs-sampled split (#512) |
| [State-Machine Model](docs/security/state-machine-model.md) | Independently written model of the aggregate lifecycle, driven by shrinking random traces (#513) |
| [Coverage-Guided Fuzzing](docs/security/fuzzing.md) | Fuzz targets, the committed minimized corpus, and the CI crash and size gates (#514) |
| [Monitoring Dashboard](docs/monitoring/README.md) | Grafana dashboard setup and metrics reference |
| [Quickstart Templates](examples/README.md) | CI-tested web (SEP-40), Python bot and indexer templates |
| [Interface Package](docs/interface-versioning.md) | Versioned `@stellar-unified-price-oracle/interface` npm package, semver rules, deprecation policy |
| [Error Registry](docs/errors/REGISTRY.md) | Every error code with meaning, cause and remediation ([JSON](docs/errors/registry.json)) |
| [Migration Guides](docs/migrations/README.md) | Per-version guides for breaking changes and deprecation windows |
| [Integration Case Studies](docs/case-studies.md) | Lending, DEX and payments integrations with threat models, worked exploits and countermeasures |
| [Seam Invariant Catalogue](docs/seam-invariants.md) | Cross-module invariants and the regression tests that enforce them |
| [SLA](docs/SLA.md) | Service levels and the machine-enforceable clause map checked by `services/sla_monitor` |
| [Runbook](docs/runbook.md) | Every paging alert mapped to meaning, first check, mitigation, escalation, resolution, owner and review cadence (#527) |
| [Incident Management](docs/incident-management/README.md) | Blameless postmortem process, template, index and tracked action items (#528) |
| [Backup and Restore](docs/backup-restore.md) | Off-chain state inventory, encrypted backups, point-in-time restore and reconciliation (#529) |
| [Capacity Planning](docs/capacity-planning.md) | Ingest, storage and ledger-budget model with headroom targets and load shedding (#530) |
| [Price Revision Metrics](docs/price-revision-metrics.md) | Correction-rate metric per asset and per source, cause separation, volume normalization and sustained-increase alerting (#499) |
| [Static Analysis](docs/static-analysis.md) | SAST rules, dependency-advisory gate, owned+expiring allowlist and the re-baseline process (#500) |
| [Reproducible Builds](docs/reproducible-builds.md) | Exact dependency pins, canonical containerized build environment, two-build digest check (#501) |
| [Secret Scanning](docs/secret-scanning.md) | Pre-commit and CI secret scanning, history scan, allowlist process and the rotation playbook (#502) |

## Documentation

- [Hackathon Bounty Track](docs/hackathon-bounty-track.md) — reusable hackathon track definition, curated ideas mapped to open issues, judging rubric, and reward tiers (#407)
- [Price Submission Bot Design](docs/price-submission-bot.md) — off-chain bot architecture for automated price submissions
- [Signed CEX/Aggregator Adapters](docs/signed-price-adapters.md) — CoinGecko/Binance/Coinbase adapters with failover, staleness handling, and signed submission (#216)
- [Disaster Recovery Plan](docs/disaster-recovery.md) — failure scenario playbooks and recovery procedures
- [Architecture](docs/ARCHITECTURE.md) — contract design and data flow
- [Monitoring](docs/monitoring/README.md) — Grafana dashboard and alerting setup

## License

MIT

## Service Level Agreement

See [docs/SLA.md](docs/SLA.md) for price freshness guarantees, uptime commitments, deviation thresholds, incident response times, and compensation terms.

## Handsoff notes

<!-- handsoff-issue-672 -->
- #672: [Economics] Fee and reward parameter sensitivity analysis

<!-- handsoff-issue-673 -->
- #673: [Economics] Economic emergency reserve policy
