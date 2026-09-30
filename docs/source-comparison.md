# Cross-Source Comparison Dashboard (#401)

Module: `contracts/price-oracle/src/source_comparison.rs`.

## Setup and data model

Enable per asset with `set_source_comparison(asset, true)`. The dashboard's
data source is `get_source_comparison(asset) -> ComparisonReport`, a pure read
that never mutates a price or a source set.

While enabled, every published round is indexed. A rolling window of the last
16 rounds is kept per asset. Each round records:

* the aggregate;
* each **admitted** source's value (registered, eligible, with a stored
  submission);
* whether that value was **counted** in the published aggregate.

## Metrics (per source, per asset)

| Metric | Method |
|---|---|
| `mean_deviation_bps` | mean of `(price − aggregate) / aggregate` over admitted rounds |
| `direction_persistence_bps` | share of admitted rounds whose deviation has the source's dominant sign |
| `influence_bps` | **Leave-one-out influence**: for each counted round, `|median(counted) − median(counted without source)| / aggregate`, averaged over counted rounds. It is the distance the published value would have moved without this source. It is 0 for a source never counted. |
| `rounds_admitted` / `rounds_counted` / `excluded` | exclusion detection, see below |

### Collusion signal

For every source pair, the report computes the **cosine similarity** of their
deviation vectors over shared rounds:
`Σ da·db / √(Σ da² · Σ db²)`, in bps.

Independent honest sources deviate with uncorrelated signs, so their
similarity is about 0. A coordinated set leans the same way every round, so
its similarity is about 10 000. This holds even when every individual series
stays inside its own deviation bound. The test
`collusion_signal_detects_coordinated_deviation_within_bounds` uses two
colluders at +40 bps, inside a 100 bps bound, among three honest sources.

A pair is flagged when **all** of these hold:

* `shared_rounds >= 4`
* `similarity_bps >= 8 000`
* both sources have `direction_persistence_bps >= 7 000`

### Exclusion detection

A source admitted in at least 4 rounds but counted in fewer than 20 % of them
is flagged `excluded`. This catches sources that are registered but
systematically filtered out. That pattern can be used to narrow the effective
source set until the remaining sources dominate. Covered by
`excluded_source_is_flagged_and_influence_quantified`.

## Thresholds and false positives

| Threshold | Value | False-positive behaviour |
|---|---|---|
| `MIN_ROUNDS` | 4 | Too few rounds make any correlation meaningless. Below 4 rounds, nothing is flagged, so collusion shorter than 4 rounds is missed. |
| `COLLUSION_SIMILARITY_BPS` | 8 000 | Two honest sources sharing an upstream venue, or lagging the same way in a trend, also co-move and **will be flagged**. Treat a flag as a prompt to check source independence (see `docs/source-coverage.md`), not as proof. |
| `PERSISTENCE_BPS` | 7 000 | Filters honest noise, which sits near 50 %. A colluding pair that alternates direction together evades it but must then give up any directional gain. |
| `EXCLUSION_COUNTED_BPS` | 2 000 | A source that is legitimately stale or outlier-filtered most of the time, for example on a thin market, is flagged. That is intended: such a source adds no independence. |
| `WINDOW_ROUNDS` | 16 | Bounds storage and read cost. Anything older is forgotten, so slow coordination spread over more than 16 rounds is invisible here. |

## Rendering

The report is a flat per-source table plus a flagged-pair list. It is designed
to be charted per asset:

* deviation and influence as bar or line series per source;
* collusion pairs as an adjacency list;
* exclusion as a flag column.

For historical trends and tail behaviour, index `get_source_comparison` over
time off-chain, and chart percentiles as well as means.

Automatic removal of a flagged source is out of scope. Use the offboarding
workflow in `docs/source-lifecycle.md`.
