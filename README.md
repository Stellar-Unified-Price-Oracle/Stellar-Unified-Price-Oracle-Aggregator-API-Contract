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

## Overview

This service aggregates price feeds and publishes them on-chain for consumption by Soroban contracts. It is designed to move from feature-complete development into a dependable production service, with explicit, reviewable, and verifiable mainnet readiness decisions.

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

Every asset enabled at launch must satisfy the policy below. A new asset **cannot** enter the production manifest unless all required review fields are present and signed off.

### 1. Eligibility

An asset is eligible for the production manifest only if it:

- Is a supported Stellar/Soroban asset (native, issued, or contract-wrapped) with a verifiable on-chain identifier.
- Has at least one reliable, independent price source available to the oracle.
- Has sufficient liquidity and market depth to produce a non-manipulable reference price.
- Is not subject to sanctions, legal restrictions, or issuer controls that would make pricing unreliable.

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

**JavaScript / TypeScript**

```ts
const DECIMALS = 7;
const SCALE = 10n ** BigInt(DECIMALS);
const I128_MAX = (1n << 127n) - 1n;
const I128_MIN = -(1n << 127n);

// Round-half-up on a rational (numerator / denominator).
function roundHalfUp(numerator: bigint, denominator: bigint): bigint {
  const q = numerator / denominator;
  const r = numerator % denominator;
  if (r === 0n) return q;
  const absR = r < 0n ? -r : r;
  const bump = absR * 2n >= denominator ? 1n : 0n;
  return numerator < 0n ? q - bump : q + bump;
}

function toInteger(human: number): bigint {
  // Convert via string to avoid float precision loss.
  const [whole, frac = ""] = human.toString().split(".");
  const fracPadded = (frac + "0".repeat(DECIMALS)).slice(0, DECIMALS);
  const numerator = BigInt(whole + fracPadded);
  const scaled = roundHalfUp(numerator, 1n);
  if (scaled > I128_MAX || scaled < I128_MIN) {
    throw new Error("price out of i128 range");
  }
  return scaled;
}

function toHuman(integer: bigint): string {
  const neg = integer < 0n;
  const abs = neg ? -integer : integer;
  const whole = abs / SCALE;
  const frac = (abs % SCALE).toString().padStart(DECIMALS, "0");
  return `${neg ? "-" : ""}${whole}.${frac}`;
}

// Examples (decimals = 7)
console.assert(toInteger(0.0000001) === 1n);
console.assert(toInteger(1.2345678) === 12345678n);
console.assert(toHuman(12345678n) === "1.2345678");
```

**Python**

```python
from decimal import Decimal, ROUND_HALF_UP

DECIMALS = 7
SCALE = 10 ** DECIMALS
I128_MAX = (1 << 127) - 1
I128_MIN = -(1 << 127)


def to_integer(human: Decimal) -> int:
    scaled = (human * SCALE).quantize(Decimal(1), rounding=ROUND_HALF_UP)
    value = int(scaled)
    if value > I128_MAX or value < I128_MIN:
        raise ValueError("price out of i128 range")
    return value


def to_human(integer: int) -> Decimal:
    return Decimal(integer) / SCALE


# Examples (decimals = 7)
assert to_integer(Decimal("0.0000001")) == 1
assert to_integer(Decimal("1.2345678")) == 12345678
assert to_integer(Decimal("0.00000005")) == 1  # round-half-up
assert to_human(12345678) == Decimal("1.2345678")
```

After deploying, record the contract ID and WASM hash in
`config/contract-registry.json` and run `./scripts/verify-contract.sh` to confirm
the on-chain hash matches the recorded release.

## Project Structure

Before initializing a production deployment, review the four initialization
parameters below against the policy for that deployment target. The policy is
encoded in `scripts/check-init-params.sh` and enforced in CI, so an
out-of-policy configuration is rejected before it can reach mainnet.

### Parameters under review

| Parameter | Meaning | Mainnet policy | Testnet / local policy |
|-----------|---------|----------------|------------------------|
| `min_sources` | Minimum distinct sources required before an aggregate price is published | `>= 3` (median needs an odd quorum to resist a single outlier) | `>= 1` |
| `decimals` | Price scaling exponent exposed via SEP-40 `decimals()` | `<= 18` (must match consumer expectations; no silent rescale) | `<= 18` |
| `max_history` | Maximum historical records retained per asset | `>= 1` and `<= 10_000` (bounds storage growth) | `>= 1` and `<= 10_000` |
| `description` | Human-readable contract description emitted in `ContractInitializedEvent` | non-empty, `<= 256` bytes | non-empty, `<= 256` bytes |

### Boundary conditions

- `min_sources = 0` is always rejected — aggregation would be undefined.
- `min_sources` greater than the number of registered sources means no price
  is published until enough sources are added; this is a valid but degraded
  state and is surfaced by `oracle-cli health-check`.
- `decimals = 0` is allowed (integer prices); values above 18 are rejected.
- `max_history = 0` disables history retention and is rejected for production.
- An empty or whitespace-only `description` is rejected.

### Failure and recovery

If `initialize` is called with out-of-policy values the contract still stores
them (the contract does not hard-code network policy), so the review is the
gate. To recover, the admin uses `set_min_sources_required`,
`set_max_history_length`, `set_decimals`, and `set_description` to bring the
contract back within policy, then re-runs the validation command below.

### Reproducible validation

```bash
# Validate a candidate initialization config against the deployment policy.
# Exits non-zero (rejecting the config) when any parameter is out of policy.
./scripts/check-init-params.sh --network mainnet --config deploy/mainnet-init.env
```

The config file is a simple `KEY=VALUE` env file:

```env
MIN_SOURCES=3
DECIMALS=18
MAX_HISTORY=1000
DESCRIPTION="Stellar Unified Price Oracle (mainnet)"
```

### Assumptions

All figures are monthly, denominated in USD, and reviewed whenever the
underlying inputs change materially.

| Input | Value | Notes |
| --- | --- | --- |
| Committed funding (treasury balance) | $60,000 | Funds already committed and available. |
| Baseline operational spend | $2,000 / month | Hosting, RPC/indexer access, monitoring, on-call stipend. |
| Low-usage operational spend | $1,400 / month | Reduced infrastructure tier; fewer provider submissions. |
| High-incident operational spend | $4,500 / month | Incident response, extra redundancy, surge support. |
| Funding-review threshold | 6 months of runway | Below this, a funding review is triggered. |

Operational spend covers infrastructure, provider incentives, monitoring, and
incident response. It excludes one-off protocol development costs.

### Runway per scenario

Runway is `committed funding / monthly operational spend`.

| Scenario | Monthly spend | Runway | Status vs. 6-month threshold |
| --- | --- | --- | --- |
| Baseline | $2,000 | 30.0 months | Healthy |
| Low-usage | $1,400 | ~42.9 months | Healthy |
| High-incident | $4,500 | ~13.3 months | Healthy, but monitor |

### Funding-review threshold

A funding review is triggered when projected runway falls **below 6 months**
under the scenario currently in effect. At the baseline spend of $2,000/month,
that corresponds to a treasury balance below **$12,000**. The review covers
replenishment options, spend reduction, and whether the high-incident scenario
has become the steady state.

### Boundary and failure/recovery paths

- **Boundary:** runway exactly at the 6-month threshold does not trigger a
  review; only strictly below it does.
- **Failure:** if the high-incident scenario persists, spend rises to
  $4,500/month and runway shrinks accordingly; sustained high-incident spend is
  treated as a funding-review trigger even while nominal runway is above the
  threshold.
- **Recovery:** once incident spend returns to baseline, runway is recomputed
  from the restored spend and the review condition is re-evaluated.

### Repeatable check

See [docs/SLA.md](docs/SLA.md) for price freshness guarantees, uptime commitments, deviation thresholds, incident response times, and compensation terms.

## Handsoff notes

<!-- handsoff-issue-672 -->
- #672: [Economics] Fee and reward parameter sensitivity analysis

<!-- handsoff-issue-673 -->
- #673: [Economics] Economic emergency reserve policy
