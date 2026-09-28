# Robust Outlier Filtering (MAD / IQR) — #491

A single wrong source value used to flow straight into the median. This feature applies a
**robust** estimator to the round's candidate prices before aggregation and removes values that
are statistical outliers, with per-asset configurable sensitivity and an auditable record of
every exclusion.

Code: `contracts/price-oracle/src/outlier_filter.rs`. Config types: `types::OutlierConfig`,
`types::OutlierExclusion`. Documentation: `docs/outlier-filtering.md`.

## Why robust statistics

A mean/standard-deviation z-score is *not* usable to find an outlier: the outlier itself drags
the mean and inflates the standard deviation, hiding its own excess. The median and the MAD are
built from order statistics, so a single wild value barely moves them — the estimator stays
honest while judging the value it is scoring.

## Detectors

`detector` in `OutlierConfig` selects the estimator:

| `detector` | Estimator | Score of `x` | Excluded when |
|---|---|---|---|
| `0` | none (default) | — | never; every candidate is kept |
| `1` | MAD | `\|x - med\| * 10_000 / scaled_mad` | `score_bps >= sensitivity_bps` |
| `2` | IQR | distance past the quartile fence, in bps of the IQR | past the fence |

`med` is the median of the round; `mad` the median absolute deviation; `scaled_mad = mad * 1.4826`
(the consistency factor that makes a MAD a normal-equivalent sigma). For MAD, the default
sensitivity `35_000` is the Iglewicz–Hoaglin modified-z threshold of 3.5.

For IQR, with nearest-rank quartiles `q1`, `q3` and `IQR = q3 - q1`, the fence is
`q1 - k*IQR` / `q3 + k*IQR` with `k = sensitivity_bps / 10_000`; the default `15_000` reproduces
the classic 1.5 x IQR rule.

## Guards

The failure mode of any pre-filter is removing legitimate values during a fast market, so the
filter is bounded on three axes:

- **Source-count floor.** Both estimators are unstable on tiny samples — a set of four prices has
  two quartiles that collapse onto individual sources. Filtering is skipped entirely when the
  round has fewer than `min_sources` candidates, and `min_sources` itself cannot be set below
  `MIN_SOURCES_FLOOR = 4`. A round below the floor is aggregated unfiltered.
- **Scale collapse.** If every candidate equals the centre, the scale is `0` and no relative
  score exists. Rather than divide by zero, the filter falls back to an *absolute* floor: the
  value must deviate by at least `sensitivity_bps / 10_000` of the centre. A set of identical
  clean prices therefore survives intact, while a value orders of magnitude away from a
  perfectly tight set is still removed. Exclusions decided this way are published with
  `detector: 0` and `scale: 0` so a consumer can tell the two paths apart.
- **Majority guard.** At most half of a round's candidates may be dropped. A round that would
  lose more than half is aggregated unfiltered, so a fractured round degrades to the unfiltered
  median rather than to a single-source aggregate. The caller re-checks quorum on the survivors
  regardless, so filtering can only reduce the contributing set, never fabricate one.

## Configuration

```rust
client.set_outlier_config(&asset, &Some(OutlierConfig {
    detector: 1,        // MAD
    sensitivity_bps: 35_000,
    min_sources: 5,
}));
client.set_outlier_config(&asset, &None); // clear the override
```

Bounds are validated on write (`InvalidConfiguration` otherwise): `detector <= 2`,
`sensitivity_bps <= 1_000_000`, `min_sources` in `4..=64`. A stored config is therefore always
usable, and an invalid write panics rather than silently falling back to a laxer setting. The
filter is **off by default** — an asset must opt in.

## Consistency downstream

The pre-filter runs *before* the policy deviation filter and produces a keep-mask that the
aggregation applies to prices, volumes, weights, sources and submission ledgers together. Every
downstream statistic is therefore computed on the same post-filter set: the median, the
confidence band, the provenance record, the latency accounting and the disagreement index. There
is no path by which an excluded price reaches a published statistic.

## Auditability

Every exclusion is published in `OutlierExcludedEvent` and the full round's exclusion set is
queryable with `get_outlier_exclusions(asset)`. Each entry carries:

- `source` and `price` — who said what was dropped;
- `score_bps` — the robust score that crossed the threshold;
- `sensitivity_bps` — the threshold in force;
- `center` and `scale` — the median/MAD (or quartile centre/IQR) the score was measured against.

That is enough to recompute the decision off-chain and confirm the on-chain one.

## Operator notes

- Start with `detector: 2` (IQR, 1.5 rule) and a `min_sources` at or above your quorum; it is the
  more conservative of the two.
- A source that is repeatedly excluded is worth investigating — the filter is a symptom
  detector, not a source punishment. Nothing here slashes or suspends anyone.
- If legitimate volatility is being filtered, raise `sensitivity_bps` or `min_sources`. The
  alternative, disabling the filter, returns you to the unfiltered median.
- `OutlierConfigChangedEvent` records every configuration change.


Code: `contracts/price-oracle/src/outlier_filter.rs`. Config types: `types::OutlierConfig`,
`types::OutlierExclusion`. Documentation: `docs/outlier-filtering.md`.
