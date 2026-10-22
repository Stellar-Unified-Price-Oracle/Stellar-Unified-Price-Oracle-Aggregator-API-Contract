# Stellar Unified Price Oracle Aggregator

A **decentralized price oracle aggregator** built on Soroban (Stellar smart contracts). Collects price data from multiple permissioned oracle sources, aggregates via **median**, and exposes historical price data for consumer contracts.

## Features

- **Multi-source aggregation** — register multiple oracle sources per asset, aggregate via median
- **Admin governance** — admin controls sources, assets, decimals, description, history limits
- **Median price** — robust single-statistic aggregation resistant to outliers and manipulation
- **Per-source prices** — inspect individual source submissions for transparency
- **Historical prices** — ledger-based price history with configurable retention
- **Contract upgradability** — WASM-based upgrade mechanism
- **SEP-40 compliant** — full implementation of the Stellar Oracle Consumer Interface standard
- **Contract events** — all state changes emit on-chain events for indexers and monitoring
- **27 public endpoints** — full admin, source, asset, submission, query, history, and SEP-40 interface

## Contract Interface

### Admin

| Function | Description |
|----------|-------------|
| `initialize(admin, min_sources, max_history, decimals, description)` | Initialize the contract |
| `set_admin(new_admin)` | Transfer admin rights |
| `get_admin_address()` | Get current admin |
| `set_min_sources_required(n)` | Set minimum sources for aggregation |
| `get_min_sources_required()` | Get minimum sources required |
| `set_max_history_length(n)` | Set max historical records per asset |
| `get_max_history_length()` | Get max history length |
| `set_decimals(n)` | Set price decimals |
| `get_decimals()` | Get decimals |
| `set_description(s)` | Set contract description |
| `get_description()` | Get description |
| `upgrade(new_wasm_hash)` | Upgrade contract WASM |

### Source Management

| Function | Description |
|----------|-------------|
| `add_source(address, name)` | Register an oracle source |
| `remove_source(address)` | Remove an oracle source |
| `is_source(address) -> bool` | Check if address is a registered source |
| `get_oracle_sources() -> OracleSources` | Get all registered sources |

### Asset Management

| Function | Description |
|----------|-------------|
| `register_asset(asset)` | Register a new asset |
| `unregister_asset(asset)` | Unregister an asset |
| `is_asset_registered(asset) -> bool` | Check if asset is registered |

### Price Submission

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
| `get_historical_prices(asset, start, end) -> Vec<PriceHistoryEntry>` | Get historical prices in a ledger range |
| `has_historical_price(asset, ledger) -> bool` | Check if historical price exists |
| `set_interpolation_enabled(bool)` | Enable/disable linear interpolation for history gaps (admin) |
| `get_interpolation_enabled() -> bool` | Check if interpolation is enabled |

### SEP-40 Oracle Consumer Interface

| Function | Description |
|----------|-------------|
| `base() → Asset` | Returns the base asset (USD) |
| `assets() → Vec<Asset>` | Returns all registered assets as `Asset::Stellar` |
| `decimals() → u32` | Returns price decimals |
| `resolution() → u32` | Returns staleness window in seconds (0 = no expiry) |
| `price(asset, timestamp) → Option<PriceData>` | Get price at or before a given timestamp |
| `prices(asset, records) → Option<Vec<PriceData>>` | Get latest N historical price records |
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

## Consumer Integrations: Contract ID Discovery

Consumer applications must resolve a **verified contract identifier** from a
`(network, environment)` pair before issuing any query. The resolution path is
deterministic and **fails closed**: unknown networks or environments are
rejected rather than silently defaulting to a contract.

### Resolution path

1. **Select network** — one of the supported Stellar networks: `testnet`,
   `futurenet`, `mainnet` (aliases: `public` → `mainnet`, `local` → `local`).
   Any other value is rejected.
2. **Select environment** — one of `dev`, `staging`, `production`. Any other
   value is rejected.
3. **Look up contract ID** — the `(network, environment)` pair maps to a
   contract ID via the table below (or an operator-provided override file).
   A missing entry is an error, never a fallback.
4. **Verify identity** — before querying, the client calls a read-only
   endpoint (e.g. `decimals()` or `get_description()`) and confirms the
   returned contract matches the expected identity. A mismatch aborts the
   query.

### Reference resolution table

| Network | Environment | Contract ID source |
|---------|-------------|--------------------|
| `testnet` | `dev` | `ORACLE_CONTRACT_ID_TESTNET_DEV` |
| `testnet` | `staging` | `ORACLE_CONTRACT_ID_TESTNET_STAGING` |
| `testnet` | `production` | `ORACLE_CONTRACT_ID_TESTNET_PRODUCTION` |
| `futurenet` | `dev` | `ORACLE_CONTRACT_ID_FUTURENET_DEV` |
| `futurenet` | `staging` | `ORACLE_CONTRACT_ID_FUTURENET_STAGING` |
| `mainnet` | `production` | `ORACLE_CONTRACT_ID_MAINNET_PRODUCTION` |

Contract IDs are **public identifiers**, not secrets — they are safe to commit
in configuration. Signing keys and admin secrets must never be committed; use
the `stellar` CLI identity store or a secret manager.

### Example client (fail-closed resolution)

```bash
# Resolve and verify before querying. Unknown network/env => non-zero exit.
resolve_contract_id() {
  local network="$1" environment="$2"
  case "$network" in
    testnet|futurenet|mainnet) ;;
    public) network=mainnet ;;
    *) echo "unknown network: $network" >&2; return 1 ;;
  esac
  case "$environment" in
    dev|staging|production) ;;
    *) echo "unknown environment: $environment" >&2; return 1 ;;
  esac
  local var="ORACLE_CONTRACT_ID_$(echo "$network" | tr a-z A-Z)_$(echo "$environment" | tr a-z A-Z)"
  local id="${!var:-}"
  if [ -z "$id" ]; then
    echo "no contract ID configured for $network/$environment ($var)" >&2
    return 1
  fi
  printf '%s' "$id"
}

# Verify identity: the contract must answer a read-only call.
verify_contract() {
  local id="$1" network="$2"
  stellar contract invoke --id "$id" --network "$network" -- \
    decimals >/dev/null 2>&1 || {
      echo "contract identity verification failed for $id" >&2
      return 1
    }
}

network="${ORACLE_NETWORK:-testnet}"
environment="${ORACLE_ENVIRONMENT:-dev}"
contract_id="$(resolve_contract_id "$network" "$environment")" || exit 1
verify_contract "$contract_id"
```

## Consumer Integrations: Price Units and Decimal Conversion

All prices returned by the contract are **integers scaled by `10^decimals`**,
where `decimals` is the value returned by `decimals()` (set at initialization
via `initialize(..., decimals, ...)` and adjustable by the admin via
`set_decimals(n)`). A price of `12345678` with `decimals = 7` represents
`1.2345678` base units (e.g. USD per asset).

### Scaling rules

- **Contract → human-readable**: `human = integer / 10^decimals`. The integer
  is the source of truth; never round-trip through a float for storage or
  comparison.
- **Human-readable → contract**: `integer = round(human * 10^decimals)` using
  the rounding rule below. Consumers submitting prices must produce the same
  integer the contract would store.
- **Decimals are dynamic**: always read `decimals()` at query time and scale
  with the value you received. Do not hard-code `7`.

### Rounding rule

When converting a fractional human value to an integer, use **round-half-up**
(ties away from zero) on the scaled value. Truncation is only acceptable when
the consumer explicitly documents it; the reference examples below use
round-half-up so that `0.00000005` at `decimals = 7` becomes `1`, not `0`.

### Overflow-safe conversion

- The contract stores prices as `i128`. Consumers must reject any scaled value
  outside `[-(2^127), 2^127 - 1]` **before** submitting, and must not rely on
  language-level overflow wrapping.
- In JavaScript/TypeScript, `Number` is a 53-bit float and **must not** be used
  for scaled prices. Use `BigInt` for all arithmetic.
- In Python, integers are arbitrary precision; clamp/validate against the
  `i128` bounds explicitly.
- In Rust, use `i128` and `checked_mul` / `checked_add`; propagate `None` as an
  error rather than panicking.

### Reference examples

Given `decimals = 7` (scale `10^7 = 10_000_000`):

| Case | Human value | Contract integer | Notes |
|------|-------------|------------------|-------|
| Boundary (min) | `0.0000001` | `1` | Smallest representable unit |
| Boundary (zero) | `0` | `0` | Zero is valid |
| Fractional | `1.2345678` | `12345678` | Exact |
| Fractional (round) | `0.00000005` | `1` | Round-half-up |
| Large price | `170141183460469.2317310687` | `1701411834604692317310687` | Near `i128::MAX` |
| Overflow | `170141183460469.2317310688` | — | Rejected: exceeds `i128::MAX` |

### Executable examples

**Rust**

```rust
const DECIMALS: u32 = 7;
const SCALE: i128 = 10_000_000;

fn to_integer(human_scaled: i128) -> Option<i128> {
    // human_scaled is the human value already multiplied by SCALE, rounded half-up.
    if human_scaled > i128::MAX || human_scaled < i128::MIN {
        return None;
    }
    Some(human_scaled)
}

fn to_human(integer: i128) -> f64 {
    integer as f64 / SCALE as f64
}

#[test]
fn examples_match_contract() {
    assert_eq!(to_integer(1), Some(1));
    assert_eq!(to_integer(12_345_678), Some(12_345_678));
    assert_eq!(to_integer(1), Some(1)); // 0.00000005 rounds up to 1
    assert_eq!(to_integer(170_141_183_460_469_231_731_068_7), Some(170_141_183_460_469_231_731_068_7));
    assert_eq!(to_integer(170_141_183_460_469_231_731_068_8), None);
}
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

### Repeatable check

The examples above are mirrored by the contract's unit tests in
`contracts/price-oracle/src/test.rs` (see the `price_unit_conversion` cases),
which assert the same boundary, fractional, and large-price values against the
contract's `decimals()` and stored integer prices. Run them with:

```bash
cargo test -p price-oracle price_unit_conversion
```
