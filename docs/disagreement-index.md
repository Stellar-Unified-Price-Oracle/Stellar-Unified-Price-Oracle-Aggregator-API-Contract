# Pairwise Source Disagreement Index — #494

The confidence band (see `docs/confidence-bands.md`) shows how far the contributing prices spread
around the median, but not the *shape* of that spread. A single dissenting source and a genuine
two-way split produce the same band width, so a consumer cannot tell a healthy-but-noisy round from
a fractured one. This feature adds a first-class disagreement metric that distinguishes them.

Code: `contracts/price-oracle/src/disagreement.rs`. Type: `types::DisagreementIndex`. Query:
`get_disagreement_index(asset)`. Emitted in `DisagreementIndexEvent` on every published aggregate.

## Definition

For the `n` counted prices, let `med` be their median. For every pair `(i, j)`:

```text
d_ij = |p_i - p_j| * 10_000 / med      (basis points)
```

Then:

- `index_bps` — the **median** of the `n(n-1)/2` pairwise deviations;
- `max_bps` — the largest pairwise deviation.

## Why these choices

**Scale invariance.** Every deviation is divided by the median price, so the index is a pure
ratio. The same prices expressed with 8 decimals and with 18 decimals give the same number, and a
$1 pair can be compared with a $100 000 pair without conversion. This is a property test
(`disagreement_index_is_scale_invariant`), not a claim.

**Median of pairs, not a mean.** A lone dissenter inflates only the `n-1` pairs it takes part in.
With five sources, one dissenter touches 4 of 10 pairs, so the *median* pair deviation stays near
the inlier spread while `max_bps` spikes. A broad split inflates most pairs, so `index_bps` rises
with it. **This is why both numbers are reported**: `index_bps` alone would miss a lone dissenter,
and `max_bps` alone cannot tell you whether the round was split or merely had one noisy source.

| Round | `index_bps` | `max_bps` | Reading |
|---|---|---|---|
| 100, 101, 99, 100, 100 (agreement) | low | low | healthy consensus |
| 100, 100, 100, 100, 250 (lone dissent) | low | high | one source to investigate |
| 100, 100, 100, 120, 120 (broad split) | high | high | fractured round; no single culprit |
| all sources equal | `0` | `0` | perfect agreement |

**Few sources.** With `n < 2` there are no pairs and the index is `0` by definition. No
division by zero is possible: the median of one price is that price, and no pair is ever formed.
For `n = 2` exactly one deviation exists, so `index_bps == max_bps` and the index is exactly that
pair's disagreement — correct, but not a consensus signal, so `low_sample` is set for `n < 3`.
Consumers should not alert on a `low_sample` round.

## Rolling baseline

Genuine volatility spikes the index, and **a spike is not an error**. Alerting on an absolute
threshold would fire on every real market move and train operators to ignore it. So the current
value is compared against a rolling median of the last `BASELINE_WINDOW = 16` values of the *same*
asset:

```text
above_baseline = baseline_bps > 0 && index_bps > 2 * baseline_bps
```

The baseline is computed **before** the current value is appended, so a spike never raises its own
reference and cannot become self-normalising. It is reported in both the event and the record, so
a spike during a real move is visible *as a spike* — current and baseline together tell the story —
rather than as a fault.

Storage is bounded: a ring of 16 `u32` values per asset, independent of how long the asset trades.
The ring lives in the *same* ledger entry as the reading itself (`DisagreementRecord`), so a round
writes one key rather than two — a per-round second key is enough on its own to push a wide batch
over the network's 100-entry footprint cap
or how many rounds are aggregated.

## What the index is not

- **It does not slash, penalise or exclude anything.** A high index is a signal for a human; the
  robust pre-filter (#491) is the automated response, and it has its own guards and its own audit
  trail.
- **It is computed on the counted submissions**, i.e. after the robust pre-filter and the
  deviation filter, so an excluded outlier cannot inflate it. A lone dissenter that the pre-filter
  removed simply does not appear.
- **It says nothing about which side is right.** The index measures divergence, not correctness.

## Operator guidance

- Establish a baseline first: a newly listed asset has no history, so `baseline_bps` is `0` and
  `above_baseline` is `false` by construction. Wait for a few rounds before alerting.
- `above_baseline` with a low `index_bps` but a high `max_bps` → check the outlier exclusions
  (`get_outlier_exclusions`) and the per-source contribution; you are looking at one source.
- `above_baseline` with a high `index_bps` *and* a high `max_bps` → the round is genuinely split.
  Check whether a market event explains it before treating it as source misbehaviour.
- Correlate with `get_latency_report`: a rising index alongside a rising `never_counted` for one
  source usually means that source is not participating, not that the others are wrong.
