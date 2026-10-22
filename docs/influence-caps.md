# Source Influence Caps (#475)

Bounds the share of aggregate influence any single source can exert.

| Item | Value |
|---|---|
| Config | `set_influence_cap(cap_bps)` / `get_influence_cap()` |
| Owner | Contract admin (`require_auth`) |
| Bounds | `1_000..=10_000` bps (10 %–100 %); `10_000` disables the cap |
| Default | `5_000` bps (the 50 % share previously hard-coded) |
| Event | `InfluenceCapAppliedEvent { asset, cap_bps, influence_bps }` |

## Enforcement

- **Weighted path (method 4)** — after freshness weighting, each weight is
  lowered until `w_i / Σw <= cap`. Capping never raises a weight and the sum is
  recomputed from the capped weights, so effective influences always sum to
  ~10 000 bps. `influence_bps` is emitted in the same order as
  `WeightedAggregationEvent.weights`, so it can be reconstructed from that event.
- **Unweighted paths (median, …)** — one source, one vote (`1/n`). The median
  also limits a single source to moving the result no further than its
  neighbouring order statistic, so no extra enforcement is needed.

## Quorum fallback

The cap never drops a source, so a satisfiable quorum stays satisfiable.

- `n < 3`: cap not applied (a two-source weighted median interpolates anyway).
- `n * cap_bps < 10_000`: no assignment can satisfy the cap; all weights fall
  back to equal (`1/n`, the smallest achievable maximum share).
- Integer rounding that stalls convergence also falls back to equal weights.

## Collusion residual bound

`k` colluding sources together hold at most `min(10_000, k * cap_bps)` bps.
When `k * cap_bps < 5_000` they cannot form a weighted majority and cannot
choose the weighted median alone. Splitting one operator across several
source identities multiplies `k`; source onboarding/diversity checks
(`docs/source-diversity.md`) are the control for that. Tested with N = 10 and
k = 1..4 colluders in `colluding_sources_bounded_by_k_times_cap`.
