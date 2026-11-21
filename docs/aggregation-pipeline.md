# How aggregation works — the canonical pipeline

This is the one document to read to understand **how a price is produced**. It
walks the pipeline in execution order, gives the default configuration at each
step, and links the detailed document for every stage. Where a stage is *not*
compiled into the shipped WASM, it is marked **[not in shipped WASM]**.

Code: `contracts/price-oracle/src/prices.rs` (`submit_prices` →
`aggregate_asset` → `aggregate_asset_mode`). Math specification:
[`aggregation-semantics.md`](aggregation-semantics.md).

```
submit_price ──► validation ──► candidate set ──► outlier filter ──► deviation filter
   ──► quorum ──► median / method ──► bounds ──► breaker ──► write + events
```

## 0. Submission

`submit_price(source, asset, price, timestamp)` (and the batch
`submit_prices`) records a `PriceEntry` per `(asset, source)`. The source must be
authorised; the entry stores the price, optional volume, the source-supplied
timestamp and the **ledger timestamp it was recorded at**.

Defaults: no volume, no weight override. Details:
[`source-onboarding.md`](source-onboarding.md), [`signed-price-adapters.md`](signed-price-adapters.md).

## 1. Validation

Before a submission can be counted:

| Check | Default | Doc |
|---|---|---|
| Source is registered and active | required | [`source-lifecycle.md`](source-lifecycle.md) |
| Price is positive and within `i128` scaling | required | [`price-scaling.md`](price-scaling.md) |
| Heartbeat / min-interval compliance | off unless configured | [`source-reliability-score.md`](source-reliability-score.md) |
| Correlation flag (source flagged for collusion) | excluded when flagged | [`source-diversity.md`](source-diversity.md) |
| Data-quality screen | off unless a round is configured | [`data-quality-pipeline.md`](data-quality-pipeline.md) |
| Probation cap on counted sources | `PROBATION_MAX_COUNTED` | [`source-lifecycle.md`](source-lifecycle.md) |

## 2. Candidate set and freshness

Each stored submission is admitted to the round unless it is stale. Freshness is
`env.ledger().timestamp() - entry.ledger_timestamp` — never wall-clock, never a
source-supplied value.

Window resolution: **per-asset window > risk-tier window > global default**.
Stale entries are excluded from quorum and reported as `stale` in
`get_freshness_status`; a `FreshnessFilteredEvent` is emitted on every gated
aggregation. Default: no global window set (no limit) until an admin sets one.
Details: [`freshness-quorum.md`](freshness-quorum.md), [`ttl-boundaries.md`](ttl-boundaries.md).

## 3. Outlier filter (robust pre-filter)

Runs **before** the deviation filter so a gross outlier is removed by an
estimator that is itself robust to it.

| `detector` | Estimator | Default sensitivity |
|---|---|---|
| `0` | none — **default** | — |
| `1` | MAD (Iglewicz–Hoaglin modified z) | `35_000` bps (z = 3.5) |
| `2` | IQR fence | `15_000` bps (k = 1.5) |

Guards bound how much the filter may remove in one round. Every exclusion is
recorded and readable. Details: [`outlier-filtering.md`](outlier-filtering.md).

## 4. Deviation filter

When the effective policy sets `max_deviation_bps`, values further than that from
the round's centre are masked out. Default: **off** (`0`). Details:
[`aggregation-hardening.md`](aggregation-hardening.md).

## 5. Quorum

`contributing_sources` (survivors of steps 2–4) must reach `min_sources`.
Default: `get_min_sources_required` (global default, admin-settable). Below
quorum the asset publishes nothing and `SourcesInsufficientEvent` is emitted.
Per-asset and per-class overrides resolve **asset → class → global**. Details:
[`freshness-quorum.md`](freshness-quorum.md), [`adaptive-quorum.md`](adaptive-quorum.md).

## 6. Median / aggregation method

`policy.method` selects the estimator; the default is the plain median
(`get_aggregation_method`).

| `method` | Estimator | Notes |
|---|---|---|
| `0` | median | default; even count floors to the lower middle |
| `1` | mean | truncated toward zero |
| `2` | trimmed mean | `trim_percent` default `0` |
| `3` | weighted median | weights clamped to `>= 1` |
| `4` | freshness-weighted median | curve default `window = 300`, `min_weight = 100` |

Method `4` on the secondary paths (candidate preview, `trigger_aggregation`)
falls back to the plain median. Exact rounding rules:
[`aggregation-semantics.md`](aggregation-semantics.md). Method selection and
per-asset policy: [`aggregation-hardening.md`](aggregation-hardening.md),
[`influence-caps.md`](influence-caps.md).

## 7. Bounds

Two-tier bounds, applied only when configured for the asset:

```
hard_min <= soft_min <= published <= soft_max <= hard_max
```

A soft violation **clamps** and flags (`clamped = true` + reason code); a hard
violation **rejects** — nothing is published, nothing enters history, the
previous aggregate stays live. Default: **no tier configured**, value passes
through unchanged. Details: [`price-bounds-tiers.md`](price-bounds-tiers.md).

## 8. Circuit breaker

The deviation breaker compares the new aggregate with the previous one. Defaults:
trip at 20 %, clear at 10 %, settle for 10 consecutive ledgers, escalate after
1 000 ledgers, auto re-arm on. `clear_breaker(asset)` is admin-only and bypasses
the settle streak. Details: [`breaker-hysteresis.md`](breaker-hysteresis.md),
[`blackout-windows.md`](blackout-windows.md).

## 9. Write and events

On success the contract writes the `AggregatePrice`, appends a
`PriceHistoryEntry` to the asset's history tier (temporary by default), records
provenance, disagreement and latency samples, and emits the round's events
(`PriceUpdated`, `ConfidenceBandEvent`, `WeightedAggregationEvent`,
`InfluenceCapAppliedEvent`, `FreshnessFilteredEvent`, `PriceClampedEvent`, …).
Details: [`event-schema-registry.md`](event-schema-registry.md),
[`provenance.md`](provenance.md), [`storage-tiers.md`](storage-tiers.md).

## Not compiled into the shipped WASM

These stages exist in the crate but are **not** part of the shipped contract
build; a consumer reading a price on mainnet is not affected by them:

- **[not in shipped WASM]** `core_pricing.rs` — the `&[i128]` implementation used
  by fuzz targets and differential tests. The published path uses the
  `storage.rs` wrappers; the two are asserted equal by `reference_diff_tests`.
- **[not in shipped WASM]** `reference_pricing.rs` — the independent reference
  implementation used only by the differential suite.
- **[not in shipped WASM]** the `fuzz/` targets and the proptest regressions.

Everything in steps 0–9 above *is* compiled in; the defaults listed are the
values in force when no admin has configured anything.
