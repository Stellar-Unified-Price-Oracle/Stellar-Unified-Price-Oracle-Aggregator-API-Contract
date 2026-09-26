# Aggregate Confidence Bands (#476)

Every aggregate is published with an interquartile **confidence band** so a
consumer can tell a tight consensus from a wide disagreement.

- Query: `get_confidence_band(asset) -> Option<ConfidenceBand>` (current submissions).
- Event: `ConfidenceBandEvent { asset, price, band }`, emitted on every aggregation.

```text
ConfidenceBand { lower, upper, median, num_sources, decimals, low_confidence }
```

## Computation

For the `n` contributing prices sorted ascending (`s[0..n]`), nearest-rank quartiles:

```text
lower = s[floor((n - 1) / 4)]
upper = s[ceil(3 * (n - 1) / 4)]
```

Properties (covered by `issues_467_468_475_476_tests`):

| Property | Why |
|---|---|
| `lower <= median <= upper` | the median indices sit between the quartile indices |
| `lower == upper` when all sources agree | both bounds are contributed prices |
| Same units/decimals as the aggregate; scales exactly by `10^k` | bounds are selected, never computed |
| A moderate source (inside the band) keeps the new band inside the old one | bounded tightening only |
| An outlier never pulls the far side of the band inward | no implausible narrowing |

`low_confidence` is `true` when `num_sources < max(min_sources_required, 4)`:
with so few points each quartile *is* a single source.

## Interpreting the band

- **Relative width** `(upper - lower) / median` is the dispersion signal. Compare
  it with the asset's normal volatility, not an absolute number.
- **Narrow band** — sources agree; the point price is reliable.
- **Wide band** — sources disagree (fragmented liquidity, a lagging or faulty
  source, or a fast market). Risk-sensitive consumers should widen safety
  margins (e.g. lower LTV, larger liquidation buffer), price conservatively
  using `lower`/`upper` as appropriate, or pause actions until it tightens.
- **`low_confidence`** — treat the band as indicative only; do not rely on its
  width to be small.
- The band describes source *dispersion*, not statistical significance.
