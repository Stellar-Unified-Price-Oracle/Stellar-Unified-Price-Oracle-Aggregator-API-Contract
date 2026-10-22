# Per-source accuracy scorecards (#490)

Reputation is a single coarse number. A scorecard answers a different question:
**how accurate was this source, recently and over the long run?**

## The reference is not circular

Scoring a source against the aggregate it helped form guarantees a good score.
Every sample here is therefore measured against a **leave-one-out reference** —
the median of the *other* sources in the same round:

```
reference(source S) = median({ price(t) : t != S })
error_bps          = (price(S) - reference) / reference * 10_000
```

A source is never compared against a value it contributed to, so accuracy is
measured rather than self-asserted. With fewer than `MIN_REFERENCE_SOURCES` (2)
peers there is no reference and **no sample is recorded** — a source is never
scored against itself.

## Two windows

| Window | Default | Answers |
|---|---|---|
| `short` | 10 samples | Is the source currently accurate? |
| `long` | 50 samples | Is the source *sustainedly* accurate? |

Both are the most recent `N` samples, so they update as submissions arrive and
old samples age out. The long window is bounded by `MAX_LONG_WINDOW` (512), so it
rolls rather than growing without bound.

## Metrics

| Field | Meaning |
|---|---|
| `mean_abs_error_bps` | Mean absolute relative error, winsorized |
| `bias_bps` | Mean **signed** error — non-zero means persistent directional bias |
| `hit_rate_bps` | Share of samples within `hit_tolerance_bps` |
| `long_max_error_bps` | Worst retained sample, so an outlier is visible even though the mean is bounded |

## One outlier cannot sink a long window

Each sample is **winsorized** at `outlier_cap_bps` (default 5000 = 50 %) before
it enters the mean, and the derived score has an explicit floor
(`SCORE_FLOOR_BPS` = 2000 = 20 %). A single catastrophic print therefore
contributes at most the cap to the average, and no single event can collapse a
long window below the floor.

Capping the *average* never hides the outlier: `long_max_error_bps` still
reports the worst retained sample, and because the cap is applied to the sample
itself, a consumer can see that a cap was hit rather than being shown a value the
window never used.

## Cold start

Below `cold_start_samples` (default 5) the source is reported as `cold_start` and
carries **no score judgement**. It is counted, not graded, so a newly onboarded
source is never penalised for having no history. `get_source_accuracy_score`
returns `0` while cold-start — meaning "not yet measured", not "measured as
terrible".

## Reporting only

Scorecards are **reported, not enforced**. They feed reputation as an input an
operator can weigh; they never change source admission, quorum or weighting on
their own. Collection is opt-in:

```rust
client.enable_scorecards();
```

With it off, the publication path pays nothing.

## Reconstructible from events

`SourceScorecardUpdatedEvent` carries every retained sample alongside the
computed window values, so an off-chain indexer can rebuild each window from the
event stream alone:

```rust
let samples = client.get_source_scorecard_samples(&source);  // oldest first
// mean_abs_error_bps is the mean of |decode(sample)| over the window
```

## Errors

| Code | Name | Meaning |
|---|---|---|
| 189 | `InvalidScorecardConfig` | A zero window, `long_window < short_window`, `cold_start_samples > long_window`, or a zero tolerance/cap |
