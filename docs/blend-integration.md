# Blend Protocol Integration Guide (#858)

This guide explains how to wire a [Blend](https://docs.blend.capital) lending pool to
this oracle on **testnet**, how to configure the pool's risk parameters against the
oracle's own guarantees, and what a pool sees when a price goes **stale** or the
oracle is **paused**.

Everything below is stated against the contract entry points documented in
[`abi.md`](./abi.md) and the error codes in [`error-codes.md`](./error-codes.md).

---

## 1. What Blend needs from an oracle

Blend's pool contract prices collateral and liabilities through a SEP-40 style
oracle. For each reserve asset the pool needs:

| Need | Oracle entry point | Notes |
|---|---|---|
| Current price | `lastprice(asset)` | Returns `None` when unregistered, zero, or older than the asset resolution. |
| Price with an explicit freshness budget | `get_price(asset, max_age)` | Returns `None` when `max_age > 0` and the aggregate is older than `max_age` seconds. |
| Price + confidence | `get_price_with_confidence(asset)` | Use the confidence to widen liquidation thresholds. |
| Historical price | `price(asset, timestamp)` | For backtesting risk parameters. |
| Decimals | `decimals()` | Blend scales oracle output to the reserve's token decimals. |
| Resolution | `resolution()` | The contract-wide staleness window; per-asset overrides win. |

The pool must treat **`None` as "no price"**, never as zero. A zero price would
make every position instantly liquidatable.

---

## 2. Testnet demo pool configuration

The demo pool is a single-asset-collateral / single-asset-liability pool on
testnet, using the oracle deployment from
[`deployment.md`](./deployment.md). Record the deployed addresses here when the
pool is created:

| Component | Testnet address | Notes |
|---|---|---|
| Oracle contract | `TBD` | Deployed from `main` at the commit recorded in the release notes. |
| Blend pool (demo) | `TBD` | Created with `blend-utils` / the Blend UI against the testnet pool factory. |
| Collateral reserve | `TBD` | e.g. `USDC:GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5` |
| Liability reserve | `TBD` | e.g. `XLM` (native SAC) |

### Oracle-side setup (admin)

```bash
# 1. Register the assets the pool will price.
stellar contract invoke --id $ORACLE -- register_asset --asset $USDC
stellar contract invoke --id $ORACLE -- register_asset --asset $XLM

# 2. Set a per-asset resolution (staleness window) that matches the pool's
#    risk appetite. 300s is a reasonable testnet default for stablecoins.
stellar contract invoke --id $ORACLE -- set_asset_resolution --asset $USDC --resolution 300

# 3. Register at least `min_sources` independent sources per asset and let them
#    submit. The pool cannot borrow against an asset that never reaches quorum.
```

### Pool-side risk parameters (demo values)

| Parameter | Value | Why |
|---|---|---|
| `max_age` passed to `get_price` | `300` s | Equal to the asset resolution, so the pool and the oracle agree on when a price is dead. |
| Collateral factor (USDC) | 0.90 | Stablecoin, high confidence. |
| Collateral factor (XLM) | 0.75 | Volatile, wider confidence band. |
| Liquidation threshold | collateral factor + 0.05 | Leaves headroom for the confidence band. |
| `min_sources` (oracle policy) | 3 | Matches the pool's assumption that a price is a quorum, not a single feed. |

Keep `max_age` **greater than or equal to** the oracle's resolution. If the pool
asks for a shorter window than the oracle enforces, the pool will see `None`
purely because of its own parameter, which is confusing during an incident.

---

## 3. Staleness from the pool's perspective

The oracle has three independent freshness gates. The pool sees the *first* one
that fires:

1. **`max_age` (caller-supplied).** `get_price(asset, max_age)` returns `None`
   and emits `PriceStaleEvent` when `timestamp + max_age < ledger_time`.
2. **Resolution (contract/asset-supplied).** `lastprice` and `get_price` both
   return `None` when `timestamp + resolution < ledger_time`. `lastprice` has no
   caller-supplied window, so resolution is the only gate it applies.
3. **Zero placeholder.** A v2-migrated asset with no aggregate yet stores a zero
   price; both entry points return `None` rather than serving `0`.

What the pool should do on `None`:

- **Do not liquidate on a missing price.** Skip the reserve for that ledger.
- **Do not accrue interest** against a stale collateral price; freeze the
  reserve's risk parameters until a fresh price arrives.
- **Surface the reason.** `PriceStaleEvent` is emitted on the stale path, so an
  indexer can distinguish "stale" from "never priced" (no event, `None` from the
  zero-placeholder path).

A frozen price (`freeze_price`) is *not* stale by itself: `get_price` keeps
serving the frozen value, and only reports it as stale once it ages past the
asset resolution. The pool therefore keeps operating on a frozen price until the
resolution window expires — which is the intended behaviour during a source
outage.

---

## 4. Pause behaviour from the pool's perspective

Two different pauses exist and they behave differently:

| Pause | Set by | Effect on the pool |
|---|---|---|
| **Contract pause** (`pause`) | admin | Price *submission* and aggregation are blocked (`ContractPaused`, code 12). Reads of already-aggregated prices still succeed until they age out. |
| **Asset pause** (`pause_asset`) | admin | The asset's circuit breaker is tripped; `get_price` serves the TWAP/last-aggregate fallback instead of the live aggregate. |
| **Asset disable** (`disable_asset`) | admin | `get_price` panics with `AssetDisabled`; historical prices remain queryable via `price(asset, timestamp)`. |

Pool integration rules:

- **Wrap reads in a try/catch.** `get_price` can *panic* (`AssetDisabled`,
  `AssetNotRegistered`), not just return `None`. A panicking oracle call must not
  brick the pool's `submit`/`liquidate` path — catch it and treat the reserve as
  unpriced.
- **On contract pause, keep serving the last good price** until it goes stale.
  Do not zero out positions.
- **On asset disable, stop accepting the asset as new collateral** and stop
  liquidations that depend on its price; existing positions stay open.
- **On asset pause (breaker tripped), expect a fallback price.** It carries
  `num_sources = 0`; the pool should treat it as lower-confidence and apply a
  haircut to the collateral factor.

---

## 5. Demo walkthrough

1. Deploy the oracle to testnet and register `USDC` + `XLM` (section 2).
2. Create the demo pool and point its oracle at the deployed contract.
3. Submit prices from three testnet sources until `get_price(USDC, 300)` returns
   `Some`.
4. Supply USDC, borrow XLM. Confirm the borrow succeeds.
5. Stop submitting USDC prices. After 300 s, `get_price(USDC, 300)` returns
   `None` and the pool refuses new borrows against USDC collateral.
6. `pause_asset(USDC)` and confirm the pool reads the fallback price instead of
   the live aggregate.
7. `disable_asset(USDC)` and confirm the pool catches `AssetDisabled` and keeps
   operating on the other reserve.

---

## 6. Open items

- Publish the deployed testnet pool address and a short screen recording of the
  walkthrough in section 5.
- Add the pool address to [`deployment.md`](./deployment.md) once it is stable.
