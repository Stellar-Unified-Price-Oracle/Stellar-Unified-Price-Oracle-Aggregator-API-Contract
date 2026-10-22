# Aggregate Spread Indicator — Uncertainty and Confidence Semantics (#713)

Module: `contracts/price-oracle/src/spread_indicator.rs`

## Purpose

Every successful price aggregation stores a `SpreadIndicator` for the asset.
This gives consumers a quantified, machine-readable view of how much the
contributing oracle sources disagreed on price — without requiring the consumer
to fetch all individual source submissions.

## API

| Endpoint | Description |
|---|---|
| `get_spread_indicator(asset) → Option<SpreadIndicator>` | Latest indicator. `None` if no aggregation yet. |

### `SpreadIndicator` struct

```rust
pub struct SpreadIndicator {
    pub price: i128,                         // Published aggregate (median)
    pub q1: i128,                            // Lower quartile of contributing prices
    pub q3: i128,                            // Upper quartile of contributing prices
    pub iqr: i128,                           // Interquartile range = q3 - q1
    pub relative_spread_bps: u32,            // IQR * 10_000 / price (basis points)
    pub spread_interpretation: SpreadInterpretation,
    pub num_sources: u32,
    pub decimals: u32,
    pub low_confidence: bool,                // true when num_sources < 3
    pub ledger: u32,
    pub timestamp: u64,
}

pub enum SpreadInterpretation {
    Tight   = 0,   // < 50 bps
    Normal  = 1,   // 50–200 bps
    Wide    = 2,   // 200–500 bps
    VeryWide = 3,  // > 500 bps or low_confidence
}
```

## Statistical meaning

The spread indicator is based on the **interquartile range (IQR)** of the
contributing source prices. For `n` sorted contributing prices `s[0..n]`:

```
Q1  = s[floor((n-1)/4)]
Q3  = s[ceil(3*(n-1)/4)]
IQR = Q3 - Q1
relative_spread_bps = IQR * 10_000 / median
```

The IQR is a robust dispersion measure: extreme outliers (the top/bottom 25%)
do not affect it, making it resistant to a single rogue source dramatically
widening the reported spread.

`relative_spread_bps` is **scale-invariant**: the same disagreement produces
the same bps value regardless of whether prices use 8 or 18 decimal places.

## What this does NOT mean

> **This is a dispersion metric, not a probability.**

`relative_spread_bps` measures how much sources disagree with each other, not
the probability that the published price is within any particular tolerance of
a true value.

| Claim | Correct? |
|---|---|
| "A tight spread means sources agree" | ✅ Yes — this is exactly what it measures |
| "A tight spread means the price is correct" | ❌ No — sources can agree on a wrong price |
| "relative_spread_bps is the probability of being within X% of true price" | ❌ No — this has no probabilistic interpretation |
| "A wide spread means the aggregate should be rejected" | ❌ Not necessarily — wide spreads are common in illiquid or fast-moving markets |

## Classification thresholds

| Band | `relative_spread_bps` | Meaning | Suggested action |
|---|---|---|---|
| `Tight` | < 50 bps (0.5%) | Sources closely agree | Use aggregate directly |
| `Normal` | 50–200 bps | Typical market disagreement | Use with standard risk margins |
| `Wide` | 200–500 bps | Notable divergence | Widen safety margins; consider secondary validation |
| `VeryWide` | > 500 bps OR `low_confidence` | High uncertainty | Treat as indicative; avoid risk-sensitive operations |

These thresholds are **advisory**. Risk-sensitive applications (e.g. lending
protocols) SHOULD define their own acceptance criteria appropriate to their
liquidation tolerance and collateral volatility.

## `low_confidence` flag

When `num_sources < 3`, Q1 and Q3 both resolve to individual data points and
the width of the IQR is not statistically meaningful. `low_confidence = true`
signals this. Consumers MUST NOT rely on the spread width when `low_confidence`
is set.

## Relationship to `ConfidenceBand`

Both `SpreadIndicator` (this feature) and `ConfidenceBand` (#476) use IQR
quartiles. They differ in purpose:

| | `SpreadIndicator` | `ConfidenceBand` |
|---|---|---|
| Endpoint | `get_spread_indicator` | `get_confidence_band` |
| Adds | classification, relative bps, documentation | lower/upper absolute bounds |
| Primary use | Consumer decision-making guidance | Collateral / liquidation buffer sizing |
| Event | `spread` (manual) | `ConfidenceBandEvent` |

Consumers needing absolute price bounds should use `ConfidenceBand`. Consumers
needing a single "how confident should I be?" signal should use `SpreadIndicator`.

## Limitations

- Spread reflects the **current round's** contributing prices only, not a
  rolling average across rounds. A single anomalous round produces a temporary
  wide spread.
- With a `Tight` spread and `low_confidence = true`, there are simply not enough
  independent data points to draw any conclusion about dispersion.
- Spread does not detect a colluding majority that agrees on a manipulated price.
  For manipulation detection, see `docs/drift-detection.md` and
  `docs/disagreement-index.md`.
