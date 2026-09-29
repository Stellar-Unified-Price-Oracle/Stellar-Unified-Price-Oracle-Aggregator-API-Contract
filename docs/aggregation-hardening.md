# Aggregation hardening: policy, freshness weighting, TWAP cardinality

## Per-asset aggregation policy (#474)

`get_effective_policy(asset)` resolves each field independently:
**asset override → asset class → global default**, and returns the layer that
supplied it (`0` global, `1` class, `2` asset).

| Field | Bounds | Global default |
|---|---|---|
| `method` | `0..=4` | `get_aggregation_method` |
| `min_sources` | `1..=100_000` | `get_min_sources_required` |
| `freshness_secs` | `1..=604_800` (unset = no limit) | none |
| `max_deviation_bps` | `1..=10_000` (unset = off) | off |

Out-of-range values are rejected at write time (`InvalidConfiguration`), so a
stored policy can never silently fall back to a laxer layer. Policies are read
only when an aggregation runs; a change never rewrites an aggregate already
computed. Every change emits `AggregationPolicyChangedEvent` with old and new
values (class assignments use `scope = 3` with `old_class`/`new_class`), so the
effective policy is reconstructible from events.

Admin endpoints: `set_asset_policy`, `set_class_policy`, `set_asset_class`
(pass `None` to clear). Freshness and deviation are applied on the main
submission path (`aggregate_asset`); quorum and method are applied on every
aggregation path. Method `4` on the two secondary paths (candidate preview and
`trigger_aggregation`) falls back to the plain median.

## Freshness-weighted median (#472)

Aggregation method `4`. Weight of a submission of age `a` seconds with curve
`(window, min_weight)`:

```
weight = 1000 - (1000 - min_weight) * min(a, window) / window
```

Defaults: `window = 300`, `min_weight = 100`; bounds `window ∈ 1..=86_400`,
`min_weight ∈ 1..=1000` (`1000` = no decay). Configure with
`set_freshness_curve(asset, window_secs, min_weight)`.

* **Bounded influence:** weights lie in `[min_weight, 1000]`; when three or
  more sources contribute, no weight may exceed the sum of the others (max
  50 % share).
* **Equal weights = plain median** (even counts interpolate like
  `compute_median`).
* `get_weighted_aggregate(asset)` returns `raw_median`, `weighted_median` and
  the per-source `weights`. Each method-4 aggregation emits
  `WeightedAggregationEvent` with the capped weights (submission order), so the
  result is reconstructible from events.

**Worked example.** Three sources submit 100 just now, one stale source
submitted 900 ten minutes ago. Weights: `1000, 1000, 1000, 100`. Cumulative
weight passes half of the total (`3100 / 2`) at the second sorted value, so
the weighted median is `100`; the plain median of `100, 100, 100, 900` is also
`100`, but with two fresh (`100`) and two stale (`900`) sources the weighted
median stays `100` while the plain median is `500`.

**Calibration.** Choose `window` near the asset's expected update interval
(sources that update every `T` seconds → `window ≈ 2T`) and keep `min_weight`
at `≥ 10 %` of `1000` so stale-but-valid feeds still count. Lower `min_weight`
for volatile assets; raise it (up to `1000`) for slow, illiquid ones.

*Weighted vs unweighted:* the plain median counts every submission in the
window equally; the weighted median lets recent data dominate while old data
fades, at the cost of reacting faster to a burst of fresh submissions.

## TWAP observation cardinality (#477)

`get_twap_ex(asset, window, method)` returns the TWAP plus:

* `cardinality` — distinct observations (one per ledger with a price change)
  inside the window;
* `max_weight_bps` — the largest single observation's share of the window;
* `concentrated` — `true` when the whole window rests on one observation.

Observations are time-weighted by the ledgers they stay in force, and history
holds one entry per ledger, so repeated submissions in one ledger add no
weight. `set_twap_min_cardinality(n)` (`1..=64`, default `1`) makes `get_twap`
and `get_twap_ex` fail closed with `TwapInsufficientObservations` when a window
has fewer observations; raising it can make illiquid assets unavailable.
A one-ledger push of price `P` in a window of `W` ledgers moves the TWAP by at
most `(P - prev) / W`.
