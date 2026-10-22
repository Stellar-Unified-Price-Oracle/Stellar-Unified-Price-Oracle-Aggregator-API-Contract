# Two-tier price bounds (#484)

A single bounds mechanism forces a choice between **rejecting** an aggregate
(halt the feed) and **clamping** it (silently distort the published value).
The aggregator now splits that into two explicitly configured tiers per asset,
so the feed can degrade *loudly* near its limits while still failing *closed*
at the hard limits.

```
hard_min  <=  soft_min  <=  published  <=  soft_max  <=  hard_max
```

| Tier | Value outside the band | Published? | Flagged? |
|---|---|---|---|
| soft | clamped to the bound | yes | `clamped = true` + reason code |
| hard | rejected outright | **no** | `rejected = true` + reason code |

## Configuration

```rust
client.set_price_bounds_tier(&asset, &BoundsTier {
    soft_min: 90,
    soft_max: 110,
    hard_min: 50,
    hard_max: 200,
});
```

Ordering is **validated on write**: `0 < hard_min <= soft_min <= soft_max <=
hard_max`. A violation panics with `ErrorCode::InvalidBoundOrdering` (156) and
is never stored, so a configured tier is always well-formed. Assets with no
tier configured are unaffected — the value passes through unchanged.

Read back with `get_price_bounds_tier`; remove with `clear_price_bounds_tier`.

## Reason codes

`get_price_bound_status(asset)` returns a `BoundStatus`:

| Field | Meaning |
|---|---|
| `raw_price` | The aggregate before any clamping |
| `price` | The published value (clamped when `clamped` is true) |
| `clamped` | The value was clamped to a soft bound |
| `rejected` | The raw aggregate was outside the hard bounds |
| `reason_code` | See table below |
| `ledger` | Ledger of the decision |

| `reason_code` | `BoundReason` | Meaning |
|---|---|---|
| 0 | `InBounds` | Inside the soft band; publish as-is |
| 1 | `ClampedToSoftMin` | Clamped **up** to `soft_min` |
| 2 | `ClampedToSoftMax` | Clamped **down** to `soft_max` |
| 3 | `RejectedBelowHardMin` | Below `hard_min`; nothing published |
| 4 | `RejectedAboveHardMax` | Above `hard_max`; nothing published |

## Events

* `PriceClampedEvent` — topics `asset`; fields `raw_price`, `clamped_price`,
  `reason_code`, `ledger`.
* `PriceBoundRejectedEvent` — topics `asset`; fields `raw_price`,
  `reason_code`, `last_published_price`, `ledger`.

The two are distinguishable from events alone: clamp and reject use different
event names, and a rejection additionally carries the last aggregate that
stayed live.

## Consumer guidance for clamped values

A clamped value is **never silent**, and a consumer must treat it as a
degraded reading rather than as market data:

1. **Always read the status.** Before using an aggregate for an asset with a
   configured tier, call `get_price_bound_status(asset)` and check `clamped`
   and `reason_code`. A value of `0` is the only in-bounds code.
2. **Do not re-aggregate a clamped value.** A clamped price has already been
   distorted toward a bound; feeding it into a further median, TWAP or
   cross-asset calculation compounds the distortion. Use `raw_price` for
   analysis instead — it is the unmodified aggregate, and it is only ever
   readable through `BoundStatus`, never through `get_price`.
3. **Prefer failing over clamping.** For assets where a wrong price is more
   costly than no price, set `soft_min`/`soft_max` equal to
   `hard_min`/`hard_max`; the soft tier then never activates and every
   violation rejects.
4. **Widen the tier, don't silence the flag.** Setting a very wide soft band
   makes clamping rare, but the flag stays live so a clamp is still visible
   when it happens.
5. **A rejected aggregate is not a price of zero.** When `rejected == true`
   the previously published aggregate remains live and `get_price` keeps
   returning it. Consumers that require a *fresh* value should additionally
   check the aggregate `timestamp`/`version`, because the live value is by
   definition the last accepted one.

## Interaction with corrections (#486)

`correct_price` respects the hard tier: a correction value outside
`[hard_min, hard_max]` is rejected with
`ErrorCode::AggregateRejectedByBounds` (160). A correction is an audited
republication, not a way around the hard limits.
